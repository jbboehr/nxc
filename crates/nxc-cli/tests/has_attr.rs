use std::{fs, io::ErrorKind, process::Command};
use tempfile::TempDir;

#[test]
fn existence_conversion_defers_imports_and_preserves_lazy_attribute_values() {
    let dir = TempDir::new().unwrap();
    let original = dir.path().join("input.nix");
    let converted = dir.path().join("converted.nxc");
    let emitted = dir.path().join("output.nix");
    let value = dir.path().join("value.nix");
    let key = dir.path().join("key.nix");
    let source = "(import ./value.nix) ? ${import ./key.nix}";
    fs::write(&original, source).unwrap();
    for (command, input, output) in [
        ("from-nix", &original, &converted),
        ("to-nix", &converted, &emitted),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_nxc"))
            .arg(command)
            .arg(input)
            .arg("-o")
            .arg(output)
            .output()
            .unwrap();
        assert!(result.status.success(), "{command}: {result:?}");
        assert!(result.stdout.is_empty());
    }
    assert!(!value.exists());
    assert!(!key.exists());
    assert_eq!(fs::read_to_string(&original).unwrap(), source);
    assert_eq!(
        nxc::nix::import(&fs::read_to_string(&emitted).unwrap()).unwrap(),
        nxc::nix::import(source).unwrap()
    );
    fs::write(&value, "{ present = abort \"unforced attribute value\"; }").unwrap();
    fs::write(&key, "\"present\"").unwrap();
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    for path in [&original, &emitted] {
        let result = Command::new("nix-instantiate")
            .args(["--store", "dummy://", "--eval", "--strict", "--json"])
            .arg(path)
            .output()
            .unwrap();
        assert!(result.status.success(), "{path:?}: {result:?}");
        assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), "true");
    }
}
