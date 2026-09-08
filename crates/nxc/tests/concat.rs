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
fn concatenation_preserves_operands_grouping_and_precedence() {
    for (source, native) in [
        ("a++b", "a ++ b"),
        ("[1]++[2]", "[1] ++ [2]"),
        ("a ++ b ++ c", "a ++ (b ++ c)"),
        ("(a ++ b) ++ c", "(a ++ b) ++ c"),
        ("a + b ++ c * d", "a + ((b ++ c) * d)"),
        ("a * b ++ c + d", "(a * (b ++ c)) + d"),
        ("a ++ b / c", "(a ++ b) / c"),
        ("-a ++ b", "(-a) ++ b"),
        ("a ++ -b", "a ++ (-b)"),
        ("!a ++ b", "!(a ++ b)"),
        ("a ++ !b", "a ++ (!b)"),
        ("a ++ b == c ++ d", "(a ++ b) == (c ++ d)"),
        ("a ++ b < c && d", "((a ++ b) < c) && d"),
        ("f(a).x ++ g(b)", "((f a).x) ++ (g b)"),
        ("s.x or a ++ b", "(s.x or a) ++ b"),
        ("__nxc_update(a ++ b, c)", "a ++ b // c"),
        ("__nxc_update(a, b ++ c)", "a // b ++ c"),
        ("/* α */ a/* β */++// γ\nb", "a ++ b"),
    ] {
        roundtrip(source, native);
    }
    // Even literal operands stay separate: conversion must not fold lists.
    for ir in [
        parse_nxc("[1] ++ [2]").unwrap(),
        nix::import("[1] ++ [2]").unwrap(),
    ] {
        let Expr::Binary { left, right, .. } = ir else {
            panic!("concatenation must remain binary")
        };
        assert_eq!(*left, Expr::List(vec![Expr::Integer(1)]));
        assert_eq!(*right, Expr::List(vec![Expr::Integer(2)]));
    }
}

#[test]
fn concatenation_composes_with_lists_calls_and_full_expressions() {
    for (source, native) in [
        ("[a ++ b c ++ d]", "[(a ++ b) (c ++ d)]"),
        ("[a, b] ++ [c, d]", "[a b] ++ [c d]"),
        ("[[a]] ++ [[b]]", "[[a]] ++ [[b]]"),
        ("f(a ++ b, c)", "f (a ++ b) c"),
        ("(a ++ b)(c)", "(a ++ b) c"),
        ("(a ++ b).x", "(a ++ b).x"),
        ("s.x or (a ++ b)", "s.x or (a ++ b)"),
        ("x => x ++ []", "x: x ++ []"),
        ("({ x ? a ++ b }) => x", "{ x ? a ++ b }: x"),
        ("a ++ (x => x)", "a ++ (x: x)"),
        ("a ++ f(x => x)", "a ++ f (x: x)"),
        ("f(x => x) ++ a", "(f (x: x)) ++ a"),
        ("a ++ (if c then b else [])", "a ++ (if c then b else [])"),
        ("if c then a ++ b else d", "if c then a ++ b else d"),
        ("assert(c, a ++ b)", "assert c; a ++ b"),
        ("with(s, a ++ b)", "with s; a ++ b"),
        ("a ++ let { x = b; yield x; }", "a ++ (let x = b; in x)"),
        ("let { x = a ++ b; yield x; }", "let x = a ++ b; in x"),
        ("{ inherit (a ++ b) x; }", "{ inherit (a ++ b) x; }"),
        (r#""${a ++ b}""#, r#""${a ++ b}""#),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn native_concatenation_calls_require_parenthesized_lambda_arguments() {
    for source in ["a ++ f x: x", "(f x: x) ++ a"] {
        assert!(nix::import(source).is_err(), "accepted {source}");
    }
}

#[test]
fn malformed_concatenation_is_lossless_and_recovers_later_items() {
    roundtrip("a ++ b", "a ++ b");
    for source in [
        "++",
        "++a",
        "a++",
        "a+++b",
        "a++++b",
        "a + + b",
        "a ++ ++ b",
        "a ++ x => x",
        "a ++ if c then b else d",
        "a++b/c/",
        "/a++b",
        "[] ++ /path",
        "__nxc_bad ++ []",
        "a -> b",
        "a ?",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
    }
    for source in [
        "f(a ++ @, h(3))",
        "f(a ++, h(3))",
        "f(a +++ b, h(3))",
        "[a ++ @, h(3)]",
        "f(a ++ (@, g(2)), h(3))",
        "{ a = x ++ @; b = h(3); }",
        "let { a = x ++ @; yield h(3); }",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(
            root.descendants()
                .any(|n| n.kind() == syntax::SyntaxKind::CallExpr && n.text() == "h(3)"),
            "lost later item: {root:#?}"
        );
    }
}

#[test]
fn concatenation_validates_both_operands_and_resource_limits() {
    let concat = nix::import("a ++ b").unwrap();
    for invalid in [
        Expr::Integer(u64::MAX),
        Expr::Variable("__nxc_bad".into()),
        (0..MAX_DEPTH - 1).fold(Expr::Integer(1), |e, _| Expr::Not(Box::new(e))),
        Expr::List(vec![Expr::Integer(1); MAX_TOKENS]),
    ] {
        for in_left in [true, false] {
            let mut expr = concat.clone();
            let Expr::Binary { left, right, .. } = &mut expr else {
                panic!("concatenation must remain binary")
            };
            **if in_left { left } else { right } = invalid.clone();
            assert!(emit::nxc(&expr).is_err());
            assert!(nix::emit(&expr).is_err());
        }
    }
    for in_left in [true, false] {
        let wrap = |inner: &str| {
            if in_left {
                format!("({inner} ++ a)")
            } else {
                format!("a ++ {inner}")
            }
        };
        let source = (0..MAX_DEPTH - 1).fold("a".to_owned(), |inner, _| wrap(&inner));
        roundtrip(&source, &source);
        let over = wrap(&source);
        let parsed = syntax::parse(&over);
        assert_eq!(parsed.syntax().unwrap().to_string(), over);
        assert!(parsed.lower().is_err());
        assert!(nix::import(&over).is_err());
    }
    let longest = format!("{}a", "a ++ ".repeat((MAX_TOKENS - 1) / 2));
    let parsed = syntax::parse(&longest);
    assert_eq!(parsed.syntax().unwrap().to_string(), longest);
    assert!(parsed.lower().is_err());
    assert!(nix::import(&longest).is_err());
    for (item, tokens, native) in [("a++b ", 3, false), ("(a++b) ", 5, true)] {
        let content = format!(
            "{}{}",
            item.repeat((MAX_TOKENS - 2) / tokens),
            "a ".repeat((MAX_TOKENS - 2) % tokens)
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
            "output parentheses and commas must count toward the limit"
        );
        assert_eq!(nix::emit(&ir).is_ok(), native);
        let over = format!("[{content}a]");
        if native {
            assert!(nix::import(&over).is_err());
        } else {
            let parsed = syntax::parse(&over);
            assert_eq!(parsed.syntax().unwrap().to_string(), over);
            assert!(parsed.lower().is_err());
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
fn native_concatenation_operand_boundaries_match_nix() {
    roundtrip("a ++ b", "a ++ b");
    if !nix_available() {
        return;
    }
    let accepts = |source: &str| {
        let wrapped = format!("a: b: c: f: s: ({source})");
        Command::new("nix-instantiate")
            .args(["--store", "dummy://", "--parse", "--expr", &wrapped])
            .output()
            .unwrap()
            .status
            .success()
    };
    for source in [
        "a++b",
        "a ++ b ++ c",
        "!a ++ b",
        "a ++ -b",
        "(a ++ b) ++ c",
        "a ++ b == c",
        "a // b ++ c",
        "[(a ++ b)]",
        "[a ++ b]",
        "f (a ++ b)",
        "f a ++ b",
        "s.a or a ++ b",
        "s.a or (a ++ b)",
        "a ++ (x: x)",
        "a ++ f x: x",
        "(f x: x) ++ a",
        "a ++ f (x: x)",
        "(f (x: x)) ++ a",
        "a ++ x: x",
        "a ++ { x }: x",
        "a ++ if c then b else a",
        "a ++ assert true; b",
        "a ++ let in b",
        "a+++b",
        "a++++b",
        "a ++",
        "++a",
    ] {
        assert_eq!(nix::import(source).is_ok(), accepts(source), "{source}");
    }
    // rnix requires parentheses here, although native Nix accepts the bare prefix.
    assert!(accepts("a ++ !b"));
    assert!(nix::import("a ++ !b").is_err());
}

#[test]
fn native_nix_confirms_list_order_nesting_and_lazy_elements() {
    if !nix_available() {
        return;
    }
    for (source, expected) in [
        ("[] ++ []", Some("[]")),
        ("[1 2] ++ [3 4]", Some("[1,2,3,4]")),
        ("[[1]] ++ [[2 3]]", Some("[[1],[2,3]]")),
        ("[] ++ [1]", Some("[1]")),
        ("[1] ++ []", Some("[1]")),
        ("[1] ++ [2] ++ [3]", Some("[1,2,3]")),
        ("builtins.head ([1] ++ [(1 / 0)])", Some("1")),
        ("builtins.elemAt ([(1 / 0)] ++ [2]) 1", Some("2")),
        ("builtins.length ([(1 / 0)] ++ [(1 / 0)])", Some("2")),
        ("builtins.length ([] ++ [(1 / 0)])", Some("1")),
        ("builtins.head ([1] ++ (assert false; []))", None),
        ("(assert false; []) ++ []", None),
        ("[] ++ (assert false; [])", None),
        ("[] ++ 1", None),
        ("1 ++ []", None),
        ("null ++ []", None),
        ("[] ++ {}", None),
        (r#""a" ++ "b""#, None),
        ("[] ++ (x: x)", None),
        ("(x: 1) (1 ++ [])", Some("1")),
        ("if false then [] ++ 1 else 2", Some("2")),
        ("let builtins = 1; in [1] ++ [2]", Some("[1,2]")),
        ("let x = 1; in [x] ++ (let x = 2; in [x])", Some("[1,2]")),
        (
            "let xs = [1] ++ [xs]; in builtins.head (builtins.elemAt xs 1)",
            Some("1"),
        ),
        ("let f = x: x; in ([f] ++ []) == [f]", Some("true")),
        (
            "(builtins.tryEval ((builtins.throw \"left\") ++ (builtins.abort \"right forced\"))).success",
            Some("false"),
        ),
        (
            "(builtins.tryEval ((builtins.abort \"left forced\") ++ (builtins.throw \"right\"))).success",
            None,
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
