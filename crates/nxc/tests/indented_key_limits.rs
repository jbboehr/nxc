use nxc::{emit, nix, parse_nxc};
use std::{io::ErrorKind, process::Command};

fn nix_available() -> bool {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(output) => {
            assert!(output.status.success());
            true
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            false
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
}

#[test]
fn indented_key_lexical_boundaries_match_native_nix() {
    let native_available = nix_available();

    let cases = [
        (format!("''{}{}''", "'''", "''$"), false),
        (format!("''{}{}''", "''$", "'''"), false),
        (r#"''${(''a'''b'')}''"#.to_owned(), false),
        (format!("''\n    {}''", r"''\x"), true),
        ("''a$$b''".to_owned(), true),
    ];

    for (key, expected_static) in cases {
        let native_let = format!("let ${{{key}}} = 1; in 0");
        let nxc_let = format!("let {{ ${{{key}}} = 1; yield 0; }}");
        assert_eq!(nix::import(&native_let).is_ok(), expected_static, "{key}");
        assert_eq!(parse_nxc(&nxc_let).is_ok(), expected_static, "{key}");

        let native_set = format!("{{ ${{{key}}} = 1; }}");
        let ir = nix::import(&native_set).unwrap_or_else(|error| panic!("{key}: {error:?}"));
        let canonical_nxc = emit::nxc(&ir).unwrap();
        assert_eq!(parse_nxc(&canonical_nxc).unwrap(), ir, "{canonical_nxc}");
        let canonical_nix = nix::emit(&ir).unwrap();
        assert_eq!(nix::import(&canonical_nix).unwrap(), ir, "{canonical_nix}");

        if native_available {
            let value = Command::new("nix-instantiate")
                .args([
                    "--store", "dummy://", "--eval", "--strict", "--json", "--expr", &key,
                ])
                .output()
                .unwrap_or_else(|error| panic!("cannot evaluate {key}: {error}"));
            assert!(
                value.status.success(),
                "invalid key fixture {key}: {}",
                String::from_utf8_lossy(&value.stderr)
            );

            let native_result = Command::new("nix-instantiate")
                .args([
                    "--store",
                    "dummy://",
                    "--eval",
                    "--strict",
                    "--json",
                    "--expr",
                    &native_let,
                ])
                .output()
                .unwrap_or_else(|error| panic!("cannot evaluate {native_let}: {error}"));
            assert_eq!(
                native_result.status.success(),
                expected_static,
                "native Nix classification changed for {key}: {}",
                String::from_utf8_lossy(&native_result.stderr)
            );
        }
    }
}
