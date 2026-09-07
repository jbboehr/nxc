use nxc::{emit, ir::Expr, nix, parse_nxc, syntax};

#[test]
fn path_components_that_look_like_other_tokens_remain_literal_paths() {
    for path in [
        "if/then",
        "else/let",
        "fn/yield",
        "rec/inherit",
        "assert/with",
        "true/false",
        "0/00",
        "01/002",
        "1e2/3",
        "+/+",
        "++/+",
        "-/-",
        "--/--",
        ".hidden/file",
        "a/.hidden",
        "a/.",
        "a/..",
        "../../foo",
        "a/.../b",
    ] {
        let expected = Expr::RelativePath(path.into());

        let parsed = syntax::parse(path);
        assert_eq!(
            parsed.syntax().unwrap().to_string(),
            path,
            "nxc CST: {path}"
        );
        assert_eq!(parsed.lower().unwrap(), expected, "nxc lowering: {path}");
        assert_eq!(nix::import(path).unwrap(), expected, "Nix lowering: {path}");

        for emitted in [emit::nxc(&expected).unwrap(), nix::emit(&expected).unwrap()] {
            assert_eq!(emitted, format!("({path})"), "emission: {path}");
            assert_eq!(
                parse_nxc(&emitted).unwrap(),
                expected,
                "nxc reparse: {path}"
            );
            assert_eq!(
                nix::import(&emitted).unwrap(),
                expected,
                "Nix reparse: {path}"
            );
        }
    }
}
