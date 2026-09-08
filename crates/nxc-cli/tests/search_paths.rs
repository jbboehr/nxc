use std::{fs, io::ErrorKind, process::Command};
use tempfile::TempDir;

#[test]
fn search_paths_resolve_only_during_evaluation_in_the_current_search_environment() {
    let dir = TempDir::new().unwrap();
    let original = dir.path().join("input.nix");
    let converted = dir.path().join("converted.nxc");
    let emitted = dir.path().join("output.nix");
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    let source =
        "let p = <fixture>; in [ (import p) (import <fixture/child.nix>) (builtins.typeOf p) ]";
    fs::write(&original, source).unwrap();
    for (command, input, output) in [
        ("from-nix", &original, &converted),
        ("to-nix", &converted, &emitted),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_nxc"))
            .env("NIX_PATH", format!("fixture={}", first.display()))
            .arg(command)
            .arg(input)
            .arg("-o")
            .arg(output)
            .output()
            .unwrap();
        assert!(result.status.success(), "{command}: {result:?}");
        assert!(result.stdout.is_empty());
    }
    assert!(!first.exists());
    assert!(!second.exists());
    assert_eq!(fs::read_to_string(&original).unwrap(), source);
    assert_eq!(
        nxc::nix::import(&fs::read_to_string(&emitted).unwrap()).unwrap(),
        nxc::nix::import(source).unwrap()
    );
    for (path, value) in [(&first, "7"), (&second, "11")] {
        fs::create_dir(path).unwrap();
        fs::write(path.join("default.nix"), value).unwrap();
        fs::write(path.join("child.nix"), value).unwrap();
    }
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(e) => panic!("cannot start Nix: {e}"),
    }
    for (leading, trailing, expected) in [
        (&first, &second, "[7,7,\"path\"]"),
        (&second, &first, "[11,11,\"path\"]"),
    ] {
        for file in [&original, &emitted] {
            let result = Command::new("nix-instantiate")
                .env(
                    "NIX_PATH",
                    format!(
                        "fixture={}:fixture={}",
                        leading.display(),
                        trailing.display()
                    ),
                )
                .args(["--store", "dummy://", "--eval", "--strict", "--json"])
                .arg(file)
                .output()
                .unwrap();
            assert!(result.status.success(), "{file:?}: {result:?}");
            assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), expected);
        }
    }
    // Invalid path spelling keeps an existing output intact in both directions.
    fs::write(&original, "/* α */\n <bad/>").unwrap();
    fs::write(&emitted, "keep").unwrap();
    for command in ["from-nix", "to-nix"] {
        let result = Command::new(env!("CARGO_BIN_EXE_nxc"))
            .arg(command)
            .arg(&original)
            .arg("-o")
            .arg(&emitted)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert!(
            String::from_utf8(result.stderr)
                .unwrap()
                .contains("input.nix:2:2:")
        );
        assert_eq!(fs::read_to_string(&emitted).unwrap(), "keep");
    }
}
