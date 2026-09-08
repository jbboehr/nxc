use nxc::{
    MAX_DEPTH, MAX_TOKENS, emit,
    ir::{BinaryOp, Expr},
    nix, parse_nxc, syntax,
};
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
fn comparison_and_boolean_spellings_roundtrip_without_folding() {
    for (spelling, op) in [
        ("==", BinaryOp::Equal),
        ("!=", BinaryOp::NotEqual),
        ("<", BinaryOp::Less),
        ("<=", BinaryOp::LessOrEqual),
        (">", BinaryOp::Greater),
        (">=", BinaryOp::GreaterOrEqual),
        ("&&", BinaryOp::And),
        ("||", BinaryOp::Or),
    ] {
        let expected = Expr::Binary {
            op,
            left: Box::new(Expr::Variable("a".into())),
            right: Box::new(Expr::Variable("b".into())),
        };
        for source in [format!("a {spelling} b"), format!("a{spelling}b")] {
            assert_eq!(parse_nxc(&source).unwrap(), expected);
            assert_eq!(nix::import(&source).unwrap(), expected);
            roundtrip(&source, &format!("a {spelling} b"));
        }
    }
    let expected = Expr::Not(Box::new(Expr::Variable("a".into())));
    assert_eq!(parse_nxc("!a").unwrap(), expected);
    assert_eq!(nix::import("!a").unwrap(), expected);
    for source in ["!true", "!!false", "! -1", "!(a == b)", "!foo-bar'"] {
        roundtrip(source, source);
    }
    roundtrip("/* α */ a/* β */!=// γ\nb", "a != b");
}

#[test]
fn emitters_validate_all_operator_operands_without_evaluating_them() {
    for invalid in [
        Expr::Integer(u64::MAX),
        Expr::Variable("if".into()),
        (0..MAX_DEPTH - 1).fold(Expr::Integer(1), |inner, _| Expr::Not(Box::new(inner))),
        Expr::List(vec![Expr::Integer(1); MAX_TOKENS]),
    ] {
        let not = Expr::Not(Box::new(invalid.clone()));
        assert!(emit::nxc(&not).is_err());
        assert!(nix::emit(&not).is_err());
        for op in [
            BinaryOp::Equal,
            BinaryOp::NotEqual,
            BinaryOp::Less,
            BinaryOp::LessOrEqual,
            BinaryOp::Greater,
            BinaryOp::GreaterOrEqual,
            BinaryOp::And,
            BinaryOp::Or,
        ] {
            for in_left in [true, false] {
                let valid =
                    Expr::Variable(if op == BinaryOp::Or { "true" } else { "false" }.into());
                let (left, right) = if in_left {
                    (invalid.clone(), valid)
                } else {
                    (valid, invalid.clone())
                };
                let expr = Expr::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                };
                assert!(emit::nxc(&expr).is_err());
                assert!(nix::emit(&expr).is_err());
            }
        }
    }
}

#[test]
fn operator_precedence_and_associativity_match_explicit_native_grouping() {
    for (source, native) in [
        ("a || b && c", "a || (b && c)"),
        ("a && b || c", "(a && b) || c"),
        ("a && b && c", "(a && b) && c"),
        ("a || b || c", "(a || b) || c"),
        ("a == b && c != d", "(a == b) && (c != d)"),
        ("a < b == c >= d", "(a < b) == (c >= d)"),
        ("a != b <= c", "a != (b <= c)"),
        ("a + b * c < d - 1", "(a + (b * c)) < (d - 1)"),
        ("!a + b", "!(a + b)"),
        ("!a < b", "(!a) < b"),
        ("!a == b", "(!a) == b"),
        ("!a && b", "(!a) && b"),
        ("a == !b", "a == (!b)"),
        ("1 + !true", "1 + (!true)"),
        ("-!true", "-(!true)"),
        ("! -a * b", "!((-a) * b)"),
        ("a == (b == c)", "a == (b == c)"),
        ("(a < b) < c", "(a < b) < c"),
        ("a < (b < c)", "a < (b < c)"),
        ("!f(x).a == g(y)", "(!((f x).a)) == (g y)"),
        ("s.a or fallback == other", "(s.a or fallback) == other"),
        ("s.a or (x || y)", "s.a or (x || y)"),
    ] {
        roundtrip(source, native);
    }
    for ops in [&["<", "<=", ">", ">="][..], &["==", "!="][..]] {
        for left in ops {
            for right in ops {
                let source = format!("a {left} b {right} c");
                let parsed = syntax::parse(&source);
                assert_eq!(parsed.syntax().unwrap().to_string(), source);
                assert!(
                    parsed.lower().is_err(),
                    "accepted chained comparison: {source}"
                );
                assert!(nix::import(&source).is_err(), "native accepted {source}");
            }
        }
    }
}

#[test]
fn operators_compose_with_delimited_and_full_expressions() {
    for (source, native) in [
        ("f(a == b, !c)", "f (a == b) (!c)"),
        ("[a < b, !c, d && e]", "[(a < b) (!c) (d && e)]"),
        ("[a < b !c d && e]", "[(a < b) (!c) (d && e)]"),
        ("(a || b)(x)", "(a || b) x"),
        ("(a && b).x", "(a && b).x"),
        ("x => x >= 1 && x <= 9", "x: x >= 1 && x <= 9"),
        ("({ x ? a == b }) => !x", "{ x ? a == b }: !x"),
        (
            "if a != b then !c else d || e",
            "if a != b then !c else d || e",
        ),
        ("a && (if b then c else d)", "a && (if b then c else d)"),
        ("assert(a && !b, c >= d)", "assert a && !b; c >= d"),
        ("with(scope, a == b) || c", "(with scope; a == b) || c"),
        ("let { x = a != b; yield !x; }", "let x = a != b; in !x"),
        ("{ x = a <= b; }", "{ x = a <= b; }"),
        ("{ inherit (a || b) x; }", "{ inherit (a || b) x; }"),
        (
            r#""${if a == b then "yes" else "no"}""#,
            r#""${if a == b then "yes" else "no"}""#,
        ),
        ("a == (x => x)", "a == (x: x)"),
        ("!(x => x)", "!(x: x)"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn malformed_and_unsupported_operator_inputs_stay_lossless_and_recover() {
    roundtrip("a == b", "a == b");
    for source in [
        "!",
        "a ==",
        "a &&",
        "|| a",
        "a & b",
        "a | b",
        "a === b",
        "a !== b",
        "a =< b",
        "a &&& b",
        "a ||| b",
        "a ** b",
        "a -> b",
        "a +++ b",
        "a |> b",
        "a <| b",
        "<nixpkgs/>",
        "a <b> c",
        "a ?",
        "a || if b then c else d",
        "!if a then b else c",
        "a == x => x",
        "s.a or !x",
        "false && ~/foo",
        "true || __nxc_bad",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
    }
    for source in [
        "f(a == @, h(3))",
        "f(!@, h(3))",
        "[a && @, h(3)]",
        "{ x = a < @; y = h(3); }",
        "let { x = a || @; yield h(3); }",
        "f(a < b < c, h(3))",
        "[a == b != c, h(3)]",
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
    for source in [
        "[!true]",
        "[true && false]",
        "f !true",
        "s.a or !true",
        "a == x: x",
        "true && x: x",
        "!x: x",
        "1 < x: x",
        "true && if true then true else false",
        "false || assert true; true",
    ] {
        assert!(
            nix::import(source).is_err(),
            "accepted invalid Nix: {source}"
        );
    }
}

#[test]
fn unary_and_binary_operators_obey_depth_and_token_limits() {
    for op in ["!", "true && ", "false || "] {
        let source = format!("{}true", op.repeat(MAX_DEPTH - 1));
        roundtrip(&source, &source);
        let over = format!("{op}{source}");
        let parsed = syntax::parse(&over);
        assert_eq!(parsed.syntax().unwrap().to_string(), over);
        assert!(parsed.lower().is_err());
        assert!(nix::import(&over).is_err());
    }
    for op in ["==", "!=", "<", "<=", ">", ">="] {
        for in_left in [true, false] {
            let wrap = |inner: &str| {
                if in_left {
                    format!("({inner} {op} 1)")
                } else {
                    format!("(1 {op} {inner})")
                }
            };
            let source = (0..MAX_DEPTH - 1).fold("1".to_owned(), |inner, _| wrap(&inner));
            roundtrip(&source, &source);
            assert!(parse_nxc(&wrap(&source)).is_err());
            assert!(nix::import(&wrap(&source)).is_err());
        }
    }
    let longest = format!("{}true", "!".repeat(MAX_TOKENS - 1));
    let parsed = syntax::parse(&longest);
    assert_eq!(parsed.syntax().unwrap().to_string(), longest);
    assert!(parsed.lower().is_err());
    assert!(nix::import(&longest).is_err());
    for (item, tokens, native) in [("1==1 ", 3, false), ("(1==1) ", 5, true)] {
        let content = format!(
            "{}{}",
            item.repeat((MAX_TOKENS - 2) / tokens),
            "1 ".repeat((MAX_TOKENS - 2) % tokens)
        );
        let source = format!("[{content}]");
        let ir = if native {
            nix::import(&source)
        } else {
            parse_nxc(&source)
        }
        .unwrap();
        assert!(emit::nxc(&ir).is_err());
        assert_eq!(nix::emit(&ir).is_ok(), native);
        let over = format!("[{content}1]");
        if native {
            assert!(nix::import(&over).is_err());
        } else {
            assert!(parse_nxc(&over).is_err());
        }
    }
}

fn nix_available() -> bool {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(output) => {
            assert!(output.status.success());
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
fn native_operator_operand_boundaries_match_nix() {
    if !nix_available() {
        return;
    }
    let native_accepts = |source: &str| {
        // Bind names so --parse does not reject otherwise valid syntax during
        // Nix's variable binding pass.
        let wrapped = format!("a: b: c: f: s: ({source})");
        Command::new("nix-instantiate")
            .args(["--store", "dummy://", "--parse", "--expr", &wrapped])
            .output()
            .unwrap()
            .status
            .success()
    };
    for source in [
        "!a == b",
        "a < b == c",
        "a == !b",
        "!!a",
        "! -1",
        "(a < b) < c",
        "a < (b < c)",
        "a == (b != c)",
        "a < b < c",
        "a == b != c",
        "a != b == c",
        "[(a == b) (!a)]",
        "[a == b]",
        "[!a]",
        "f !a",
        "s.a or !b",
        "s.a or (!b)",
        "a && (x: x)",
        "!(x: x)",
        "a == x: x",
        "a != x: x",
        "a < x: x",
        "a <= x: x",
        "a > x: x",
        "a >= x: x",
        "a && x: x",
        "a || x: x",
        "!x: x",
    ] {
        assert_eq!(
            nix::import(source).is_ok(),
            native_accepts(source),
            "{source}"
        );
    }
    // These valid Nix forms exceed rnix's grammar. Keep the limitation explicit;
    // the equivalent parenthesized inputs and nxc spellings are tested above.
    for source in ["1 + !true", "-!true"] {
        assert!(native_accepts(source));
        assert!(nix::import(source).is_err());
    }
}

#[test]
fn native_nix_confirms_values_short_circuiting_and_comparison_laziness() {
    if !nix_available() {
        return;
    }
    for (source, expected) in [
        ("!true", Some("false")),
        ("!!true", Some("true")),
        ("1 == 1", Some("true")),
        ("1 != 1", Some("false")),
        ("1 < 2", Some("true")),
        ("2 <= 2", Some("true")),
        ("2 > 1", Some("true")),
        ("2 >= 2", Some("true")),
        ("2 < 1", Some("false")),
        ("2 <= 1", Some("false")),
        ("1 > 2", Some("false")),
        ("1 >= 2", Some("false")),
        ("false || true && false", Some("false")),
        ("true || false && false", Some("true")),
        ("false && (1 / 0)", Some("false")),
        ("true || (1 / 0)", Some("true")),
        ("true && false", Some("false")),
        ("false || true", Some("true")),
        ("true && 1", None),
        ("false || 1", None),
        ("1 && false", None),
        ("1 || true", None),
        ("!1", None),
        ("true < false", None),
        ("1 == true", Some("false")),
        (r#""aa" < "b""#, Some("true")),
        (r#""a" + "b" == "ab""#, Some("true")),
        ("[1 (1 / 0)] == [2 (1 / 0)]", Some("false")),
        ("[1 (1 / 0)] < [2 (1 / 0)]", Some("true")),
        ("{ a = 1 / 0; } == { b = 1 / 0; }", Some("false")),
        ("let x = { self = x; }; in x == x", Some("true")),
        ("let f = x: x; in [f] == [f]", Some("true")),
        ("let f = x: x; in f == f", Some("false")),
        ("let true = false; in true || false", Some("false")),
        ("assert 1 < 2 && !(3 == 4); 5", Some("5")),
        ("if 1 + 2 * 3 >= 7 then 1 else 2", Some("1")),
        ("false && missing", None),
        ("true || missing", None),
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
