mod support;
use support::nxc;

use nxc::{MAX_DEPTH, MAX_TOKENS, emit, nix, parse_nxc, syntax};

fn roundtrip(source: &str, native: &str) {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let actual = parsed
        .lower()
        .unwrap_or_else(|e| panic!("{source:?}: {e:?}"));
    let expected = nix::import(native).unwrap_or_else(|e| panic!("{native:?}: {e:?}"));
    assert_eq!(actual, expected, "{source:?} versus {native:?}");
    let generated = emit::nxc(&expected).unwrap();
    assert_eq!(parse_nxc(&generated).unwrap(), expected, "{generated}");
    let generated = nix::emit(&actual).unwrap();
    assert_eq!(nix::import(&generated).unwrap(), actual, "{generated}");
}

#[test]
fn lists_accept_optional_commas_and_preserve_nested_elements() {
    for (source, native) in [
        ("[]", "[]"),
        ("[1,]", "[1]"),
        ("[a, b, f(x),]", "[a b (f x)]"),
        ("[a\nb\nf(x)]", "[a b (f x)]"),
        ("[1 2, 3 4,]", "[1 2 3 4]"),
        ("[[] [1, 2] [3]]", "[[] [1 2] [3]]"),
        (
            r#"["a" "${x}" { a = [1, 2]; } rec { a = [a]; }]"#,
            r#"["a" "${x}" { a = [1 2]; } rec { a = [a]; }]"#,
        ),
        ("[1, /* , ] */ 2 # ]\n 3, // ]\n 4]", "[1 2 3 4]"),
        ("({ xs ? [1, 2] }) => xs", "{ xs ? [1 2] }: xs"),
        (r#""${f([1, 2])}""#, r#""${f [1 2]}""#),
    ] {
        roundtrip(source, native);
    }
    // Generated commas are part of the nxc contract and disambiguate elements.
    assert_eq!(
        emit::nxc(&nix::import("[a b c]").unwrap()).unwrap(),
        "[a, b, c]"
    );
}

#[test]
fn list_boundaries_use_maximal_expressions() {
    for (source, native) in [
        ("[a - b]", "[(a - b)]"),
        ("[a, -b]", "[a (-b)]"),
        ("[a -b]", "[(a - b)]"),
        ("[a-b]", "[a-b]"),
        ("[1 + 2 * 3, -4]", "[(1 + 2 * 3) (-4)]"),
        ("[f (x)]", "[(f x)]"),
        ("[f, (x)]", "[f (x)]"),
        ("[f(x) y]", "[(f x) y]"),
        ("[s.a or [1, 2] 3]", "[s.a or [1 2] 3]"),
        ("[s.a or f(x)]", "[((s.a or f) x)]"),
        ("[x => x + 1, y]", "[(x: x + 1) y]"),
        ("[(x => x)(1) 2]", "[((x: x) 1) 2]"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn malformed_lists_are_lossless_errors() {
    for source in [
        "[",
        "[1",
        "[1,",
        "[,1]",
        "[1,,2]",
        "[1;2]",
        "[1)]",
        "[1 +]",
        "[1, @, 2]",
        "[./path${__curPos}]",
        "[let x = 1; in x]",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
        for diagnostic in parsed.diagnostics() {
            assert!(diagnostic.span.start <= diagnostic.span.end);
            assert!(diagnostic.span.end <= source.len());
        }
    }
    for native in [
        "[1, 2]",
        "[x: x]",
        "[{}: 1]",
        "[{x}: x]",
        "[args@{}: args]",
        "[{}.a or x: x]",
        "[-1]",
        "[1 + 2]",
    ] {
        assert!(
            nix::import(native).is_err(),
            "accepted invalid native list {native}"
        );
    }
}

#[test]
fn list_recovery_retains_later_elements_arguments_and_bindings() {
    for source in [
        "[1, @, h(3), 4]",
        "[@ + [g(1, 2), 3], h(3)]",
        r#"[@ + "${g([1, 2])}", h(3)]"#,
        "f(@ + [g(1, 2), 3], h(3))",
        "{ a = @ + [1, { b = 2; }]; b = h(3); }",
        "{ a = [@, 2]; b = h(3); }",
    ] {
        let parsed = syntax::parse(source);
        assert!(parsed.lower().is_err());
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(
            root.descendants()
                .any(|node| node.kind() == syntax::SyntaxKind::CallExpr && node.text() == "h(3)"),
            "lost later call: {root:#?}"
        );
    }
}

#[test]
fn lists_enforce_token_and_nesting_boundaries() {
    let deepest = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
    roundtrip(&deepest, &deepest);
    let over_depth = format!("[{deepest}]");
    let flat = format!("[{}]", "1 ".repeat(MAX_TOKENS - 2));
    assert_eq!(parse_nxc(&flat).unwrap(), nix::import(&flat).unwrap());
    let over_tokens = flat.replacen(']', "1]", 1);
    for source in [over_depth, over_tokens] {
        let parsed = syntax::parse(&source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert_eq!(parsed.diagnostics().len(), 1);
        assert!(parsed.lower().is_err());
        assert!(nix::import(&source).is_err());
    }
}

#[test]
fn emitters_validate_list_elements_and_output_limits() {
    use nxc::{MAX_SOURCE_BYTES, ir::Expr};
    for expr in [
        Expr::List(vec![Expr::Integer(u64::MAX)]),
        Expr::List(vec![Expr::Variable("__curPos".into())]),
        Expr::List(vec![Expr::Integer(1); MAX_TOKENS]),
        (0..MAX_DEPTH).fold(Expr::List(vec![]), |inner, _| Expr::List(vec![inner])),
    ] {
        assert!(emit::nxc(&expr).is_err());
        assert!(nix::emit(&expr).is_err());
    }

    let mut items = vec![Expr::Integer(1); (MAX_TOKENS - 2) / 2];
    items[0] = Expr::List(vec![]);
    let exact = Expr::List(items.clone());
    let source = emit::nxc(&exact).unwrap();
    assert_eq!(
        syntax::lexer::lex(&source)
            .iter()
            .filter(|t| !t.kind.is_trivia())
            .count(),
        MAX_TOKENS
    );
    assert_eq!(parse_nxc(&source).unwrap(), exact);
    items.push(Expr::Integer(1));
    assert!(emit::nxc(&Expr::List(items)).is_err());

    let exact = Expr::List(vec![Expr::Integer(1); MAX_TOKENS - 2]);
    let source = nix::emit(&exact).unwrap();
    assert_eq!(nix::import(&source).unwrap(), exact);
    assert!(nix::emit(&Expr::List(vec![Expr::Integer(1); MAX_TOKENS - 1])).is_err());

    let exact = Expr::List(vec![Expr::Variable("a".repeat(MAX_SOURCE_BYTES - 2))]);
    for source in [emit::nxc(&exact).unwrap(), nix::emit(&exact).unwrap()] {
        assert_eq!(source.len(), MAX_SOURCE_BYTES);
        assert_eq!(parse_nxc(&source).unwrap(), exact);
        assert_eq!(nix::import(&source).unwrap(), exact);
    }
    let over_size = Expr::List(vec![Expr::Variable("a".repeat(MAX_SOURCE_BYTES - 1))]);
    assert!(emit::nxc(&over_size).is_err());
    assert!(nix::emit(&over_size).is_err());
}
