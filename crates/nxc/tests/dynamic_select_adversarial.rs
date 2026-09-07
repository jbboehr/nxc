use nxc::nix;

#[test]
fn native_dynamic_selection_rejects_non_simple_defaults() {
    for selection in ["s.a", "s.${key}"] {
        for default in ["-1", "!true", "assert true; 1"] {
            let source = format!("{selection} or {default}");
            assert!(
                nix::import(&source).is_err(),
                "accepted invalid native Nix: {source}"
            );
        }
    }
}
