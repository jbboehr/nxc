use nxc::{MAX_SOURCE_BYTES, emit, nix};

#[test]
fn implication_normalization_counts_generated_source_growth_at_the_exact_limit() {
    let at_limit = format!("{} -> b", "a".repeat(MAX_SOURCE_BYTES - 10));
    let ir = nix::import(&at_limit).unwrap();
    let nxc = emit::nxc(&ir).unwrap();
    let native = nix::emit(&ir).unwrap();
    assert_eq!(nxc.len(), MAX_SOURCE_BYTES);
    assert_eq!(native.len(), MAX_SOURCE_BYTES);
    assert_eq!(nix::import(&native).unwrap(), ir);

    let over_limit = format!("{} -> b", "a".repeat(MAX_SOURCE_BYTES - 9));
    assert!(over_limit.len() < MAX_SOURCE_BYTES);
    let ir = nix::import(&over_limit).unwrap();
    assert!(emit::nxc(&ir).is_err());
    assert!(nix::emit(&ir).is_err());
}
