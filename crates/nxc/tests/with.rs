use nxc::{MAX_DEPTH, MAX_TOKENS, emit, ir::Expr, nix, parse_nxc, syntax};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let actual = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    let expected = nix::import(native).unwrap_or_else(|e| panic!("{native}: {e:?}"));
    assert_eq!(actual, expected, "{source}");
    assert_eq!(parse_nxc(&emit::nxc(&expected).unwrap()).unwrap(), expected);
    assert_eq!(nix::import(&nix::emit(&actual).unwrap()).unwrap(), expected);
}

#[test]
fn with_keeps_its_context_and_body_as_separate_expressions() {
    let expected = Expr::With {
        scope: Box::new(Expr::Variable("scope".into())),
        body: Box::new(Expr::Variable("value".into())),
    };
    assert_eq!(parse_nxc("with(scope, value)").unwrap(), expected);
    assert_eq!(nix::import("with scope; value").unwrap(), expected);
    for (source, native) in [
        ("with(scope, value)", "with scope; value"),
        ("with(scope, value,)", "with scope; value"),
        ("with({ x = 1; }, x)", "with { x = 1; }; x"),
        ("with(f(1, 2), g(3, 4))", "with f 1 2; g 3 4"),
        ("with({}, [1, 2])", "with {}; [1 2]"),
        ("with({}, x => x)", "with {}; x: x"),
        ("with(x => x, 1)", "with x: x; 1"),
        (
            "with(scope, with(inner, body))",
            "with scope; with inner; body",
        ),
        (
            "with(with(scope, inner), body)",
            "with (with scope; inner); body",
        ),
        (
            "with(let { yield {}; }, let { yield 1; })",
            "with (let in {}); let in 1",
        ),
        (
            "/* α */ with /* β */ (scope, // γ\nvalue,)",
            "with scope; value",
        ),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn emitters_validate_both_with_children_even_when_the_context_is_unused() {
    for invalid in [
        Expr::Integer(u64::MAX),
        Expr::Variable("with".into()),
        (0..MAX_DEPTH - 1).fold(Expr::Integer(1), |inner, _| Expr::Negate(Box::new(inner))),
        Expr::List(vec![Expr::Integer(1); MAX_TOKENS]),
    ] {
        for expr in [
            Expr::With {
                scope: Box::new(invalid.clone()),
                body: Box::new(Expr::Integer(1)),
            },
            Expr::With {
                scope: Box::new(Expr::Integer(1)),
                body: Box::new(invalid.clone()),
            },
        ] {
            assert!(emit::nxc(&expr).is_err(), "accepted {expr:?}");
            assert!(nix::emit(&expr).is_err(), "accepted {expr:?}");
        }
    }
}

#[test]
fn with_composes_with_calls_selections_and_other_expression_positions() {
    for (source, native) in [
        ("with({}, x => x)(2)", "(with {}; x: x) 2"),
        ("with({}, { a = 1; }).a", "(with {}; { a = 1; }).a"),
        (
            "s.a or with(scope, fallback)",
            "s.a or (with scope; fallback)",
        ),
        ("2 * with({}, 3 + 4)", "2 * (with {}; 3 + 4)"),
        ("with({}, 1) + 2", "(with {}; 1) + 2"),
        ("[with({}, 1) with({}, 2)]", "[(with {}; 1) (with {}; 2)]"),
        ("f(with({}, 1), 2)", "f (with {}; 1) 2"),
        ("x => with(scope, x)", "x: with scope; x"),
        (
            "({ x ? with(scope, fallback) }) => x",
            "{ x ? with scope; fallback }: x",
        ),
        (
            "let { inherit (with(scope, source)) x; yield x; }",
            "let inherit (with scope; source) x; in x",
        ),
        (
            r#""${with({ x = "value"; }, x)}""#,
            r#""${with { x = "value"; }; x}""#,
        ),
        (
            "with({ x = ''value''; }, ''${x}'')",
            "with { x = ''value''; }; ''${x}''",
        ),
    ] {
        roundtrip(source, native);
    }
    for source in [
        "[with {}; 1]",
        "{}.a or with {}; 1",
        "1 + with {}; 2",
        "f with {}; 1",
    ] {
        assert!(
            nix::import(source).is_err(),
            "accepted invalid Nix: {source}"
        );
    }
}

#[test]
fn malformed_with_forms_are_lossless_and_preserve_later_expressions() {
    roundtrip("with({}, 1)", "with {}; 1");
    for source in [
        "with",
        "with()",
        "with(scope)",
        "with(scope,)",
        "with(scope)(value)",
        "with(scope, value, extra)",
        "with(, value)",
        "with(scope,, value)",
        "with(scope, value,,)",
        "with(scope; value)",
        "with scope; value",
        "with(scope, value",
        "with(scope, @)",
        "f(with)",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
    }
    for source in [
        "with(@, h(3))",
        "with(f(@, { x = 1; }), h(3))",
        "with(@ + ''commas, ${f(1, 2)}'', h(3))",
        "f(with(scope), h(3))",
        "f(with(scope, @), h(3))",
        "f(with(scope, value, extra), h(3))",
        "[with(scope, @), h(3)]",
        "let { a = with(@, x); yield h(3); }",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(
            root.descendants()
                .any(|n| n.kind() == syntax::SyntaxKind::CallExpr && n.text() == "h(3)"),
            "lost later expression: {root:#?}"
        );
    }
}

#[test]
fn with_contexts_and_bodies_obey_depth_and_token_limits() {
    let source = (0..MAX_DEPTH - 1).fold("1".to_owned(), |body, _| format!("with({{}}, {body})"));
    let native = format!("{}1", "with {}; ".repeat(MAX_DEPTH - 1));
    roundtrip(&source, &native);
    assert!(parse_nxc(&format!("with({{}}, {source})")).is_err());
    assert!(nix::import(&format!("with {{}}; {native}")).is_err());
    let source = (0..MAX_DEPTH - 1).fold("1".to_owned(), |scope, _| format!("with({scope}, 1)"));
    let native = (0..MAX_DEPTH - 1).fold("1".to_owned(), |scope, _| format!("with ({scope}); 1"));
    roundtrip(&source, &native);
    assert!(parse_nxc(&format!("with({source}, 1)")).is_err());
    assert!(nix::import(&format!("with ({native}); 1")).is_err());

    let count = (MAX_TOKENS - 2) / 7;
    let padding = "1 ".repeat((MAX_TOKENS - 2) % 7);
    let source = format!("[{}{padding}]", "with({}, 1) ".repeat(count));
    let native = format!("[{}{padding}]", "(with {}; 1) ".repeat(count));
    let ir = parse_nxc(&source).unwrap();
    assert_eq!(nix::import(&native).unwrap(), ir);
    // Explicit output commas can exceed the budget even for accepted input.
    assert!(emit::nxc(&ir).is_err());
    assert!(nix::emit(&ir).is_ok());
    for source in [
        format!("[{}{padding}1]", "with({}, 1) ".repeat(count)),
        format!("[{}{padding}1]", "(with {}; 1) ".repeat(count)),
    ] {
        assert!(parse_nxc(&source).is_err());
        assert!(nix::import(&source).is_err());
    }
}

#[test]
fn native_nix_confirms_scope_precedence_laziness_and_failures() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(output) => assert!(output.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    for (source, expected) in [
        ("with { x = 1; }; x", Some("1")),
        ("let x = 3; in with { x = 1; }; x", Some("3")),
        ("with { x = 1; }; let x = 2; in x", Some("2")),
        ("(x: with { x = 1; }; x) 3", Some("3")),
        (
            "with { x = 1; y = 4; }; with { x = 2; }; [x y]",
            Some("[2,4]"),
        ),
        ("with 1; 2", Some("2")),
        ("with (1 / 0); 2", Some("2")),
        ("let x = 3; in with (1 / 0); x", Some("3")),
        ("with { bad = 1 / 0; x = 4; }; x", Some("4")),
        ("with { x = 1; }; let inherit x; in x", Some("1")),
        ("with { x = 4; }; ({ y ? x }: y) {}", Some("4")),
        ("with { x = { a = 4; }; }; with x; a", Some("4")),
        ("with {}; missing", None),
        ("with 1; x", None),
        ("with { x = 1 / 0; }; x", None),
        ("with { a = 1; x = a; }; x", None),
    ] {
        let ir = nix::import(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let generated = nix::emit(&parse_nxc(&emit::nxc(&ir).unwrap()).unwrap()).unwrap();
        for value in [source, generated.as_str()] {
            let output = Command::new("nix-instantiate")
                .args([
                    "--store", "dummy://", "--eval", "--strict", "--json", "--expr", value,
                ])
                .output()
                .unwrap();
            if let Some(expected) = expected {
                assert!(
                    output.status.success(),
                    "{value}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(
                    String::from_utf8(output.stdout).unwrap().trim(),
                    expected,
                    "{value}"
                );
            } else {
                assert!(!output.status.success(), "{value} must fail");
            }
        }
    }
}
