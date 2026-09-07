use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

fn cli(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nxc"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn conversion_does_not_evaluate_dynamic_keys_or_defaults() {
    let dir = TempDir::new().unwrap();
    let native = dir.path().join("input.nix");
    let converted = dir.path().join("converted.nxc");
    let restored = dir.path().join("restored.nix");
    let missing_key = dir.path().join("missing-key.nix");
    let missing_default = dir.path().join("missing-default.nix");
    let source = "let s = {}; in s.${import ./missing-key.nix} or (import ./missing-default.nix)";
    fs::write(&native, source).unwrap();

    for (command, from, to) in [
        ("from-nix", &native, &converted),
        ("to-nix", &converted, &restored),
    ] {
        let result = cli(&[
            command.as_ref(),
            from.as_os_str(),
            "-o".as_ref(),
            to.as_os_str(),
        ]);
        assert!(
            result.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stdout.is_empty());
    }

    assert!(!missing_key.exists());
    assert!(!missing_default.exists());
    assert_eq!(
        nxc::nix::import(&fs::read_to_string(&restored).unwrap()).unwrap(),
        nxc::nix::import(source).unwrap()
    );
}
