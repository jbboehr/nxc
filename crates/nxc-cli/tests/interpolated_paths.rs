use std::{fs, io::ErrorKind, process::Command};
use tempfile::TempDir;

#[test]
fn path_interpolation_resolves_late_and_keeps_file_and_home_bases() {
    let dir = TempDir::new().unwrap();
    let original = dir.path().join("input.nix");
    let converted = dir.path().join("converted.nxc");
    let emitted = dir.path().join("output.nix");
    let first_home = dir.path().join("first home");
    let second_home = dir.path().join("second home");
    let conversion_home = dir.path().join("nonexistent");
    let absolute_target = dir.path().join("absolute");
    let source = format!(
        r#"let name = "target.nix"; in [ (import ./relative/${{name}}) (import {}/${{name}}) (import ~/home/${{name}}) ]"#,
        absolute_target.display()
    );
    fs::write(&original, &source).unwrap();
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
            .current_dir("/")
            .arg(command)
            .arg(input)
            .arg("-o")
            .arg(output)
            .output()
            .unwrap();
        assert!(result.status.success(), "{command}: {result:?}");
    }
    assert!(!conversion_home.exists());
    assert!(!absolute_target.exists());
    assert_eq!(fs::read_to_string(&original).unwrap(), source);
    let generated = fs::read_to_string(&emitted).unwrap();
    assert!(generated.contains("./relative/${name}"));
    assert!(generated.contains("~/home/${name}"));
    for (path, value) in [
        (dir.path().join("relative"), "7"),
        (absolute_target, "11"),
        (first_home.join("home"), "13"),
        (second_home.join("home"), "17"),
    ] {
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("target.nix"), value).unwrap();
    }
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(e) => panic!("cannot start Nix: {e}"),
    }
    for (home, expected) in [(&first_home, "[7,11,13]"), (&second_home, "[7,11,17]")] {
        for path in [&original, &emitted] {
            let result = Command::new("nix-instantiate")
                .env("HOME", home)
                .current_dir("/")
                .args(["--store", "dummy://", "--eval", "--strict", "--json"])
                .arg(path)
                .output()
                .unwrap();
            assert!(result.status.success(), "{path:?}: {result:?}");
            assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), expected);
            let result = Command::new("nix-instantiate")
                .env("HOME", home)
                .args(["--store", "dummy://", "--eval", "--pure-eval", "--expr"])
                .arg(fs::read_to_string(path).unwrap())
                .output()
                .unwrap();
            assert!(!result.status.success());
            assert!(
                String::from_utf8(result.stderr)
                    .unwrap()
                    .contains("can not be resolved in pure mode")
            );
        }
    }
}

#[test]
fn malformed_path_interpolation_preserves_existing_output_and_locations() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("input.nix");
    let output = dir.path().join("output.nix");
    fs::write(&input, "/* α */\n ./${x}/").unwrap();
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
                .contains("input.nix:2:")
        );
        assert_eq!(fs::read_to_string(&output).unwrap(), "keep");
    }
}
