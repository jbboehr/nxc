use nxc::{
    Limits, MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, emit, ir::Expr, nix, parse_nxc, syntax,
};

#[test]
fn ordinary_large_files_survive_both_conversion_directions() {
    for source in [
        format!("[{}]", "1 ".repeat(20_000)),
        format!("\"{}\"", "a".repeat(1024 * 1024 + 100)),
    ] {
        let ir = nix::import(&source).unwrap();
        let converted = emit::nxc(&ir).unwrap();
        let reparsed = parse_nxc(&converted).unwrap();
        assert_eq!(reparsed, ir);
        assert_eq!(nix::import(&nix::emit(&reparsed).unwrap()).unwrap(), ir);
    }
}

#[test]
fn diagnostics_identify_source_bytes_tokens_and_delimiter_depth() {
    for (source, kind, observed, limit) in [
        (
            " ".repeat(MAX_SOURCE_BYTES + 1),
            "source byte",
            MAX_SOURCE_BYTES + 1,
            MAX_SOURCE_BYTES,
        ),
        (
            "x ".repeat(MAX_TOKENS + 1),
            "source token",
            MAX_TOKENS + 1,
            MAX_TOKENS,
        ),
        (
            format!(
                "{}1{}",
                "(".repeat(MAX_DEPTH + 1),
                ")".repeat(MAX_DEPTH + 1)
            ),
            "delimiter depth",
            MAX_DEPTH + 1,
            MAX_DEPTH,
        ),
    ] {
        let nxc = syntax::parse(&source);
        if source.len() <= MAX_SOURCE_BYTES {
            assert_eq!(nxc.syntax().unwrap().to_string(), source);
        } else {
            assert!(nxc.syntax().is_none());
        }
        for errors in [nxc.lower().unwrap_err(), nix::import(&source).unwrap_err()] {
            assert_eq!(errors.len(), 1);
            assert_eq!(
                errors[0].message,
                format!("{kind} limit exceeded: observed {observed}, limit {limit}")
            );
        }
    }
}

#[test]
fn semantic_depth_is_distinct_from_delimiter_depth() {
    let source = format!("{}1", "- ".repeat(MAX_DEPTH));
    for errors in [
        parse_nxc(&source).unwrap_err(),
        nix::import(&source).unwrap_err(),
    ] {
        assert_eq!(
            errors[0].message,
            format!(
                "semantic depth limit exceeded: observed {}, limit {MAX_DEPTH}",
                MAX_DEPTH + 1
            )
        );
    }
}

#[test]
fn smaller_budgets_follow_cloned_parses_lowering_and_both_emitters() {
    let limits = Limits::new(100, 3).unwrap();
    let parsed = nix::parse_with_limits("a -> b", limits).unwrap();
    let clone = parsed.clone();
    drop(parsed);
    assert_eq!(
        clone.lower().unwrap_err()[0].message,
        "semantic node limit exceeded: observed 4, limit 3"
    );
    assert!(nix::import("a -> b").is_ok());

    let limits = Limits::new(100, 4).unwrap();
    let ir = nix::import_with_limits("[1 1]", limits).unwrap();
    assert_eq!(nix::emit_with_limits(&ir, limits).unwrap(), "[1 1]");
    assert_eq!(
        emit::nxc_with_limits(&ir, limits).unwrap_err().message,
        "generated nxc token limit exceeded: observed 5, limit 4"
    );
    let parsed = syntax::parse_with_limits("[1, 1]", limits);
    assert_eq!(
        parsed.clone().lower().unwrap_err()[0].message,
        "source token limit exceeded: observed 5, limit 4"
    );
    assert_eq!(parsed.syntax().unwrap().to_string(), "[1, 1]");

    let limits = Limits::new(3, 1).unwrap();
    assert_eq!((limits.source_bytes(), limits.tokens()), (3, 1));
    let ir = nxc::parse_nxc_with_limits("abc", limits).unwrap();
    assert_eq!(emit::nxc_with_limits(&ir, limits).unwrap(), "abc");
    assert_eq!(nix::emit_with_limits(&ir, limits).unwrap(), "abc");
    for error in [
        emit::nxc_with_limits(&Expr::Variable("abcd".into()), limits).unwrap_err(),
        nix::emit_with_limits(&Expr::Variable("abcd".into()), limits).unwrap_err(),
    ] {
        assert!(
            error
                .message
                .contains("byte limit exceeded: observed 4, limit 3")
        );
    }
    assert!(nxc::parse_nxc_with_limits("abc ", limits).is_err());
    assert!(nix::import_with_limits("abc ", limits).is_err());

    let zero = Limits::new(0, 0).unwrap();
    assert!(nxc::parse_nxc_with_limits("1", zero).is_err());
    assert!(emit::nxc_with_limits(&Expr::Integer(1), zero).is_err());
    assert!(nix::emit_with_limits(&Expr::Integer(1), zero).is_err());
    assert!(Limits::new(MAX_SOURCE_BYTES + 1, 1).is_none());
    assert!(Limits::new(1, MAX_TOKENS + 1).is_none());
}

#[test]
fn malformed_files_keep_lossless_text_and_bounded_diagnostics() {
    let limits = Limits::new(16_384, 4096).unwrap();
    let lexical = "§ ".repeat(200);
    let bindings: String = (0..200).map(|i| format!("a{i} = ; ")).collect();
    for source in [&lexical, &format!("let {{ {bindings} yield 1; }}")] {
        let parsed = syntax::parse_with_limits(source, limits);
        assert_eq!(parsed.syntax().unwrap().to_string(), *source);
        let errors = parsed.lower().unwrap_err();
        assert!(!errors.is_empty() && errors.len() <= 100);
    }
    let errors = nix::import_with_limits(&format!("let {bindings} in 1"), limits).unwrap_err();
    assert!(!errors.is_empty() && errors.len() <= 100);
}
