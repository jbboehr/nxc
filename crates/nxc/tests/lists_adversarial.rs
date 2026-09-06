use nxc::{emit, nix, parse_nxc};

#[test]
fn omitted_commas_preserve_maximal_expression_boundaries() {
    for (omitted, explicit, native) in [
        ("[1 + 2 3]", "[1 + 2, 3]", "[(1 + 2) 3]"),
        ("[x => x y]", "[x => x, y]", "[(x: x) y]"),
        ("[x => x(y) z]", "[x => x(y), z]", "[(x: x y) z]"),
        ("[s.a or f g]", "[s.a or f, g]", "[(s.a or f) g]"),
        ("[-f(x) y]", "[-f(x), y]", "[(-(f x)) y]"),
        ("[f(x) + g(y) z]", "[f(x) + g(y), z]", "[((f x) + (g y)) z]"),
        ("[f(x)(y) z]", "[f(x)(y), z]", "[((f x) y) z]"),
    ] {
        let expected = nix::import(native)
            .unwrap_or_else(|errors| panic!("native oracle rejected {native:?}: {errors:?}"));
        let without_comma = parse_nxc(omitted)
            .unwrap_or_else(|errors| panic!("failed to parse {omitted:?}: {errors:?}"));
        let with_comma = parse_nxc(explicit)
            .unwrap_or_else(|errors| panic!("failed to parse {explicit:?}: {errors:?}"));

        assert_eq!(without_comma, expected, "wrong boundary in {omitted:?}");
        assert_eq!(with_comma, expected, "comma changed {explicit:?}");
        assert_eq!(
            parse_nxc(&emit::nxc(&expected).unwrap()).unwrap(),
            expected,
            "canonical nxc changed {omitted:?}"
        );
        assert_eq!(
            nix::import(&nix::emit(&expected).unwrap()).unwrap(),
            expected,
            "native emission changed {omitted:?}"
        );
    }
}
