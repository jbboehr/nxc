mod support;
use support::nxc;

use nxc::{
    MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, emit,
    ir::{AttrName, Binding, Expr, StringPart},
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
fn dynamic_bindings_round_trip_without_reordering_or_expanding_paths() {
    for (source, native) in [
        ("{ ${name} = value; }", "{ ${name} = value; }"),
        (
            r#"{ "prefix-${name}" = value; }"#,
            r#"{ "prefix-${name}" = value; }"#,
        ),
        (
            r#"{ a.${name}."b.c"."${suffix}" = value; }"#,
            r#"{ a.${name}."b.c"."${suffix}" = value; }"#,
        ),
        (
            r#"{ ${""} = 1; ${"a"}.b = 2; }"#,
            r#"{ ${""} = 1; ${"a"}.b = 2; }"#,
        ),
        (
            r#"{ ${if flag then "a" else "b"} = 1; }"#,
            r#"{ ${if flag then "a" else "b"} = 1; }"#,
        ),
        ("{ ${f(name)} = value; }", "{ ${f name} = value; }"),
        (
            "{ ${with(ctx, name)} = value; }",
            "{ ${with ctx; name} = value; }",
        ),
        (
            r#"{ ${let { x = "a"; yield x; }} = 1; }"#,
            r#"{ ${let x = "a"; in x} = 1; }"#,
        ),
        (
            "{ ${ { ${key} = name; }.${key} } = 1; }",
            "{ ${ { ${key} = name; }.${key} } = 1; }",
        ),
        (
            "rec { name = outer; ${name} = name; }",
            "rec { name = outer; ${name} = name; }",
        ),
        ("let { a.${key} = 1; yield a; }", "let a.${key} = 1; in a"),
        (r#"let { ${"x"} = 1; yield x; }"#, r#"let ${"x"} = 1; in x"#),
        ("let { ${''x''} = 1; yield x; }", "let ${''x''} = 1; in x"),
        (
            r#"{ inherit (s) ${("x")}; }"#,
            r#"{ inherit (s) ${("x")}; }"#,
        ),
        (
            r#"let { x = 1; yield { inherit ${"x"}; }; }"#,
            r#"let x = 1; in { inherit ${"x"}; }"#,
        ),
        (
            "{ a = rec { key = y; }; a.${key} = 1; }",
            "{ a = rec { key = y; }; a.${key} = 1; }",
        ),
        (
            "{ a.${key} = 1; a = rec { key = y; }; }",
            "{ a.${key} = 1; a = rec { key = y; }; }",
        ),
        (
            "{ ${ /* } ; ${ */ name # }\n } = 1; }",
            "{ ${ /* } ; ${ */ name # }\n } = 1; }",
        ),
    ] {
        roundtrip(source, native);
    }
    roundtrip(r#"{ "${key}" = 1; }"#, r#"{ ${"${key}"} = 1; }"#);
    assert_ne!(
        nix::import(r#"{ ${key} = 1; }"#).unwrap(),
        nix::import(r#"{ "${key}" = 1; }"#).unwrap()
    );
}

#[test]
fn only_statically_known_keys_participate_in_binding_conflict_checks() {
    for source in [
        r#"{ ${"x"} = 1; x = 2; }"#,
        r#"{ x = 1; ${( "x" )} = 2; }"#,
        r#"{ ${"a"}.b = 1; a.${"b"} = 2; }"#,
        r#"{ a = 1; a.${key} = 2; }"#,
        r#"{ a.${key} = 2; a = 1; }"#,
        r#"{ ${"a"} = { b = 1; }; a.b = 2; }"#,
        r#"{ inherit (s) ${"x"}; x = 2; }"#,
    ] {
        assert!(parse_nxc(source).is_err(), "accepted nxc {source}");
        assert!(nix::import(source).is_err(), "accepted native {source}");
    }
    for source in [
        r#"{ ${key}.a = 1; ${key}.a = 2; }"#,
        r#"{ "${"a"}".b = 1; a.b = 2; }"#,
        r#"{ a.${key}.b = 1; a.${key}.b = 2; }"#,
        r#"{ ${null} = 1; ${null} = 2; }"#,
        r#"{ ${"a"}.b = 1; a.c = 2; }"#,
        r#"{ ${key} = { a = 1; }; ${other} = { a = 2; }; }"#,
    ] {
        roundtrip(source, source);
    }
    let invalid = "{ ${key} = { a = 1; a = 2; }; }";
    assert!(parse_nxc(invalid).is_err());
    assert!(nix::import(invalid).is_err());
}

#[test]
fn invalid_dynamic_bindings_remain_lossless_and_recover_later_bindings() {
    for source in [
        "{ ${} = 1; }",
        "{ ${name} = ; }",
        "{ a.${name}. = 1; }",
        r#"{ "${}" = 1; }"#,
        "{ ''${key}'' = 1; }",
        "{ ${__curPos} = 1; }",
        "{ ${./path${__curPos}} = 1; }",
        "{ ${null} = __curPos; }",
        "{ ${null}.${__curPos} = 1; }",
        "{ inherit ${key}; }",
        r#"{ inherit (s) "${key}"; }"#,
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
        assert!(nix::import(source).is_err(), "accepted native {source}");
    }
    for name in [
        r#"${key}"#,
        r#""${key}""#,
        r#"${if true then "x" else "y"}"#,
        r#"${"or"}"#,
        r#"${"__curPos"}"#,
        r#"${"__nxc_update"}"#,
    ] {
        assert!(parse_nxc(&format!("let {{ {name} = 1; yield 2; }}")).is_err());
        assert!(nix::import(&format!("let {name} = 1; in 2")).is_err());
    }
    for source in [
        "{ ${} = 1; good = 3; }",
        "{ ${key} = ; good = 3; }",
        r#"{ "${}" = 1; good = 3; }"#,
        "let { a.${} = 1; good = 3; yield good; }",
        "f({ ${} = 1; }, { good = 3; })",
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
fn dynamic_binding_keys_and_values_obey_implicit_path_depth() {
    let path = vec!["${key}"; MAX_DEPTH - 1].join(".");
    let source = format!("{{ {path} = 1; }}");
    roundtrip(&source, &source);
    let over = format!("{{ {path}.${{key}} = 1; }}");
    assert!(parse_nxc(&over).is_err());
    assert!(nix::import(&over).is_err());
    let prefix = vec!["a"; MAX_DEPTH - 2].join(".");
    let source = format!("{{ {prefix}.${{ {{}} }} = 1; }}");
    roundtrip(&source, &source);
    let over = format!("{{ {prefix}.${{ {{ a = 1; }} }} = 1; }}");
    assert!(parse_nxc(&over).is_err());
    assert!(nix::import(&over).is_err());
    // A key containing an attrset consumes two lexical delimiters per level.
    let source =
        (0..MAX_DEPTH / 2).fold("key".to_owned(), |key, _| format!("{{ ${{{key}}} = 1; }}"));
    roundtrip(&source, &source);
}

#[test]
fn dynamic_binding_keys_share_byte_and_node_budgets() {
    for key in [
        Expr::Variable("__curPos".into()),
        Expr::Integer(u64::MAX),
        Expr::String(vec![StringPart::Literal("\0".into())]),
        Expr::String(vec![StringPart::Literal("x".repeat(MAX_SOURCE_BYTES + 1))]),
        Expr::List(vec![Expr::Integer(1); MAX_TOKENS]),
    ] {
        let expr = Expr::AttrSet {
            recursive: false,
            bindings: vec![Binding::Assign {
                path: vec![
                    AttrName::Dynamic(Box::new(Expr::Variable("null".into()))),
                    AttrName::Dynamic(Box::new(key)),
                ],
                value: Expr::Integer(1),
            }],
        };
        assert!(emit::nxc(&expr).is_err());
        assert!(nix::emit(&expr).is_err());
    }
    // Six tokens per dynamic binding, plus braces and the empty inherit.
    let source = format!(
        "{{ {}inherit; }}",
        "${key} = 1; ".repeat((MAX_TOKENS - 2) / 6)
    );
    assert_eq!(
        syntax::lexer::lex(&source)
            .iter()
            .filter(|t| !t.kind.is_trivia())
            .count(),
        MAX_TOKENS
    );
    roundtrip(&source, &source);
    let over = source.replacen("inherit;", "inherit; inherit;", 1);
    let parsed = syntax::parse(&over);
    assert_eq!(parsed.syntax().unwrap().to_string(), over);
    assert_eq!(parsed.diagnostics().len(), 1);
    assert!(parsed.lower().is_err());
    assert!(nix::import(&over).is_err());
}

#[test]
fn native_nix_confirms_dynamic_binding_scope_omission_and_collisions() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    for (source, expected, error) in [
        (r#"let key = "x"; in { ${key} = 1; }.x"#, Some("1"), ""),
        (r#"{ "prefix-${"x"}" = 2; }."prefix-x""#, Some("2"), ""),
        (r#"{ ${null} = abort "unused"; }"#, Some("{}"), ""),
        (
            r#"{ a.${null}.${abort "unused"} = abort "unused"; }"#,
            Some(r#"{"a":{}}"#),
            "",
        ),
        (r#"let null = "x"; in { ${null} = 3; }.x"#, Some("3"), ""),
        (r#"rec { ${"x"} = 4; y = x; }.y"#, Some("4"), ""),
        (
            r#"let x = 5; in rec { "${"x"}" = 6; y = x; }.y"#,
            Some("5"),
            "",
        ),
        (
            r#"let name = "outer"; in rec { name = "inner"; ${name} = name; }.inner"#,
            Some(r#""inner""#),
            "",
        ),
        (
            r#"let key = "x"; in { a = rec { key = "y"; }; a.${key} = 1; }.a.y"#,
            Some("1"),
            "",
        ),
        (
            r#"let key = "x"; in { a.${key} = 1; a = rec { key = "y"; }; }.a.x"#,
            Some("1"),
            "",
        ),
        (r#"let ${"x"} = 7; in x"#, Some("7"), ""),
        (r#"let key = "x"; a.${key} = 8; in a.x"#, Some("8"), ""),
        (r#"let x = 9; in rec { inherit ${"x"}; }.x"#, Some("9"), ""),
        (r#"{ inherit ({ x = 10; }) ${"x"}; }.x"#, Some("10"), ""),
        (
            r#"{ ${"a"}.b = 1; a.c = 2; }.a"#,
            Some(r#"{"b":1,"c":2}"#),
            "",
        ),
        (
            r#"{ "${{ __toString = _: "x"; }}" = 11; }.x"#,
            Some("11"),
            "",
        ),
        (
            r#"{ ${ { __toString = _: "x"; } } = 1; }"#,
            None,
            "expected a string",
        ),
        (r#"{ "${null}" = 1; }"#, None, "cannot coerce null"),
        (
            r#"let key = "x"; in { ${key}.a = 1; ${key}.b = 2; }"#,
            None,
            "already defined",
        ),
        (r#"{ "${"x"}" = 1; x = 2; }"#, None, "already defined"),
        (
            r#"{ x = 1; ${abort "key-forced"} = 2; }.x"#,
            None,
            "key-forced",
        ),
        (r#"{ ${./not-read} = 1; }"#, None, "expected a string"),
        (
            r#"{ ${builtins.appendContext "x" { "/nix/store/00000000000000000000000000000000-key" = { path = true; }; }} = 1; }"#,
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
                assert!(stderr.contains(error), "{native}: {stderr}");
            }
        }
    }
}
