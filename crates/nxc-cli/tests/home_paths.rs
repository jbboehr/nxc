use std::{fs, io::ErrorKind, process::Command};
use tempfile::TempDir;

#[test]
fn conversion_ignores_existing_home_targets_and_preserves_dot_components() {
    let dir = TempDir::new().unwrap();
    let first_home = dir.path().join("first home");
    let second_home = dir.path().join("second home");
    for home in [&first_home, &second_home] {
        fs::create_dir_all(home.join("a")).unwrap();
        fs::write(home.join("target.nix"), "1").unwrap();
    }

    let original = dir.path().join("input.nix");
    let converted = dir.path().join("converted.nxc");
    let emitted = dir.path().join("output.nix");
    fs::write(&original, "~/a/../target.nix").unwrap();

    for (command, input, output, home) in [
        ("from-nix", &original, &converted, &first_home),
        ("to-nix", &converted, &emitted, &second_home),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_nxc"))
            .env("HOME", home)
            .arg(command)
            .arg(input)
            .arg("-o")
            .arg(output)
            .output()
            .unwrap();
        assert!(result.status.success(), "{command}: {result:?}");
    }

    assert_eq!(
        fs::read_to_string(&converted).unwrap(),
        "(~/a/../target.nix)\n"
    );
    assert_eq!(
        fs::read_to_string(&emitted).unwrap(),
        "(~/a/../target.nix)\n"
    );
}

#[test]
fn home_paths_resolve_using_nix_home_after_conversion() {
    let dir = TempDir::new().unwrap();
    let original = dir.path().join("input.nix");
    let output_dir = dir.path().join("output");
    fs::create_dir(&output_dir).unwrap();
    let converted = output_dir.join("converted.nxc");
    let emitted = output_dir.join("output.nix");
    let conversion_home = dir.path().join("conversion-home");
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    let source = "let p = ~/fixture.nix; in [ (import p) (builtins.typeOf p) ]";
    fs::write(&original, source).unwrap();
    for (command, input, output) in [
        ("from-nix", &original, &converted),
        ("to-nix", &converted, &emitted),
    ] {
        let mut process = Command::new(env!("CARGO_BIN_EXE_nxc"));
        if command == "from-nix" {
            process.env("HOME", &conversion_home);
        } else {
            process.env_remove("HOME");
        }
        let result = process
            .current_dir(&output_dir)
            .arg(command)
            .arg(input)
            .arg("-o")
            .arg(output)
            .output()
            .unwrap();
        assert!(result.status.success(), "{command}: {result:?}");
        assert!(result.stdout.is_empty());
    }
    assert!(!conversion_home.exists());
    assert!(!first.exists());
    assert!(!second.exists());
    assert_eq!(fs::read_to_string(&original).unwrap(), source);
    let generated = fs::read_to_string(&emitted).unwrap();
    assert!(generated.contains("~/fixture.nix"));
    assert_eq!(
        nxc::nix::import(&generated).unwrap(),
        nxc::nix::import(source).unwrap()
    );
    for (path, value) in [(&first, "7"), (&second, "11")] {
        fs::create_dir(path).unwrap();
        fs::write(path.join("fixture.nix"), value).unwrap();
    }
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(e) => panic!("cannot start Nix: {e}"),
    }
    for (evaluation_home, expected) in [(&first, "[7,\"path\"]"), (&second, "[11,\"path\"]")] {
        for file in [&original, &emitted] {
            let result = Command::new("nix-instantiate")
                .env("HOME", evaluation_home)
                .current_dir(&output_dir)
                .args(["--store", "dummy://", "--eval", "--strict", "--json"])
                .arg(file)
                .output()
                .unwrap();
            assert!(result.status.success(), "{file:?}: {result:?}");
            assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), expected);
        }
    }
}

#[test]
fn invalid_home_paths_report_locations_and_preserve_existing_output() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("input.nix");
    let output = dir.path().join("output.nix");
    fs::write(&input, "/* α */\n ~/").unwrap();
    fs::write(&output, "keep").unwrap();
    for command in ["from-nix", "to-nix"] {
        let result = Command::new(env!("CARGO_BIN_EXE_nxc"))
            .arg(command)
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert!(
            String::from_utf8(result.stderr)
                .unwrap()
                .contains("input.nix:2:2:")
        );
        assert_eq!(fs::read_to_string(&output).unwrap(), "keep");
    }
}
