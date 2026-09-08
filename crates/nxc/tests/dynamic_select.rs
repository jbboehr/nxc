use nxc::{
    MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, emit,
    ir::{AttrName, Expr, StringPart},
    nix, parse_nxc, syntax,
};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) -> String {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let ir = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    assert_eq!(nix::import(native).unwrap(), ir, "{native}");
    let converted = emit::nxc(&ir).unwrap();
    let reparsed = parse_nxc(&converted).unwrap();
    assert_eq!(reparsed, ir, "{converted}");
    let output = nix::emit(&reparsed).unwrap();
    assert_eq!(nix::import(&output).unwrap(), ir, "{output}");
    assert_eq!(emit::nxc(&reparsed).unwrap(), converted);
    output
}

#[test]
fn dynamic_selections_round_trip_in_both_directions() {
    for (source, native) in [
        ("s.${key}", "s.${key}"),
        (r#"s."prefix-${key}""#, r#"s."prefix-${key}""#),
        (r#"s.${""}"#, r#"s.${""}"#),
        (
            r#"s.a.${key}."b.c"."${suffix}" or fallback"#,
            r#"s.a.${key}."b.c"."${suffix}" or fallback"#,
        ),
        (
            r#"s.${if flag then "a" else "b"}"#,
            r#"s.${if flag then "a" else "b"}"#,
        ),
        (
            r#"s.${let { key = "a"; yield key; }}"#,
            r#"s.${let key = "a"; in key}"#,
        ),
        ("s.${f(x)}", "s.${f x}"),
        ("s.${x => x}", "s.${x: x}"),
        ("s.${with(ctx, key)}", "s.${with ctx; key}"),
        ("s.${assert(ok, key)}", "s.${assert ok; key}"),
        ("s.${s.${key}}", "s.${s.${key}}"),
        (r#"s.${{ a = "x"; }.a}"#, r#"s.${{ a = "x"; }.a}"#),
        (r#"s.${''x''}"#, r#"s.${''x''}"#),
        (
            r#""outer-${s."${key}"}-end""#,
            r#""outer-${s."${key}"}-end""#,
        ),
        ("s.${ /* } ${ */ key # }\n }", "s.${ /* } ${ */ key # }\n }"),
        ("s.${key}(1).${result}", "(s.${key} 1).${result}"),
        ("s.${key} or f(1)", "(s.${key} or f) 1"),
        ("s.${key} or (f(1))", "s.${key} or (f 1)"),
        ("s.${key} or t.${other} or 2", "s.${key} or t.${other} or 2"),
        ("[s.${a}, s.${b}]", "[s.${a} s.${b}]"),
        ("fn({ x ? s.${key} }) => x", "{ x ? s.${key} }: x"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn quoted_interpolation_retains_string_coercion() {
    let direct = nix::import("s.${key}").unwrap();
    let quoted = nix::import(r#"s."${key}""#).unwrap();
    assert_ne!(direct, quoted);
    assert_eq!(quoted, nix::import(r#"s.${"${key}"}"#).unwrap());
    roundtrip(r#"s."${key}""#, r#"s.${"${key}"}"#);
    // Literal quoting remains canonical with bare names; explicit expressions
    // are retained without constant folding.
    assert_eq!(parse_nxc(r#"s."a""#).unwrap(), parse_nxc("s.a").unwrap());
    assert_ne!(parse_nxc(r#"s.${"a"}"#).unwrap(), parse_nxc("s.a").unwrap());
}

#[test]
fn malformed_dynamic_keys_are_lossless_and_later_items_recover() {
    for source in [
        "s.${}",
        "s.${key",
        "s.${key}}",
        "s.$key",
        "s.${key}.",
        "s.${key} or",
        "s.${key} or -1",
        "s.${key} or x => x",
        "s.${__curPos}",
        "s.${/absolute/path}",
        "s.${x ?}",
        r#"s."${}""#,
        "s.''${key}''",
        "${key}",
        "{ ${} = 1; }",
        r#"{ "${}" = 1; }"#,
        "{ a.${} = 1; }",
        "{ inherit (s) ${key}; }",
        r#"{ inherit (s) "${key}"; }"#,
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
    }
    for native in [
        "s.${}",
        "s.${key",
        "s.${key} or x: x",
        "s.${__curPos}",
        "s.${/absolute/path}",
        "s.${x ?}",
        "s.''${key}''",
        "{ ${} = 1; }",
        r#"{ "${}" = 1; }"#,
        "{ a.${} = 1; }",
        "{ inherit (s) ${key}; }",
    ] {
        assert!(nix::import(native).is_err(), "accepted native {native}");
    }
    for source in [
        "{ bad = s.${}; good = 3; }",
        "let { bad = s.${}; good = 3; yield good; }",
        "f(s.${}, { good = 3; })",
        "[s.${}, { good = 3; }]",
        r#"f(s."${}", { good = 3; })"#,
        "f(s.${key} or , { good = 3; })",
    ] {
        let parsed = syntax::parse(source);
        assert!(parsed.lower().is_err());
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(
            root.descendants()
                .any(|node| node.kind() == syntax::SyntaxKind::AssignBinding
                    && node.text() == "good = 3"),
            "lost recovery: {source}\n{root:#?}"
        );
    }
}

#[test]
fn dynamic_key_nesting_and_path_lengths_are_bounded() {
    let path = vec!["${key}"; MAX_DEPTH].join(".");
    let source = format!("s.{path}");
    roundtrip(&source, &source);
    for source in [
        format!("{source}.${{key}}"),
        (0..MAX_DEPTH).fold("key".to_owned(), |key, _| format!("s.${{{key}}}")),
    ] {
        let parsed = syntax::parse(&source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(nix::import(&source).is_err());
    }
    // This fits the semantic and delimiter bounds even though CST key wrappers
    // add physical tree depth. Generated parentheses can reach the output limit.
    let source = (0..MAX_DEPTH - 1).fold("key".to_owned(), |key, _| format!("s.${{{key}}}"));
    assert_eq!(parse_nxc(&source).unwrap(), nix::import(&source).unwrap());
    let source = (0..(MAX_DEPTH - 1) / 2).fold("key".to_owned(), |key, _| format!("s.${{{key}}}"));
    roundtrip(&source, &source);
}

#[test]
fn dynamic_keys_share_source_token_and_ir_budgets() {
    let source = format!("[{} 1 1]", "s.${key} ".repeat((MAX_TOKENS - 2) / 5));
    assert_eq!(
        syntax::lexer::lex(&source)
            .iter()
            .filter(|t| !t.kind.is_trivia())
            .count(),
        MAX_TOKENS
    );
    assert_eq!(parse_nxc(&source).unwrap(), nix::import(&source).unwrap());
    let over = source.replacen(']', " 1]", 1);
    let parsed = syntax::parse(&over);
    assert_eq!(parsed.syntax().unwrap().to_string(), over);
    assert_eq!(parsed.diagnostics().len(), 1);
    assert!(parsed.lower().is_err());
    assert!(nix::import(&over).is_err());

    for key in [
        Expr::Variable("__curPos".into()),
        Expr::Integer(u64::MAX),
        Expr::String(vec![StringPart::Literal("\0".into())]),
        Expr::String(vec![StringPart::Literal("x".repeat(MAX_SOURCE_BYTES + 1))]),
        Expr::List(vec![Expr::Integer(1); MAX_TOKENS]),
        (0..MAX_DEPTH - 1).fold(Expr::Integer(1), |e, _| Expr::Not(Box::new(e))),
    ] {
        let ir = Expr::Select {
            value: Box::new(Expr::Variable("s".into())),
            path: vec!["missing".into(), AttrName::Dynamic(Box::new(key))],
            default: Some(Box::new(Expr::Integer(1))),
        };
        // Even a key which might never be forced must pass conversion bounds.
        assert!(emit::nxc(&ir).is_err());
        assert!(nix::emit(&ir).is_err());
    }
}

#[test]
fn native_nix_confirms_dynamic_keys_coercion_scope_and_lazy_defaults() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    for (source, expected, error_fragment) in [
        (r#"let key = "x"; in { x = 1; }.${key}"#, Some("1"), ""),
        (r#"{ "prefix-x" = 2; }."prefix-${"x"}""#, Some("2"), ""),
        (r#"{ "" = 3; }.${""}"#, Some("3"), ""),
        (r#"{ a."b.c".d = 4; }.a.${"b.c"}."${"d"}""#, Some("4"), ""),
        (r#"{ x = 5; }.${"x"} or (abort "unused")"#, Some("5"), ""),
        (r#"{}.missing.${abort "unused"} or 6"#, Some("6"), ""),
        (r#"{}.${"missing"}.${abort "unused"} or 7"#, Some("7"), ""),
        (r#"{ x = 1; }.${"x"}.y or 8"#, Some("8"), ""),
        (r#"with { key = "x"; }; { x = 9; }.${key}"#, Some("9"), ""),
        (
            r#"let key = "x"; in with { key = "y"; }; { x = 10; }.${key}"#,
            Some("10"),
            "",
        ),
        (
            r#"({ key ? "x", s }: s.${key}) { s.x = 11; }"#,
            Some("11"),
            "",
        ),
        (
            r#"{ x = 12; }."${{ __toString = _: "x"; }}""#,
            Some("12"),
            "",
        ),
        (
            r#"{ x = 13; }.${{ __toString = _: "x"; }} or 0"#,
            None,
            "expected a string",
        ),
        (r#"{}.${null} or 0"#, None, "expected a string"),
        (r#"{}.${./not-read} or 0"#, None, "expected a string"),
        (r#"{}.${abort "current-key"} or 0"#, None, "current-key"),
        (
            r#"(abort "base-first").${abort "key-second"} or 0"#,
            None,
            "base-first",
        ),
        (
            r#"{ x = abort "value-forced"; }.${"x"} or 0"#,
            None,
            "value-forced",
        ),
        (
            r#"{}.${builtins.appendContext "x" { "/nix/store/00000000000000000000000000000000-key" = { path = true; }; }} or 0"#,
            None,
            "not allowed to refer",
        ),
    ] {
        let ir = nix::import(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let converted = emit::nxc(&ir).unwrap();
        let output = roundtrip(&converted, source);
        for native in [source, &output] {
            let result = Command::new("nix-instantiate")
                .args([
                    "--store",
                    "dummy://",
                    "--eval",
                    "--strict",
                    "--json",
                    "--expr",
                    &format!("({native})"),
                ])
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&result.stderr);
            if let Some(expected) = expected {
                assert!(result.status.success(), "{native}: {stderr}");
                assert_eq!(
                    String::from_utf8(result.stdout).unwrap().trim(),
                    expected,
                    "{native}"
                );
            } else {
                assert!(!result.status.success(), "{native} must fail");
                assert!(stderr.contains(error_fragment), "{native}: {stderr}");
            }
        }
    }
}
