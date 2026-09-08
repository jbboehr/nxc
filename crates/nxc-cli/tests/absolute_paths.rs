use std::{fs, io::ErrorKind, process::Command};
use tempfile::TempDir;

#[test]
fn absolute_paths_keep_their_target_when_files_and_working_directory_change() {
    let dir = TempDir::new().unwrap();
    let input_dir = dir.path().join("input");
    let output_dir = dir.path().join("output");
    let eval_dir = dir.path().join("evaluation");
    for path in [&input_dir, &output_dir, &eval_dir] {
        fs::create_dir(path).unwrap();
    }
    let target = dir.path().join("fixture.nix");
    let spelling = format!("{}/absent/.././fixture.nix", dir.path().display());
    let original = input_dir.join("input.nix");
    let converted = output_dir.join("converted.nxc");
    let emitted = output_dir.join("output.nix");
    let source = format!("let p = {spelling}; in [ (import p) (builtins.typeOf p) ]");
    fs::write(&original, &source).unwrap();
    for (command, input, output) in [
        ("from-nix", &original, &converted),
        ("to-nix", &converted, &emitted),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_nxc"))
            .current_dir(&eval_dir)
            .arg(command)
            .arg(input)
            .arg("-o")
            .arg(output)
            .output()
            .unwrap();
        assert!(result.status.success(), "{command}: {result:?}");
        assert!(result.stdout.is_empty());
    }
    assert!(!target.exists());
    assert!(!dir.path().join("absent").exists());
    assert_eq!(fs::read_to_string(&original).unwrap(), source);
    let output = fs::read_to_string(&emitted).unwrap();
    assert!(output.contains(&spelling));
    assert_eq!(
        nxc::nix::import(&output).unwrap(),
        nxc::nix::import(&source).unwrap()
    );
    fs::write(&target, "42").unwrap();
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(e) => panic!("cannot start Nix: {e}"),
    }
    for cwd in [&input_dir, &eval_dir] {
        for file in [&original, &emitted] {
            let result = Command::new("nix-instantiate")
                .current_dir(cwd)
                .args(["--store", "dummy://", "--eval", "--strict", "--json"])
                .arg(file)
                .output()
                .unwrap();
            assert!(result.status.success(), "{file:?}: {result:?}");
            assert_eq!(
                String::from_utf8(result.stdout).unwrap().trim(),
                "[42,\"path\"]"
            );
        }
    }
}

#[test]
fn invalid_absolute_paths_report_locations_and_preserve_existing_output() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("input.nix");
    let output = dir.path().join("output.nix");
    fs::write(&input, "/* α */\n /bad/").unwrap();
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
