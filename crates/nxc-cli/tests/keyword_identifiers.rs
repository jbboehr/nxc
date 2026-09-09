use std::{fs, io::ErrorKind, process::Command};
use tempfile::TempDir;

#[test]
fn keyword_identifiers_roundtrip_through_files_with_native_lookup() {
    let dir = TempDir::new().unwrap();
    let original = dir.path().join("input.nix");
    let converted = dir.path().join("converted.nxc");
    let emitted = dir.path().join("output.nix");
    let source = r#"let
        fn = { fn, yield ? fn, ... }: yield;
        yield = rec { fn = 7; yield = fn; __nxc_ident_fn = 99; };
    in [ (fn yield) (builtins.functionArgs fn)
         (with yield; builtins.functionArgs fn) (let inherit (yield) fn; in fn)
         yield.__nxc_ident_fn ]"#;
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
    let checked = Command::new(env!("CARGO_BIN_EXE_nxc"))
        .arg("check")
        .arg(&converted)
        .output()
        .unwrap();
    assert!(checked.status.success(), "{checked:?}");
    assert_eq!(fs::read_to_string(&original).unwrap(), source);
    assert_eq!(
        nxc::nix::import(&fs::read_to_string(&emitted).unwrap()).unwrap(),
        nxc::nix::import(source).unwrap()
    );

    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(e) => panic!("cannot start Nix: {e}"),
    }
    // Lexical fn takes priority over the with scope's integer-valued fn.
    for file in [&original, &emitted] {
        let result = Command::new("nix-instantiate")
            .args(["--store", "dummy://", "--eval", "--strict", "--json"])
            .arg(file)
            .output()
            .unwrap();
        assert!(result.status.success(), "{file:?}: {result:?}");
        assert_eq!(
            String::from_utf8(result.stdout).unwrap().trim(),
            r#"[7,{"fn":false,"yield":true},{"fn":false,"yield":true},7,99]"#
        );
    }
}

#[test]
fn unknown_alias_reports_source_location_and_preserves_output() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("input.nxc");
    let output = dir.path().join("output.nix");
    fs::write(&input, "/* α */\n __nxc_ident_unknown").unwrap();
    fs::write(&output, "keep").unwrap();
    for command in ["to-nix", "from-nix"] {
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
                .contains("input.nxc:2:2:")
        );
        assert_eq!(fs::read_to_string(&output).unwrap(), "keep");
    }
}
