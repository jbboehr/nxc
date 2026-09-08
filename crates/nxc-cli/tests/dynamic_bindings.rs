use std::{fs, io::ErrorKind, process::Command};
use tempfile::TempDir;

#[test]
fn binding_conversion_defers_imports_in_keys_and_values() {
    let dir = TempDir::new().unwrap();
    let native = dir.path().join("input.nix");
    let converted = dir.path().join("converted.nxc");
    let output = dir.path().join("output.nix");
    let key = dir.path().join("key.nix");
    let value = dir.path().join("value.nix");
    let source = "{ ${import ./key.nix} = import ./value.nix; }";
    fs::write(&native, source).unwrap();
    for (command, from, to) in [
        ("from-nix", &native, &converted),
        ("to-nix", &converted, &output),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_nxc"))
            .arg(command)
            .arg(from)
            .arg("-o")
            .arg(to)
            .output()
            .unwrap();
        assert!(result.status.success(), "{command}: {result:?}");
        assert!(result.stdout.is_empty());
    }
    assert!(!key.exists());
    assert!(!value.exists());
    assert_eq!(fs::read_to_string(&native).unwrap(), source);
    assert_eq!(
        nxc::nix::import(&fs::read_to_string(&output).unwrap()).unwrap(),
        nxc::nix::import(source).unwrap()
    );
    fs::write(&key, "\"x\"").unwrap();
    fs::write(&value, "42").unwrap();
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    for path in [&native, &output] {
        let result = Command::new("nix-instantiate")
            .args(["--store", "dummy://", "--eval", "--strict", "--json"])
            .arg(path)
            .output()
            .unwrap();
        assert!(result.status.success(), "{path:?}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap().trim(),
            r#"{"x":42}"#
        );
    }
}
