use nxc::{
    MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, emit,
    ir::{BinaryOp, Expr},
    nix, parse_nxc, syntax,
};

fn balanced_sum(leaves: usize) -> String {
    if leaves == 1 {
        return "1".into();
    }
    let left = leaves / 2;
    format!("({} + {})", balanced_sum(left), balanced_sum(leaves - left))
}

fn balanced_expr(leaves: usize) -> Expr {
    if leaves == 1 {
        return Expr::Integer(1);
    }
    let left = leaves / 2;
    Expr::Binary {
        op: BinaryOp::Add,
        left: Box::new(balanced_expr(left)),
        right: Box::new(balanced_expr(leaves - left)),
    }
}

#[test]
fn exact_resource_limits_are_accepted_and_the_next_value_is_rejected() {
    let exact_size = format!("1{}", " ".repeat(MAX_SOURCE_BYTES - 1));
    assert_eq!(exact_size.len(), MAX_SOURCE_BYTES);
    assert!(parse_nxc(&exact_size).is_ok());
    assert!(nix::import(&exact_size).is_ok());
    let too_large = format!("{exact_size} ");
    assert!(parse_nxc(&too_large).is_err());
    assert!(nix::import(&too_large).is_err());

    // A 256-leaf fully parenthesized sum has 1,021 tokens. Three unary
    // operators reach the token limit without creating a deep expression.
    let at_token_limit = format!("---{}", balanced_sum(256));
    assert_eq!(
        syntax::lexer::lex(&at_token_limit)
            .iter()
            .filter(|token| !token.kind.is_trivia())
            .count(),
        MAX_TOKENS
    );
    assert_eq!(
        parse_nxc(&at_token_limit).unwrap(),
        nix::import(&at_token_limit).unwrap()
    );
    let over_token_limit = format!("-{at_token_limit}");
    let parsed = syntax::parse(&over_token_limit);
    assert_eq!(parsed.syntax().unwrap().to_string(), over_token_limit);
    assert!(parsed.lower().is_err());
    assert!(nix::import(&over_token_limit).is_err());

    let at_paren_limit = format!("{}1{}", "(".repeat(MAX_DEPTH), ")".repeat(MAX_DEPTH));
    assert_eq!(
        parse_nxc(&at_paren_limit).unwrap(),
        nix::import(&at_paren_limit).unwrap()
    );
    let over_paren_limit = format!("({at_paren_limit})");
    assert!(parse_nxc(&over_paren_limit).is_err());
    assert!(nix::import(&over_paren_limit).is_err());

    let at_expression_depth = format!("{}1", "-".repeat(MAX_DEPTH - 1));
    assert_eq!(
        parse_nxc(&at_expression_depth).unwrap(),
        nix::import(&at_expression_depth).unwrap()
    );
    let over_expression_depth = format!("-{at_expression_depth}");
    assert!(parse_nxc(&over_expression_depth).is_err());
    assert!(nix::import(&over_expression_depth).is_err());
}

#[test]
fn emitted_output_honors_exact_size_and_token_limits() {
    let exact_size = Expr::Variable("a".repeat(MAX_SOURCE_BYTES));
    for source in [
        emit::nxc(&exact_size).unwrap(),
        nix::emit(&exact_size).unwrap(),
    ] {
        assert_eq!(source.len(), MAX_SOURCE_BYTES);
        assert_eq!(parse_nxc(&source).unwrap(), exact_size);
        assert_eq!(nix::import(&source).unwrap(), exact_size);
    }
    let over_size = Expr::Variable("a".repeat(MAX_SOURCE_BYTES + 1));
    assert!(emit::nxc(&over_size).is_err());
    assert!(nix::emit(&over_size).is_err());

    let exact_tokens = Expr::Negate(Box::new(balanced_expr(256)));
    for source in [
        emit::nxc(&exact_tokens).unwrap(),
        nix::emit(&exact_tokens).unwrap(),
    ] {
        assert_eq!(
            syntax::lexer::lex(&source)
                .iter()
                .filter(|token| !token.kind.is_trivia())
                .count(),
            MAX_TOKENS
        );
        assert_eq!(parse_nxc(&source).unwrap(), exact_tokens);
        assert_eq!(nix::import(&source).unwrap(), exact_tokens);
    }
    let over_tokens = Expr::Negate(Box::new(exact_tokens));
    assert!(emit::nxc(&over_tokens).is_err());
    assert!(nix::emit(&over_tokens).is_err());
}

#[test]
fn excessive_invalid_input_produces_one_limit_error_and_a_lossless_cst() {
    let sources = [
        format!("# α\n{}", "@ ".repeat(MAX_TOKENS + 1)),
        format!("# α\n{}", "@ ".repeat(MAX_TOKENS * 2)),
        format!(
            "{}@{}",
            "(".repeat(MAX_DEPTH + 1),
            ")".repeat(MAX_DEPTH + 1)
        ),
    ];
    for source in sources {
        let parsed = syntax::parse(&source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert_eq!(root.children().count(), 1);
        assert_eq!(parsed.diagnostics().len(), 1);
        assert!(parsed.diagnostics()[0].message.contains("limit"));
        assert_eq!(parsed.diagnostics()[0].span, 0..source.len());
        assert!(parsed.lower().is_err());
    }
}
