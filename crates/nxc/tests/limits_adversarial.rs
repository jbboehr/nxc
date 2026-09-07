use nxc::{MAX_TOKENS, nix, parse_nxc, syntax};

fn balanced_sum(leaves: usize) -> String {
    if leaves == 1 {
        return "1".into();
    }
    let left = leaves / 2;
    format!("({} + {})", balanced_sum(left), balanced_sum(leaves - left))
}

#[test]
fn exact_token_limit_clone_survives_original_parsed_drop() {
    let source = format!("---{}", balanced_sum(MAX_TOKENS / 4));
    assert_eq!(
        syntax::lexer::lex(&source)
            .iter()
            .filter(|token| !token.kind.is_trivia())
            .count(),
        MAX_TOKENS
    );

    let parsed = nix::parse(&source).unwrap();
    let surviving_clone = parsed.clone();
    drop(parsed);

    assert_eq!(
        surviving_clone.lower().unwrap(),
        parse_nxc(&source).unwrap()
    );
}
