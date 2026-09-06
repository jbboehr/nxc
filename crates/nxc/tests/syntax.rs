use nxc::syntax::{self, SyntaxKind, lexer};

#[test]
fn lexer_spans_partition_utf8_source_including_trivia_and_errors() {
    let source = "# α\nf /* β */ (1, // γ\n 🦀)";
    let tokens = lexer::lex(source);
    let mut cursor = 0;
    for token in &tokens {
        assert_eq!(token.span.start, cursor);
        assert!(token.span.end > cursor);
        assert!(source.is_char_boundary(token.span.end));
        cursor = token.span.end;
    }
    assert_eq!(cursor, source.len());
    assert!(tokens.iter().any(|t| t.kind.is_trivia()));
    assert!(tokens.iter().any(|t| t.kind == SyntaxKind::ErrorToken));
}

#[test]
fn cst_preserves_source_even_when_parsing_fails() {
    for source in [
        "",
        " \t\n",
        "// hello\r\nf /* mid */ (1, x,)\n",
        "f(1, , 2)",
        "f(1 + @, g(2), @)",
        "f(",
        "/* unterminated 🦀",
        "\"${broken\"",
        "[not supported]",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source, "{source:?}");
    }
}

#[test]
fn cst_contains_expression_structure_and_recovers_at_argument_boundaries() {
    let parsed = syntax::parse("f(1, @, g(2), $)");
    assert!(parsed.diagnostics().len() >= 2);
    assert!(parsed.lower().is_err());
    let kinds: Vec<_> = parsed
        .syntax()
        .unwrap()
        .descendants()
        .map(|n| n.kind())
        .collect();
    assert_eq!(
        kinds.iter().filter(|&&k| k == SyntaxKind::CallExpr).count(),
        2
    );
    assert!(kinds.contains(&SyntaxKind::ErrorExpr));
}

#[test]
fn malformed_nested_argument_retains_later_outer_arguments() {
    let parsed = syntax::parse("f(@ + g(1, 2), h(3), 4)");
    assert!(!parsed.diagnostics().is_empty());
    assert!(parsed.lower().is_err());

    let root = parsed.syntax().unwrap();
    let call_texts: Vec<_> = root
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::CallExpr)
        .map(|node| node.text().to_string())
        .collect();
    assert!(
        call_texts.iter().any(|text| text == "h(3)"),
        "later valid call argument was lost: {call_texts:?}"
    );
    let integer_texts: Vec<_> = root
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::IntegerExpr)
        .map(|node| node.text().to_string())
        .collect();
    assert!(
        integer_texts.iter().any(|text| text == "4"),
        "later valid integer argument was lost: {integer_texts:?}"
    );
}

#[test]
fn recovery_skips_nested_parentheses_before_the_outer_separator() {
    let source = "f(@ + g((1 + 2), i(3, 4)), h(5, 6), 7,)";
    let parsed = syntax::parse(source);
    assert!(!parsed.diagnostics().is_empty());
    assert!(parsed.lower().is_err());
    let root = parsed.syntax().unwrap();
    assert_eq!(root.to_string(), source);
    let outer = root
        .children()
        .find(|node| node.kind() == SyntaxKind::CallExpr)
        .expect("outer call should survive recovery");
    let arguments: Vec<_> = outer
        .children()
        .skip(1)
        .map(|node| (node.kind(), node.to_string()))
        .collect();
    assert_eq!(
        arguments,
        [
            (SyntaxKind::ErrorExpr, "@ + g((1 + 2), i(3, 4))".into()),
            (SyntaxKind::CallExpr, "h(5, 6)".into()),
            (SyntaxKind::IntegerExpr, "7".into()),
        ]
    );
}

#[test]
fn successful_parsing_has_no_diagnostics() {
    let parsed = syntax::parse(" /* hi */ f(1 + 2, x) ");
    assert!(
        parsed.diagnostics().is_empty(),
        "{:?}",
        parsed.diagnostics()
    );
    assert!(parsed.lower().is_ok());
}
