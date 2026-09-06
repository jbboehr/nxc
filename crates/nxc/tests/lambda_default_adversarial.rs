use nxc::nix;
use std::{io::ErrorKind, process::Command};

fn nix_parse(source: &str) -> Option<bool> {
    match Command::new("nix-instantiate")
        .args(["--store", "dummy://", "--parse", "--expr", source])
        .output()
    {
        Ok(output) => Some(output.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => panic!("cannot start Nix for {source}: {error}"),
    }
}

#[test]
fn selection_lambda_defaults_match_native_nix_across_contexts() {
    for (source, accepted) in [
        ("{}.a or (x: {}.b or y: y)", false),
        ("{ f ? {}.a or x: x }: f", false),
        ("{ value = {}.a or x: x; }", false),
        ("x: {}.a or y: y", false),
        ("{}.a or { value = {}.b or x: x; }", false),
        ("({}.a or {}).b or { x ? 1 }: x", false),
        ("{}.a or ({ x ? 1, ... }@args: x)", true),
        ("{}.a or ((x: x))", true),
        ("{}.a or ({ f ? x: x }: f)", true),
        ("{ f ? { x, ... }: x }: f", true),
        ("{ value = { x ? 1, ... }@args: x; }", true),
        ("({}.a or (x: x)).name or ({ y }: y)", true),
        ("{}.a or (x /* parameter */ : x)", true),
        ("{}.a or x /* separator */ : x", false),
    ] {
        if let Some(native_accepted) = nix_parse(source) {
            assert_eq!(native_accepted, accepted, "native Nix changed for {source}");
        }
        assert_eq!(
            nix::import(source).is_ok(),
            accepted,
            "native adapter disagreed for {source}"
        );
    }
}
