use nxc::{nix, parse_nxc};
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
fn native_evaluator_confirms_precedence_currying_and_failure_behavior() {
    if !nix_available() {
        return;
    }
    for (source, expected) in [
        ("1 + 2 * 3", "7"),
        ("(1 + 2) * 3", "9"),
        ("20 - 3 - 2", "15"),
        ("20 - (3 - 2)", "19"),
        ("20 / 3", "6"),
        ("-f(2, 3)", "-23"),
        ("f(1 + 2, x)", "34"),
        ("f(g(2), 3)", "43"),
        ("ignore(1 / 0)", "7"),
        ("true", "5"),
    ] {
        let native = nix::emit(&parse_nxc(source).unwrap()).unwrap();
        let wrapped = format!(
            "let f = a: b: a * 10 + b; g = a: a * 2; x = 4; ignore = a: 7; true = 5; in {native}"
        );
        let parsed = Command::new("nix-instantiate")
            .args(["--store", "dummy://", "--parse", "--expr", &wrapped])
            .output()
            .unwrap();
        assert!(
            parsed.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&parsed.stderr)
        );
        let evaluated = Command::new("nix-instantiate")
            .args([
                "--store", "dummy://", "--eval", "--strict", "--json", "--expr", &wrapped,
            ])
            .output()
            .unwrap();
        assert!(
            evaluated.status.success(),
            "{source}: {}",
            String::from_utf8_lossy(&evaluated.stderr)
        );
        assert_eq!(
            String::from_utf8(evaluated.stdout).unwrap().trim(),
            expected,
            "{source}"
        );
    }
    for source in ["1 / 0", "9223372036854775807 + 1"] {
        let native = nix::emit(&parse_nxc(source).unwrap()).unwrap();
        let evaluated = Command::new("nix-instantiate")
            .args([
                "--store", "dummy://", "--eval", "--strict", "--expr", &native,
            ])
            .output()
            .unwrap();
        assert!(
            !evaluated.status.success(),
            "{source} must retain its evaluation failure"
        );
    }
}

#[test]
fn native_cr_comment_semantics_are_not_silently_discarded() {
    if !nix_available() {
        return;
    }
    let source = "1 # comment\r+ 2";
    let evaluated = Command::new("nix-instantiate")
        .args([
            "--store", "dummy://", "--eval", "--strict", "--json", "--expr", source,
        ])
        .output()
        .unwrap();
    assert!(evaluated.status.success(), "{:?}", evaluated.stderr);
    assert_eq!(String::from_utf8(evaluated.stdout).unwrap().trim(), "3");
    assert!(nix::import(source).is_err());
}

#[test]
fn adapter_rejects_whitespace_that_the_native_parser_rejects() {
    if !nix_available() {
        return;
    }
    for whitespace in ["\u{000b}", "\u{000c}", "\u{00a0}", "\u{2003}"] {
        let source = format!("1{whitespace}+ 2");
        let parsed = Command::new("nix-instantiate")
            .args(["--store", "dummy://", "--parse", "--expr", &source])
            .output()
            .unwrap();
        assert!(!parsed.status.success(), "Nix accepted {source:?}");
        assert!(nix::import(&source).is_err(), "adapter accepted {source:?}");
    }
}
