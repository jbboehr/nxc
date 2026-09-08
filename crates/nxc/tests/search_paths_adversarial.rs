use nxc::{emit, nix, parse_nxc, syntax};

#[test]
fn comparison_comments_do_not_become_malformed_search_path_tokens() {
    let expected = parse_nxc("f(1 < 2, good(<ok>))").unwrap();
    for marker in ["//", "#"] {
        // Angle brackets and delimiters inside a real comment remain trivia.
        let source = format!("f(1<2{marker} ignored>, good(<ignored>))\n, good(<ok>))");
        let parsed = syntax::parse(&source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        let actual = parsed.lower().unwrap();
        assert_eq!(actual, expected);
        assert_eq!(parse_nxc(&emit::nxc(&actual).unwrap()).unwrap(), expected);
        assert_eq!(nix::import(&nix::emit(&actual).unwrap()).unwrap(), expected);
    }
}
