mod support;
use support::nxc;

use nxc::{MAX_DEPTH, emit, nix, parse_nxc, syntax};
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

fn nix_available() -> bool {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => {
            assert!(result.status.success());
            true
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            false
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
}

#[test]
fn attribute_existence_round_trips_with_static_and_dynamic_paths() {
    for (source, native) in [
        ("s ? a", "s ? a"),
        ("s?a.b", "s ? a.b"),
        (r#"s ? "a.b"."""#, r#"s ? "a.b"."""#),
        (
            r#"s ? a.${key}."prefix-${suffix}""#,
            r#"s ? a.${key}."prefix-${suffix}""#,
        ),
        ("s ? ${f(key)}", "s ? ${f key}"),
        (r#"s ? ${''a'''b''}"#, r#"s ? ${''a'''b''}"#),
        (
            "s ? or.fn.yield.__nxc_update",
            "s ? or.fn.yield.__nxc_update",
        ),
        (
            "s ? ${if flag then a else b}",
            "s ? ${if flag then a else b}",
        ),
        ("s ? ${with(ctx, key)}", "s ? ${with ctx; key}"),
        ("s ? ${let { yield key; }}", "s ? ${let in key}"),
        ("fn({ x ? s ? a }) => x", "{ x ? s ? a }: x"),
        ("[s ? a, t ? b]", "[(s ? a) (t ? b)]"),
        ("f(s ? a, t ? b)", "f (s ? a) (t ? b)"),
        (
            "let { found = s ? a; yield found; }",
            "let found = s ? a; in found",
        ),
        ("if s ? a then s.a else 0", "if s ? a then s.a else 0"),
        ("assert(s ? a, s.a)", "assert s ? a; s.a"),
        (r#""${s ? a}""#, r#""${s ? a}""#),
        ("{ ${s ? a} = 1; }", "{ ${s ? a} = 1; }"),
        ("s.${t ? a}", "s.${t ? a}"),
        ("s /* ? */ ? // key\n a", "s ? # key\n a"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn attribute_existence_precedence_matches_native_grouping() {
    for (source, native) in [
        ("f(x) ? a", "(f x) ? a"),
        ("s.a ? b", "(s.a) ? b"),
        ("s.a or t ? b", "(s.a or t) ? b"),
        ("s.a or (t ? b)", "s.a or (t ? b)"),
        ("-s ? a", "(-s) ? a"),
        ("!s ? a", "!(s ? a)"),
        ("!s ? a == b", "(!(s ? a)) == b"),
        ("s ? a ++ t", "(s ? a) ++ t"),
        ("s ++ t ? a", "s ++ (t ? a)"),
        ("s ? a * 2 + 1", "((s ? a) * 2) + 1"),
        ("s + t ? a", "s + (t ? a)"),
        ("s ? a < t ? b", "(s ? a) < (t ? b)"),
        ("s ? a == t ? b", "(s ? a) == (t ? b)"),
        ("s ? a && t ? b || u ? c", "((s ? a) && (t ? b)) || (u ? c)"),
        ("s ? a ? b", "(s ? a) ? b"),
        ("x => x ? a", "x: (x ? a)"),
        ("__nxc_update(s, t) ? a", "(s // t) ? a"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn malformed_existence_paths_are_lossless_and_recover_later_items() {
    for source in [
        "? a",
        "s ?",
        "s ? a.",
        "s ? 1",
        "s ? ${}",
        r#"s ? "${}""#,
        "s ? ''a''",
        "s ? a or t",
        "s ? ${./path${__curPos}}",
        "s ? missing.${__curPos}",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
        assert!(nix::import(source).is_err(), "accepted native {source}");
    }
    for source in [
        "f(s ? , good(3))",
        "[s ? ${}, good(3)]",
        "{ a = s ? ; b = good(3); }",
    ] {
        let parsed = syntax::parse(source);
        assert!(parsed.lower().is_err());
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(
            root.descendants()
                .any(|n| n.kind() == syntax::SyntaxKind::CallExpr && n.text() == "good(3)"),
            "lost recovery: {root:#?}"
        );
    }
}

#[test]
fn existence_value_and_dynamic_keys_obey_depth_and_path_limits() {
    let path = vec!["${key}"; MAX_DEPTH].join(".");
    let source = format!("s ? {path}");
    roundtrip(&source, &source);
    let chain = format!("s{}", " ? a".repeat(MAX_DEPTH - 1));
    roundtrip(&chain, &chain);
    let nested = (0..MAX_DEPTH - 1).fold("key".to_owned(), |key, _| format!("s ? ${{{key}}}"));
    assert_eq!(parse_nxc(&nested).unwrap(), nix::import(&nested).unwrap());
    let shallow =
        (0..(MAX_DEPTH - 1) / 2).fold("key".to_owned(), |key, _| format!("s ? ${{{key}}}"));
    roundtrip(&shallow, &shallow);
    for over in [
        format!("{source}.${{key}}"),
        format!("{chain} ? a"),
        format!("s ? ${{{nested}}}"),
    ] {
        let parsed = syntax::parse(&over);
        assert_eq!(parsed.syntax().unwrap().to_string(), over);
        assert!(parsed.lower().is_err(), "accepted {over}");
        assert!(nix::import(&over).is_err(), "accepted native {over}");
    }
}

#[test]
fn native_nix_confirms_existence_values_laziness_coercion_and_forcing_order() {
    if !nix_available() {
        return;
    }
    for (source, expected, error) in [
        ("{} ? missing", Some("false"), ""),
        ("{ a = 1; } ? a", Some("true"), ""),
        ("{ a.b = 1; } ? a.b", Some("true"), ""),
        ("{ a = 1; } ? a.b", Some("false"), ""),
        ("1 ? a", Some("false"), ""),
        ("{} ? a ? b", Some("false"), ""),
        ("{ a = abort \"unused\"; } ? a", Some("true"), ""),
        ("{} ? missing.${abort \"unused\"}", Some("false"), ""),
        ("{ a = null; } ? a", Some("true"), ""),
        (
            "let key = \"a\"; in { a.b = 1; } ? ${key}.b",
            Some("true"),
            "",
        ),
        (
            r#"{ "x" = 1; } ? "${{ __toString = _: "x"; }}""#,
            Some("true"),
            "",
        ),
        (
            "{ a = abort \"value-forced\"; } ? a.b",
            None,
            "value-forced",
        ),
        (
            "(abort \"value-first\") ? ${abort \"key-second\"}",
            None,
            "value-first",
        ),
        ("1 ? ${abort \"key-forced\"}", None, "key-forced"),
        ("{ a = 1; } ? a.${abort \"key-forced\"}", None, "key-forced"),
        ("{} ? ${null}", None, "expected a string"),
        (r#"{} ? "${null}""#, None, "cannot coerce null"),
        ("{} ? ${./not-read}", None, "expected a string"),
        (
            r#"{} ? ${builtins.appendContext "x" { "/nix/store/00000000000000000000000000000000-key" = { path = true; }; }}"#,
            None,
            "not allowed to refer",
        ),
        ("true || (abort \"unused\") ? a", Some("true"), ""),
    ] {
        let ir = nix::import(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let output = roundtrip(&emit::nxc(&ir).unwrap(), source);
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

#[test]
fn existence_children_and_paths_share_resource_budgets() {
    use nxc::{
        MAX_SOURCE_BYTES, MAX_TOKENS,
        ir::{AttrName, Expr, StringPart},
    };
    for invalid in [
        Expr::Variable("__curPos".into()),
        Expr::Integer(u64::MAX),
        Expr::String(vec![StringPart::Literal("\0".into())]),
        Expr::String(vec![StringPart::Literal("x".repeat(MAX_SOURCE_BYTES + 1))]),
        Expr::List(vec![Expr::Integer(1); MAX_TOKENS]),
        (0..MAX_DEPTH - 1).fold(Expr::Integer(1), |e, _| Expr::Not(Box::new(e))),
    ] {
        for expr in [
            Expr::HasAttr {
                value: Box::new(invalid.clone()),
                path: vec!["a".into()],
            },
            Expr::HasAttr {
                value: Box::new(Expr::Integer(1)),
                path: vec!["missing".into(), AttrName::Dynamic(Box::new(invalid))],
            },
        ] {
            assert!(emit::nxc(&expr).is_err());
            assert!(nix::emit(&expr).is_err());
        }
    }
    for path in [
        vec![],
        vec!["x".into(); MAX_DEPTH + 1],
        vec!["\0".into()],
        vec![AttrName::Static("x".repeat(MAX_SOURCE_BYTES / 2 + 1)); 2],
    ] {
        let expr = Expr::HasAttr {
            value: Box::new(Expr::Integer(1)),
            path,
        };
        assert!(emit::nxc(&expr).is_err());
        assert!(nix::emit(&expr).is_err());
    }
    let bindings = (0..(MAX_TOKENS - 2) / 7)
        .map(|i| format!("a{i} = {{}} ? a; "))
        .collect::<String>();
    let source = format!("{{ {bindings}inherit; }}");
    assert_eq!(
        syntax::lexer::lex(&source)
            .iter()
            .filter(|t| !t.kind.is_trivia())
            .count(),
        MAX_TOKENS
    );
    assert_eq!(parse_nxc(&source).unwrap(), nix::import(&source).unwrap());
    let over = source.replacen("inherit;", "inherit extra;", 1);
    let parsed = syntax::parse(&over);
    assert_eq!(parsed.syntax().unwrap().to_string(), over);
    assert_eq!(parsed.diagnostics().len(), 1);
    assert!(parsed.lower().is_err());
    assert!(nix::import(&over).is_err());
}

#[test]
fn native_existence_expression_boundaries_match_nix() {
    if !nix_available() {
        return;
    }
    for (source, accepted) in [
        ("[{} ? a]", false),
        ("[({} ? a)]", true),
        ("f {} ? a", true),
        ("f ({} ? a)", true),
        ("{} ? a f", false),
        ("{} ? a (x: x)", false),
        ("{} ? a: a", false),
        ("{} ? a ? b", true),
        ("{}.a or x ? b", true),
        ("{}.a or (x ? b)", true),
        ("{}.a or x: x ? b", false),
        ("!{} ? a", true),
        ("-{} ? a", true),
        ("{} ? a + x: x", false),
        ("{} ? ${x: x}", true),
        ("{} ? if", false),
        ("{} ? ''a''", false),
        ("{} ? a..b", false),
        ("{} ? or", true),
        ("{} ? a or 0", false),
        ("{} ? (${key})", false),
        ("(x: x) ? a", true),
        ("x: x ? a", true),
    ] {
        let native = format!("let f = x: x; x = {{}}; key = \"a\"; in ({source})");
        let result = Command::new("nix-instantiate")
            .args(["--store", "dummy://", "--parse", "--expr", &native])
            .output()
            .unwrap();
        assert_eq!(
            result.status.success(),
            accepted,
            "oracle {native}: {result:?}"
        );
        assert_eq!(nix::import(&native).is_ok(), accepted, "adapter {native}");
        if accepted {
            let ir = nix::import(&native).unwrap();
            roundtrip(&emit::nxc(&ir).unwrap(), &native);
        }
    }
}
