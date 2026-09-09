mod support;
use support::nxc;

use nxc::{MAX_DEPTH, MAX_TOKENS, emit, ir::Expr, nix, parse_nxc, syntax};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let actual = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    let expected = nix::import(native).unwrap_or_else(|e| panic!("{native}: {e:?}"));
    assert_eq!(actual, expected, "{source}");
    assert_eq!(parse_nxc(&emit::nxc(&actual).unwrap()).unwrap(), expected);
    assert_eq!(nix::import(&nix::emit(&actual).unwrap()).unwrap(), expected);
}

#[test]
fn assertions_preserve_the_condition_and_body_in_both_dialects() {
    let expected = Expr::Assert {
        condition: Box::new(Expr::Variable("condition".into())),
        body: Box::new(Expr::Variable("value".into())),
    };
    assert_eq!(parse_nxc("assert(condition, value)").unwrap(), expected);
    assert_eq!(nix::import("assert condition; value").unwrap(), expected);
    for (source, native) in [
        ("assert(condition, value)", "assert condition; value"),
        ("assert(condition, value,)", "assert condition; value"),
        ("assert(f(1, 2), g(3, 4))", "assert f 1 2; g 3 4"),
        ("assert(true, x => x)", "assert true; x: x"),
        ("assert(x => x, 1)", "assert x: x; 1"),
        ("assert(a, assert(b, value))", "assert a; assert b; value"),
        ("assert(assert(a, b), value)", "assert (assert a; b); value"),
        (
            "assert(let { yield true; }, with({}, 1))",
            "assert let in true; with {}; 1",
        ),
        (
            "assert(if c then a else b, if d then 1 else 2)",
            "assert if c then a else b; if d then 1 else 2",
        ),
        ("/* α */ assert /* β */ (true, // γ\n1,)", "assert true; 1"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn emitters_validate_both_assertion_children_even_when_the_body_cannot_run() {
    for invalid in [
        Expr::Integer(u64::MAX),
        Expr::Variable("assert".into()),
        (0..MAX_DEPTH - 1).fold(Expr::Integer(1), |inner, _| Expr::Negate(Box::new(inner))),
        Expr::List(vec![Expr::Integer(1); MAX_TOKENS]),
    ] {
        for expr in [
            Expr::Assert {
                condition: Box::new(invalid.clone()),
                body: Box::new(Expr::Integer(1)),
            },
            Expr::Assert {
                condition: Box::new(Expr::Variable("false".into())),
                body: Box::new(invalid.clone()),
            },
        ] {
            assert!(emit::nxc(&expr).is_err(), "accepted {expr:?}");
            assert!(nix::emit(&expr).is_err(), "accepted {expr:?}");
        }
    }
}

#[test]
fn assertions_compose_with_existing_expression_positions() {
    for (source, native) in [
        ("assert(true, x => x)(2)", "(assert true; x: x) 2"),
        ("assert(true, { a = 1; }).a", "(assert true; { a = 1; }).a"),
        ("s.a or assert(c, fallback)", "s.a or (assert c; fallback)"),
        ("2 * assert(c, 3 + 4)", "2 * (assert c; 3 + 4)"),
        ("assert(c, 1) + 2", "(assert c; 1) + 2"),
        ("-assert(c, 1)", "-(assert c; 1)"),
        (
            "[assert(a, 1) assert(b, 2)]",
            "[(assert a; 1) (assert b; 2)]",
        ),
        ("f(assert(c, 1), 2)", "f (assert c; 1) 2"),
        ("x => assert(c, x)", "x: assert c; x"),
        (
            "({ x ? assert(c, fallback) }) => x",
            "{ x ? assert c; fallback }: x",
        ),
        (
            "let { inherit (assert(c, source)) x; yield assert(c, x); }",
            "let inherit (assert c; source) x; in assert c; x",
        ),
        ("{ x = assert(c, 1); }", "{ x = assert c; 1; }"),
        (
            "with(assert(c, scope), assert(c, body))",
            "with (assert c; scope); assert c; body",
        ),
        (
            "if assert(c, a) then assert(c, b) else assert(c, d)",
            "if assert c; a then assert c; b else assert c; d",
        ),
        (r#""${assert(c, "value")}""#, r#""${assert c; "value"}""#),
        ("assert(c, ''${x}'')", "assert c; ''${x}''"),
    ] {
        roundtrip(source, native);
    }
    for source in [
        "[assert true; 1]",
        "{}.a or assert true; 1",
        "1 + assert true; 2",
        "f assert true; 1",
        "-assert true; 1",
    ] {
        assert!(
            nix::import(source).is_err(),
            "accepted invalid Nix: {source}"
        );
    }
}

#[test]
fn malformed_assertions_are_lossless_and_recovery_keeps_later_expressions() {
    roundtrip("assert(true, 1)", "assert true; 1");
    for source in [
        "assert",
        "assert()",
        "assert(c)",
        "assert(c,)",
        "assert(c)(x)",
        "assert(c, x, extra)",
        "assert(, x)",
        "assert(c,, x)",
        "assert(c, x,,)",
        "assert(c; x)",
        "assert c; x",
        "assert(c, x",
        "assert(c, @)",
        "f(assert)",
        "assert => 1",
        "{ assert = 1; }",
        "s.assert",
        "assert(false, ./path${__nxc_unsupported})",
        "assert(__nxc_bad, 1)",
        "assert(false, __nxc_bad)",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
    }
    for source in [
        "assert(@, h(3))",
        "assert(f(@, { x = 1; }), h(3))",
        "assert(@ + ''commas, ${f(1, 2)}'', h(3))",
        "f(assert(c), h(3))",
        "f(assert(c, @), h(3))",
        "f(assert(c, x, extra), h(3))",
        "[assert(c, @), h(3)]",
        "let { a = assert(@, x); yield h(3); }",
        "{ a = assert(@, x); b = h(3); }",
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
fn both_assertion_children_obey_depth_and_token_limits() {
    for condition in [false, true] {
        let wrap = |inner: &str, native| match (condition, native) {
            (false, false) => format!("assert(true, {inner})"),
            (false, true) => format!("assert true; {inner}"),
            (true, false) => format!("assert({inner}, true)"),
            (true, true) => format!("assert {inner}; true"),
        };
        let source = (0..MAX_DEPTH - 1).fold("true".to_owned(), |inner, _| wrap(&inner, false));
        let native = (0..MAX_DEPTH - 1).fold("true".to_owned(), |inner, _| wrap(&inner, true));
        roundtrip(&source, &native);
        let too_deep = wrap(&source, false);
        let parsed = syntax::parse(&too_deep);
        assert_eq!(parsed.syntax().unwrap().to_string(), too_deep);
        assert!(parsed.lower().is_err());
        assert!(nix::import(&wrap(&native, true)).is_err());
        // Native assertion chains have no delimiter nesting to limit recursion.
        let longest =
            (0..(MAX_TOKENS - 1) / 3).fold("true".to_owned(), |inner, _| wrap(&inner, true));
        assert!(nix::import(&longest).is_err());
    }
    for (item, native) in [("assert(true, 1) ", false), ("(assert true; 1) ", true)] {
        let content = format!(
            "{}{}",
            item.repeat((MAX_TOKENS - 2) / 6),
            "1 ".repeat((MAX_TOKENS - 2) % 6)
        );
        let source = format!("[{content}]");
        let ir = if native {
            nix::import(&source)
        } else {
            parse_nxc(&source)
        }
        .unwrap();
        assert!(
            emit::nxc(&ir).is_err(),
            "output commas must count toward the budget"
        );
        assert!(nix::emit(&ir).is_ok());
        let over = format!("[{content}1]");
        if native {
            assert!(nix::import(&over).is_err());
        } else {
            let parsed = syntax::parse(&over);
            assert_eq!(parsed.syntax().unwrap().to_string(), over);
            assert!(parsed.lower().is_err());
        }
    }
}

#[test]
fn native_nix_confirms_assertion_failures_order_and_laziness() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(output) => assert!(output.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    for (source, expected) in [
        ("assert true; 42", Some("42")),
        ("assert false; 42", None),
        ("assert 1; 42", None),
        ("assert null; 42", None),
        ("assert (1 / 0); 42", None),
        ("assert true; 1 / 0", None),
        ("let true = false; in assert true; 1", None),
        ("with { true = false; }; assert true; 1", Some("1")),
        ("(assert true; x: x + 1) 2", Some("3")),
        ("(assert true; { good = 1; bad = 1 / 0; }).good", Some("1")),
        ("(x: 1) (assert false; 2)", Some("1")),
        ("if false then (assert false; 1) else 2", Some("2")),
        ("builtins.head [1 (assert false; 2)]", Some("1")),
        ("({ x ? assert false; 1 }: 2) {}", Some("2")),
        ("assert assert true; true; 3", Some("3")),
        ("assert false; missing", None),
        (
            r#"builtins.tryEval (assert false; builtins.abort "body forced")"#,
            Some(r#"{"success":false,"value":false}"#),
        ),
        (
            r#"builtins.tryEval (assert (builtins.throw "condition"); builtins.abort "body forced")"#,
            Some(r#"{"success":false,"value":false}"#),
        ),
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
