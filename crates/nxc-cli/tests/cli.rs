use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

fn cli(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nxc"))
        .args(args)
        .output()
        .unwrap()
}

fn balanced_sum(leaves: usize) -> String {
    if leaves == 1 {
        return "1".into();
    }
    let left = leaves / 2;
    format!("({} + {})", balanced_sum(left), balanced_sum(leaves - left))
}

#[test]
fn conversion_commands_write_stdout_and_named_output() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("input.nxc");
    let native = dir.path().join("converted.nix");
    fs::write(&input, "f(1 + 2, x)").unwrap();

    let checked = cli(&["check".as_ref(), input.as_os_str()]);
    assert!(checked.status.success(), "{:?}", checked);
    assert!(checked.stdout.is_empty());
    assert!(checked.stderr.is_empty());

    let stdout = cli(&["to-nix".as_ref(), input.as_os_str()]);
    assert!(stdout.status.success(), "{:?}", stdout);
    let expected = nxc::parse_nxc("f(1 + 2, x)").unwrap();
    assert_eq!(
        nxc::nix::import(std::str::from_utf8(&stdout.stdout).unwrap()).unwrap(),
        expected
    );

    let saved = cli(&[
        "to-nix".as_ref(),
        input.as_os_str(),
        "-o".as_ref(),
        native.as_os_str(),
    ]);
    assert!(saved.status.success(), "{:?}", saved);
    assert!(saved.stdout.is_empty());
    assert_eq!(fs::read(&native).unwrap(), stdout.stdout);

    let restored = cli(&["from-nix".as_ref(), native.as_os_str()]);
    assert!(restored.status.success(), "{:?}", restored);
    assert_eq!(
        nxc::parse_nxc(std::str::from_utf8(&restored.stdout).unwrap()).unwrap(),
        expected
    );

    let restored_file = dir.path().join("restored.nxc");
    let saved = cli(&[
        "from-nix".as_ref(),
        native.as_os_str(),
        "--output".as_ref(),
        restored_file.as_os_str(),
    ]);
    assert!(saved.status.success(), "{:?}", saved);
    assert!(saved.stdout.is_empty());
    assert_eq!(fs::read(restored_file).unwrap(), restored.stdout);
}

#[test]
fn errors_do_not_produce_output_or_truncate_existing_files() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("broken.nxc");
    let output = dir.path().join("keep.nix");
    fs::write(&input, "# α\nf(1, @)").unwrap();
    fs::write(&output, "keep me").unwrap();
    for command in ["check", "to-nix"] {
        let mut args = vec![command.as_ref(), input.as_os_str()];
        if command == "to-nix" {
            args.extend(["-o".as_ref(), output.as_os_str()]);
        }
        let failed = cli(&args);
        assert!(!failed.status.success());
        assert!(failed.stdout.is_empty());
        let error = String::from_utf8(failed.stderr).unwrap();
        assert!(error.contains("broken.nxc:2:6"), "{error}");
        assert_eq!(fs::read_to_string(&output).unwrap(), "keep me");
    }
    fs::write(&input, "assert true; 1").unwrap();
    let failed = cli(&[
        "from-nix".as_ref(),
        input.as_os_str(),
        "-o".as_ref(),
        output.as_os_str(),
    ]);
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    assert_eq!(fs::read_to_string(output).unwrap(), "keep me");
}

#[test]
fn io_errors_have_nonzero_exit_status() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing.nxc");
    let result = cli(&["check".as_ref(), missing.as_os_str()]);
    assert!(!result.status.success());
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("missing.nxc")
    );

    fs::write(&missing, "1").unwrap();
    let result = cli(&[
        "to-nix".as_ref(),
        missing.as_os_str(),
        "-o".as_ref(),
        dir.path().as_os_str(),
    ]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
}

#[test]
fn emission_limit_errors_do_not_open_existing_destinations() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("at-limit.expr");
    let output = dir.path().join("keep.expr");
    // This source is accepted at exactly 1,024 tokens. Canonical emission wraps
    // each negation, so conversion must report an output-limit error.
    fs::write(&input, format!("---{}", balanced_sum(256))).unwrap();

    for command in ["to-nix", "from-nix"] {
        fs::write(&output, "keep me").unwrap();
        let failed = cli(&[
            command.as_ref(),
            input.as_os_str(),
            "-o".as_ref(),
            output.as_os_str(),
        ]);
        assert!(!failed.status.success(), "{command}: {failed:?}");
        assert!(failed.stdout.is_empty(), "{command}: {failed:?}");
        let error = String::from_utf8(failed.stderr).unwrap();
        assert!(error.contains("limit"), "{command}: {error}");
        assert_eq!(fs::read_to_string(&output).unwrap(), "keep me", "{command}");
    }
}

fn assert_conversion_byte_limit_includes_final_newline(command: &str) {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("input.expr");
    let output = dir.path().join("output.expr");
    let at_limit = "a".repeat(nxc::MAX_SOURCE_BYTES);
    fs::write(&input, &at_limit[..at_limit.len() - 1]).unwrap();

    let accepted = cli(&[command.as_ref(), input.as_os_str()]);
    assert!(
        accepted.status.success(),
        "{command}: {:?}",
        accepted.stderr
    );
    assert_eq!(accepted.stdout.len(), nxc::MAX_SOURCE_BYTES);
    assert_eq!(
        &accepted.stdout[..accepted.stdout.len() - 1],
        &at_limit.as_bytes()[..at_limit.len() - 1]
    );
    assert_eq!(accepted.stdout.last(), Some(&b'\n'));
    let saved = cli(&[
        command.as_ref(),
        input.as_os_str(),
        "-o".as_ref(),
        output.as_os_str(),
    ]);
    assert!(saved.status.success(), "{command}: {:?}", saved.stderr);
    assert!(saved.stdout.is_empty());
    assert_eq!(fs::read(&output).unwrap(), accepted.stdout);

    fs::write(&input, at_limit).unwrap();
    for destination in [None, Some(&output)] {
        fs::write(&output, "keep me").unwrap();
        let mut args = vec![command.as_ref(), input.as_os_str()];
        if let Some(destination) = destination {
            args.extend(["-o".as_ref(), destination.as_os_str()]);
        }
        let failed = cli(&args);
        assert!(
            !failed.status.success(),
            "{command} accepted an oversized final payload"
        );
        assert!(failed.stdout.is_empty());
        let error = String::from_utf8(failed.stderr).unwrap();
        assert!(error.contains("limit"), "{command}: {error}");
        assert_eq!(fs::read_to_string(&output).unwrap(), "keep me", "{command}");
    }
}

#[test]
fn to_nix_byte_limit_includes_final_newline() {
    assert_conversion_byte_limit_includes_final_newline("to-nix");
}

#[test]
fn from_nix_byte_limit_includes_final_newline() {
    assert_conversion_byte_limit_includes_final_newline("from-nix");
}

#[test]
fn rejected_native_cr_comments_do_not_produce_output_or_overwrite_files() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("bare-cr.nix");
    let output = dir.path().join("keep.nxc");
    fs::write(&input, "1 # comment\r+ 2").unwrap();
    fs::write(&output, "keep me").unwrap();
    for destination in [None, Some(&output)] {
        let mut args = vec!["from-nix".as_ref(), input.as_os_str()];
        if let Some(destination) = destination {
            args.extend(["-o".as_ref(), destination.as_os_str()]);
        }
        let failed = cli(&args);
        assert!(!failed.status.success());
        assert!(failed.stdout.is_empty());
        assert_eq!(fs::read_to_string(&output).unwrap(), "keep me");
    }
}

#[test]
fn over_budget_lexical_errors_produce_one_cli_diagnostic() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("invalid.nxc");
    fs::write(&input, "@\n".repeat(nxc::MAX_TOKENS + 1)).unwrap();
    for command in ["check", "to-nix"] {
        let failed = cli(&[command.as_ref(), input.as_os_str()]);
        assert!(!failed.status.success());
        assert!(failed.stdout.is_empty());
        let error = String::from_utf8(failed.stderr).unwrap();
        assert_eq!(error.lines().count(), 1);
        assert!(error.contains("limit"), "{error}");
    }
}
