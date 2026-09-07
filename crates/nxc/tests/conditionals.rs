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
fn conditionals_keep_all_three_expressions_and_nested_branch_boundaries() {
    let expected = Expr::If {
        condition: Box::new(Expr::Variable("condition".into())),
        then_branch: Box::new(Expr::Variable("yes".into())),
        else_branch: Box::new(Expr::Variable("no".into())),
    };
    assert_eq!(
        parse_nxc("if condition then yes else no").unwrap(),
        expected
    );
    assert_eq!(
        nix::import("if condition then yes else no").unwrap(),
        expected
    );
    for (source, native) in [
        (
            "if condition then yes else no",
            "if condition then yes else no",
        ),
        (
            "if (condition) then yes else no",
            "if condition then yes else no",
        ),
        ("if f(1) then g(2) else h(3)", "if f 1 then g 2 else h 3"),
        ("if c then 1 + 2 else 3 * 4", "if c then 1 + 2 else 3 * 4"),
        (
            "if if a then b else c then d else e",
            "if (if a then b else c) then d else e",
        ),
        (
            "if a then if b then c else d else e",
            "if a then (if b then c else d) else e",
        ),
        (
            "if a then b else if c then d else e",
            "if a then b else (if c then d else e)",
        ),
        (
            "if c then x => x else y => y + 1",
            "if c then x: x else y: y + 1",
        ),
        ("if x => x then 1 else 2", "if x: x then 1 else 2"),
        (
            "if let { yield true; } then with({}, 1) else let { yield 2; }",
            "if let in true then with {}; 1 else let in 2",
        ),
        (
            "/* α */ if // β\ntrue then /* γ */ 1 else 2",
            "if true then 1 else 2",
        ),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn emitters_validate_every_conditional_child_without_selecting_a_branch() {
    for invalid in [
        Expr::Integer(u64::MAX),
        Expr::Variable("then".into()),
        (0..MAX_DEPTH - 1).fold(Expr::Integer(1), |inner, _| Expr::Negate(Box::new(inner))),
        Expr::List(vec![Expr::Integer(1); MAX_TOKENS]),
    ] {
        for position in 0..3 {
            let mut children = [Expr::Integer(1), Expr::Integer(2), Expr::Integer(3)];
            children[position] = invalid.clone();
            let [condition, then_branch, else_branch] = children;
            let expr = Expr::If {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            };
            assert!(emit::nxc(&expr).is_err(), "accepted {expr:?}");
            assert!(nix::emit(&expr).is_err(), "accepted {expr:?}");
        }
    }
}

#[test]
fn conditionals_compose_with_existing_expression_positions() {
    for (source, native) in [
        ("(if c then f else g)(x)", "(if c then f else g) x"),
        ("(if c then a else b).x", "(if c then a else b).x"),
        ("s.a or (if c then a else b)", "s.a or (if c then a else b)"),
        ("1 + (if c then 2 else 3)", "1 + (if c then 2 else 3)"),
        ("-(if c then 1 else 2)", "-(if c then 1 else 2)"),
        (
            "if c then a else b(x).y + 1",
            "if c then a else (b x).y + 1",
        ),
        (
            "[if c then 1 else 2, if c then 3 else 4]",
            "[(if c then 1 else 2) (if c then 3 else 4)]",
        ),
        (
            "[if c then 1 else 2 if c then 3 else 4]",
            "[(if c then 1 else 2) (if c then 3 else 4)]",
        ),
        ("f(if c then 1 else 2, 3)", "f (if c then 1 else 2) 3"),
        ("x => if x then 1 else 2", "x: if x then 1 else 2"),
        (
            "({ x ? if c then 1 else 2 }) => x",
            "{ x ? if c then 1 else 2 }: x",
        ),
        ("{ x = if c then 1 else 2; }", "{ x = if c then 1 else 2; }"),
        (
            "let { x = if c then 1 else 2; yield if c then x else 3; }",
            "let x = if c then 1 else 2; in if c then x else 3",
        ),
        (
            "{ inherit (if c then a else b) x; }",
            "{ inherit (if c then a else b) x; }",
        ),
        (
            "with(if c then a else b, if c then x else y)",
            "with (if c then a else b); if c then x else y",
        ),
        (
            r#""${if c then "a" else ''b''}""#,
            r#""${if c then "a" else ''b''}""#,
        ),
    ] {
        roundtrip(source, native);
    }
    for source in [
        "1 + if c then 2 else 3",
        "s.a or if c then 1 else 2",
        "-if c then 1 else 2",
    ] {
        assert!(parse_nxc(source).is_err(), "accepted {source}");
        assert!(nix::import(source).is_err(), "native accepted {source}");
    }
    for source in ["f if c then 1 else 2", "[if c then 1 else 2]"] {
        assert!(
            nix::import(source).is_err(),
            "accepted invalid Nix: {source}"
        );
    }
}

#[test]
fn malformed_conditionals_are_lossless_and_keep_later_enclosing_items() {
    roundtrip("if true then 1 else 2", "if true then 1 else 2");
    for source in [
        "if",
        "if true",
        "if true then",
        "if true then 1",
        "if true then 1 else",
        "if then 1 else 2",
        "if true then else 2",
        "if true else 2",
        "if true 1 else 2",
        "if true then 1 else 2 else 3",
        "if true then if false then 1 else 2",
        "if(true, 1, 2)",
        "if (true) { 1 } else { 2 }",
        "if true then @ else 2",
        "if true then 1 else /foo",
        "if true then 1 else __nxc_bad",
        "if __nxc_bad then 1 else 2",
        "if false then __nxc_bad else 1",
        "{ if = 1; }",
        "s.then",
        "x => else",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
    }
    for source in [
        "f(if @ then 1 else 2, h(3))",
        "f(if true then @ else 2, h(3))",
        "f(if true then 1 else @, h(3))",
        "f(if true then f(@, [1, 2]) else 2, h(3))",
        "[if true then 1 else @, h(3)]",
        "let { x = if true then @ else 2; yield h(3); }",
        "{ x = if true then @ else 2; y = h(3); }",
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
fn all_conditional_children_obey_depth_and_token_limits() {
    for position in 0..3 {
        let wrap = |inner: &str| match position {
            0 => format!("if {inner} then true else false"),
            1 => format!("if true then {inner} else false"),
            _ => format!("if true then false else {inner}"),
        };
        let source = (0..MAX_DEPTH - 1).fold("true".to_owned(), |inner, _| wrap(&inner));
        roundtrip(&source, &source);
        let too_deep = wrap(&source);
        let parsed = syntax::parse(&too_deep);
        assert_eq!(parsed.syntax().unwrap().to_string(), too_deep);
        assert!(parsed.lower().is_err());
        assert!(nix::import(&too_deep).is_err());

        // Bare conditionals can exceed semantic depth without delimiter nesting.
        // Exercise the longest chain that still passes the token preflight.
        let longest = (0..(MAX_TOKENS - 1) / 5).fold("true".to_owned(), |inner, _| wrap(&inner));
        let parsed = syntax::parse(&longest);
        assert_eq!(parsed.syntax().unwrap().to_string(), longest);
        assert!(parsed.lower().is_err());
        assert!(nix::import(&longest).is_err());
    }
    // Unparenthesized nxc list elements use six tokens; native elements use eight.
    for (item, tokens, native) in [
        ("if true then 1 else 2 ", 6, false),
        ("(if true then 1 else 2) ", 8, true),
    ] {
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
        // Canonical nxc parentheses and commas can exceed the output budget.
        assert!(emit::nxc(&ir).is_err());
        assert_eq!(nix::emit(&ir).is_ok(), native);
        let too_many = format!("[{content}1]");
        if native {
            assert!(nix::import(&too_many).is_err());
        } else {
            let parsed = syntax::parse(&too_many);
            assert_eq!(parsed.syntax().unwrap().to_string(), too_many);
            assert!(parsed.lower().is_err());
        }
    }
}

#[test]
fn native_nix_confirms_branch_selection_laziness_and_condition_failures() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(output) => assert!(output.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    for (source, expected) in [
        ("if true then 1 else 2", Some("1")),
        ("if (false) then 1 else 2", Some("2")),
        ("if true then 1 else 1 / 0", Some("1")),
        ("if false then 1 / 0 else 2", Some("2")),
        ("if if false then true else true then 1 else 2", Some("1")),
        ("if true then if false then 1 else 2 else 3", Some("2")),
        ("if false then 1 else if true then 2 else 3", Some("2")),
        ("(if true then x: x else y: y + 1) 4", Some("4")),
        ("let true = false; in if true then 1 else 2", Some("2")),
        ("with { true = false; }; if true then 1 else 2", Some("1")),
        (
            "(if true then { a = 1; bad = 1 / 0; } else {}).a",
            Some("1"),
        ),
        ("if 1 then 1 else 2", None),
        ("if null then 1 else 2", None),
        ("if (1 / 0) then 1 else 2", None),
        ("if true then 1 / 0 else 2", None),
        ("if true then 1 else missing", None),
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
