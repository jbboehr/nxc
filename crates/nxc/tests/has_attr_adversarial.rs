use nxc::{emit, nix, parse_nxc};
use std::{io::ErrorKind, process::Command};

fn nix_available() -> bool {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(output) => {
            assert!(output.status.success());
            true
        }
        Err(error) if error.kind() == ErrorKind::NotFound => false,
        Err(error) => panic!("cannot start Nix: {error}"),
    }
}

fn evaluate(source: &str) -> std::process::Output {
    Command::new("nix-instantiate")
        .args([
            "--store", "dummy://", "--eval", "--strict", "--json", "--expr", source,
        ])
        .output()
        .unwrap()
}

#[test]
fn dynamic_existence_keys_keep_forcing_order_and_short_circuit() {
    if !nix_available() {
        return;
    }
    for (source, expected, failure) in [
        (
            r#"let first = abort "first-key"; second = abort "second-key"; in { a = {}; } ? ${first}.${second}"#,
            None,
            Some("first-key"),
        ),
        (
            r#"let first = "missing"; second = abort "must-stay-lazy"; in {} ? ${first}.${second}"#,
            Some("false"),
            None,
        ),
    ] {
        let imported = nix::import(source).unwrap();
        let nxc_source = emit::nxc(&imported).unwrap();
        let reparsed = parse_nxc(&nxc_source).unwrap();
        assert_eq!(reparsed, imported);

        for generated in [source.to_owned(), nix::emit(&reparsed).unwrap()] {
            let result = evaluate(&generated);
            let stderr = String::from_utf8_lossy(&result.stderr);
            if let Some(expected) = expected {
                assert!(result.status.success(), "{generated}: {stderr}");
                assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), expected);
            } else {
                assert!(!result.status.success(), "{generated} must fail");
                let abort = format!(
                    "evaluation aborted with the following error message: '{}'",
                    failure.unwrap()
                );
                assert!(stderr.contains(&abort), "{generated}: {stderr}");
            }
        }
    }
}
