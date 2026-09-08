use nxc::{ir::Float, nix, parse_nxc};

#[test]
fn exact_subnormal_accepts_equivalent_scientific_spelling() {
    let expected = Float::new(f64::from_bits(f64::MIN_POSITIVE.to_bits() - 1)).unwrap();
    let fixed = expected.to_string();
    let fraction = fixed.strip_prefix("0.").unwrap();
    let first_significant = fraction.find(|byte| byte != '0').unwrap();
    let significant = &fraction[first_significant..];
    let scientific = format!(
        "{}.{}e-{}",
        &significant[..1],
        &significant[1..],
        first_significant + 1
    );

    let expected_expr = parse_nxc(&fixed).unwrap();
    assert_eq!(scientific.parse::<Float>().unwrap(), expected);
    assert_eq!(parse_nxc(&scientific).unwrap(), expected_expr);
    assert_eq!(nix::import(&scientific).unwrap(), expected_expr);
}
