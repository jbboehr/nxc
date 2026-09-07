use nxc::syntax::{self, SyntaxKind};

#[test]
fn malformed_nested_operator_operand_does_not_consume_the_next_outer_argument() {
    let source = "f(a == (@, h(1)), k(2))";
    let parsed = syntax::parse(source);
    let root = parsed.syntax().unwrap();

    assert_eq!(root.to_string(), source);
    assert!(parsed.lower().is_err());
    assert!(
        root.descendants()
            .any(|node| node.kind() == SyntaxKind::CallExpr && node.text() == "k(2)"),
        "lost the later outer argument during nested recovery: {root:#?}"
    );
}
