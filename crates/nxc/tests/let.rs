use nxc::{MAX_DEPTH, MAX_TOKENS, emit, nix, parse_nxc, syntax};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let actual = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    let expected = nix::import(native).unwrap_or_else(|e| panic!("{native}: {e:?}"));
    assert_eq!(actual, expected, "{source}");
    let generated = emit::nxc(&expected).unwrap();
    assert_eq!(parse_nxc(&generated).unwrap(), expected, "{generated}");
    assert_eq!(nix::import(&nix::emit(&actual).unwrap()).unwrap(), expected);
}

#[test]
fn let_bindings_and_final_yield_roundtrip() {
    use nxc::ir::{Binding, Expr};
    let expected = Expr::Let {
        bindings: vec![Binding::Assign {
            path: vec!["a".into()],
            value: Expr::Integer(1),
        }],
        body: Box::new(Expr::Variable("a".into())),
    };
    assert_eq!(parse_nxc("let { a = 1; yield a; }").unwrap(), expected);
    assert_eq!(nix::import("let a = 1; in a").unwrap(), expected);
    for (source, native) in [
        ("let { yield 1; }", "let in 1"),
        (
            "let { a = b + 1; b = 2; yield a; }",
            "let a = b + 1; b = 2; in a",
        ),
        (
            "let { a.b = 1; a.c = 2; yield a; }",
            "let a.b = 1; a.c = 2; in a",
        ),
        (
            "let { a = rec { b = c; }; a.c = 2; yield a.b; }",
            "let a = rec { b = c; }; a.c = 2; in a.b",
        ),
        (
            "let { inherit x y; inherit (src) z; yield [x, y, z]; }",
            "let inherit x y; inherit (src) z; in [x y z]",
        ),
        (
            "let { inherit; inherit (1 / 0); yield 1; }",
            "let inherit; inherit (1 / 0); in 1",
        ),
        (
            "let { a.yield = 1; a.fn = 2; yield a.yield; }",
            "let a.yield = 1; a.fn = 2; in a.yield",
        ),
        (
            "let { yield let { a = 1; yield a; }; }",
            "let in let a = 1; in a",
        ),
        (
            "let { f = x => x + 1; yield f(2); }",
            "let f = x: x + 1; in f 2",
        ),
        (
            "/* α */ let { // β\na = 2; yield /* γ */ a; /* δ */ }",
            "let a = 2; in a",
        ),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn emitters_validate_the_let_body_and_variable_bindings() {
    use nxc::ir::{Binding, Expr};
    for expr in [
        Expr::Let {
            bindings: vec![],
            body: Box::new(Expr::Integer(u64::MAX)),
        },
        Expr::Let {
            bindings: vec![],
            body: Box::new(Expr::Variable("yield".into())),
        },
        Expr::Let {
            bindings: vec![Binding::Assign {
                path: vec!["fn".into()],
                value: Expr::Integer(1),
            }],
            body: Box::new(Expr::Integer(2)),
        },
        Expr::Let {
            bindings: vec![Binding::Inherit {
                source: Some(Expr::Variable("src".into())),
                names: vec!["yield".into()],
            }],
            body: Box::new(Expr::Integer(2)),
        },
    ] {
        assert!(emit::nxc(&expr).is_err(), "accepted {expr:?}");
        assert!(nix::emit(&expr).is_err(), "accepted {expr:?}");
    }
}

#[test]
fn delimited_let_expressions_compose_with_existing_syntax() {
    for (source, native) in [
        ("let { yield x => x; }(2)", "(let in x: x) 2"),
        (
            "let { yield { a = 1; }; }.a + 2",
            "(let in { a = 1; }).a + 2",
        ),
        ("2 * let { yield 3 + 4; }", "2 * (let in 3 + 4)"),
        ("s.a or let { yield 2; }", "s.a or (let in 2)"),
        (
            "[let { yield 1; } let { yield 2; }]",
            "[(let in 1) (let in 2)]",
        ),
        ("f(let { yield 1; }, 2)", "f (let in 1) 2"),
        ("x => let { a = x; yield a; }", "x: let a = x; in a"),
        ("({ x ? let { yield 1; } }) => x", "{ x ? let in 1 }: x"),
        (
            "let { yield \"${let { yield ''nested''; }}\"; }",
            "let in \"${let in ''nested''}\"",
        ),
        ("{ yield = 1; }.yield", "{ yield = 1; }.yield"),
    ] {
        roundtrip(source, native);
    }
    for native in [
        "[let in 1]",
        "{}.a or let in 1",
        "1 + let in 2",
        "f let in 1",
    ] {
        assert!(
            nix::import(native).is_err(),
            "accepted invalid Nix: {native}"
        );
    }
}

#[test]
fn malformed_lets_are_lossless_and_recovery_keeps_the_result() {
    for source in [
        "let {}",
        "let { a = 1; }",
        "let { yield; }",
        "let { yield 1 }",
        "let { yield 1; yield 2; }",
        "let { yield 1; a = 2; }",
        "let { a = 1 yield a; }",
        "let { a = ; yield a; }",
        "let { yield 1;; }",
        "let { a = 1; yield a;",
        "yield 1;",
        "let a = 1; in a",
        "let { body = 1; }",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
    }
    for source in [
        "let { a = @; b = 2; yield h(3); }",
        "let { a = @ yield h(3); }",
        "let { a = @.yield yield h(3); }",
        "let { a = @ + { x = 1; }; yield h(3); }",
        "let { a = f(@, let { yield [1, 2]; }); yield h(3); }",
        "f(let { yield @; }, h(3))",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(
            root.descendants()
                .any(|n| n.kind() == syntax::SyntaxKind::CallExpr && n.text() == "h(3)"),
            "lost result or later argument: {root:#?}"
        );
    }
}

#[test]
fn binding_recovery_keeps_later_items_after_a_qualified_yield() {
    for source in [
        "let { a = @.yield; b = 2; yield h(3); }",
        "let { a = @ + s . /* comment */ yield; b = 2; yield h(3); }",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
        assert!(
            root.descendants()
                .any(|node| node.kind() == syntax::SyntaxKind::AssignBinding
                    && node.text() == "b = 2"),
            "lost later binding: {root:#?}"
        );
        assert!(
            root.descendants()
                .any(|node| node.kind() == syntax::SyntaxKind::CallExpr && node.text() == "h(3)"),
            "lost result: {root:#?}"
        );
    }
}

#[test]
fn let_bindings_reject_conflicts_and_reserved_variable_names() {
    // Establish positive support before checking rejection at its boundaries.
    roundtrip("let { x = 1; yield x; }", "let x = 1; in x");
    for bindings in [
        "a = 1; a = 2;",
        "a.b = 1; a = 2;",
        "a = { b = 1; }; a.b = 2;",
        "inherit a; a.b = 1;",
        "inherit (src) a a;",
        "fn = 1;",
        "yield = 1;",
        "or = 1;",
        "__curPos = 1;",
        "__nxc_private.x = 1;",
        "inherit (src) fn;",
        "inherit yield;",
        "\"quoted\" = 1;",
        "${name} = 1;",
    ] {
        let source = format!("let {{ {bindings} yield 1; }}");
        let parsed = syntax::parse(&source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
        let native = format!("let {bindings} in 1");
        assert!(nix::import(&native).is_err(), "accepted {native}");
    }
    assert!(nix::import("let { body = 1; }").is_err());
}

#[test]
fn let_nesting_and_binding_counts_obey_resource_limits() {
    let source = (0..MAX_DEPTH - 1).fold("1".to_owned(), |inner, _| {
        format!("let {{ yield {inner}; }}")
    });
    let native = format!("{}1", "let in ".repeat(MAX_DEPTH - 1));
    roundtrip(&source, &native);
    assert!(parse_nxc(&format!("let {{ yield {source}; }}")).is_err());
    assert!(nix::import(&format!("let in {native}")).is_err());
    let source = (0..MAX_DEPTH - 1).fold("1".to_owned(), |inner, _| {
        format!("let {{ a = {inner}; yield a; }}")
    });
    let native =
        (0..MAX_DEPTH - 1).fold("1".to_owned(), |inner, _| format!("let a = {inner}; in a"));
    roundtrip(&source, &native);
    assert!(parse_nxc(&format!("let {{ a = {source}; yield a; }}")).is_err());
    assert!(nix::import(&format!("let a = {native}; in a")).is_err());
    let bindings = "inherit; ".repeat((MAX_TOKENS - 6) / 2);
    let source = format!("let {{ {bindings} yield 1; }}");
    assert!(parse_nxc(&source).is_ok());
    assert!(parse_nxc(&source.replace("yield", "inherit; yield")).is_err());
}

#[test]
fn native_nix_confirms_recursive_scope_inheritance_and_laziness() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(output) => assert!(output.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    for (source, expected) in [
        ("let in 1", Some("1")),
        ("let a = b + 1; b = 2; in a", Some("3")),
        ("let a = 1 / 0; in 2", Some("2")),
        ("let x = 1; in let x = 2; y = x; in y", Some("2")),
        ("let x = 7; in let inherit x; in x", Some("7")),
        ("let src = { x = 5; }; inherit (src) x; in x", Some("5")),
        ("let inherit (1 / 0); in 3", Some("3")),
        ("let a = rec { b = c; }; a.c = 2; in a.b", Some("2")),
        ("let a.b = 1; a.c = 2; in a.b + a.c", Some("3")),
        ("let f = x: g x; g = x: x + 1; in f 2", Some("3")),
        ("let x = 4; f = { y ? x }: y; in f {}", Some("4")),
        ("let a = 1 / 0; in a", None),
        ("let a = b; b = a; in a", None),
        ("let inherit absent; in 1", None),
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
