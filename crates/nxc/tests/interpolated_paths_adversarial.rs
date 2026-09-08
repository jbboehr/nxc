use nxc::{
    emit,
    ir::{Expr, StringPart},
    nix, parse_nxc,
};
use proptest::prelude::*;

fn embedded_expression() -> impl Strategy<Value = Expr> {
    prop_oneof![
        prop::sample::select(vec!["x", "y", "path"]).prop_map(|name| Expr::Variable(name.into())),
        (0_u64..=9).prop_map(Expr::Integer),
        prop::sample::select(vec!["", "segment", "x/y", "with space"]).prop_map(|text| {
            if text.is_empty() {
                Expr::String(vec![])
            } else {
                Expr::String(vec![StringPart::Literal(text.into())])
            }
        }),
    ]
}

fn interpolated_path() -> impl Strategy<Value = Expr> {
    (
        prop::sample::select(vec![
            "./",
            "./prefix",
            "../a/",
            "relative/path",
            "/",
            "/root/",
            "~/",
            "~/home/",
        ]),
        embedded_expression(),
        embedded_expression(),
        prop::collection::vec(
            (
                prop::sample::select(vec!["tail", ".nix", "-suffix", "+suffix", "/component"]),
                embedded_expression(),
            ),
            0..=2,
        ),
        prop::option::of(prop::sample::select(vec![
            "tail",
            ".nix",
            "-suffix",
            "+suffix",
            "/component",
        ])),
    )
        .prop_map(|(prefix, first, second, interpolations, suffix)| {
            let mut parts = vec![
                StringPart::Literal(prefix.into()),
                StringPart::Interpolation(first),
                StringPart::Interpolation(second),
            ];
            for (literal, value) in interpolations {
                parts.push(StringPart::Literal(literal.into()));
                parts.push(StringPart::Interpolation(value));
            }
            if let Some(suffix) = suffix {
                parts.push(StringPart::Literal(suffix.into()));
            }
            Expr::InterpolatedPath(parts)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn generated_fragments_survive_both_public_conversion_cycles(path in interpolated_path()) {
        let nxc_source = emit::nxc(&path).unwrap();
        let from_nxc = parse_nxc(&nxc_source).unwrap();
        prop_assert_eq!(&from_nxc, &path, "nxc source: {}", nxc_source);
        prop_assert_eq!(emit::nxc(&from_nxc).unwrap(), nxc_source);

        let nix_source = nix::emit(&path).unwrap();
        let from_nix = nix::import(&nix_source).unwrap();
        prop_assert_eq!(&from_nix, &path, "Nix source: {}", nix_source);
        prop_assert_eq!(nix::emit(&from_nix).unwrap(), nix_source);
    }
}
