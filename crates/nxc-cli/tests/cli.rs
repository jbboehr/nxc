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
fn floats_roundtrip_through_files_and_range_errors_preserve_output() {
    let dir = TempDir::new().unwrap();
    let original = dir.path().join("input.nix");
    let converted = dir.path().join("converted.nxc");
    let output = dir.path().join("output.nix");
    let source = "[ 1.0 .5 1.e100 (-0.0) 1.0000000000000002 ]";
    fs::write(&original, source).unwrap();
    for (command, input, output) in [
        ("from-nix", &original, &converted),
        ("to-nix", &converted, &output),
    ] {
        let result = cli(&[
            command.as_ref(),
            input.as_os_str(),
            "-o".as_ref(),
            output.as_os_str(),
        ]);
        assert!(result.status.success(), "{command}: {result:?}");
        assert!(result.stdout.is_empty());
    }
    assert_eq!(fs::read_to_string(&original).unwrap(), source);
    assert_eq!(
        nxc::nix::import(&fs::read_to_string(&output).unwrap()).unwrap(),
        nxc::nix::import(source).unwrap()
    );

    fs::write(&original, "/* α */\n 1.0e309").unwrap();
    fs::write(&output, "keep").unwrap();
    for command in ["from-nix", "to-nix"] {
        let result = cli(&[
            command.as_ref(),
            original.as_os_str(),
            "-o".as_ref(),
            output.as_os_str(),
        ]);
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        let error = String::from_utf8(result.stderr).unwrap();
        assert!(error.contains("input.nix:2:2:"), "{error}");
        assert!(error.contains("float"), "{error}");
        assert_eq!(fs::read_to_string(&output).unwrap(), "keep");
    }
}

#[test]
fn relative_paths_keep_file_based_imports_and_do_not_read_targets_during_conversion() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("input.nix");
    let converted = dir.path().join("converted.nxc");
    let output = dir.path().join("output.nix");
    let value = dir.path().join("value.nix");
    let source = "let value = import ./sub/../value.nix; in assert builtins.isPath ../missing.nix; if true then value else import ../missing.nix";
    fs::write(&input, source).unwrap();
    // Conversion succeeds before even the selected import's target exists.
    for (command, from, to) in [
        ("from-nix", &input, &converted),
        ("to-nix", &converted, &output),
    ] {
        let result = cli(&[
            command.as_ref(),
            from.as_os_str(),
            "-o".as_ref(),
            to.as_os_str(),
        ]);
        assert!(result.status.success(), "{command}: {result:?}");
        assert!(result.stdout.is_empty());
        let generated = fs::read_to_string(to).unwrap();
        assert!(generated.contains("./sub/../value.nix"));
        assert!(generated.contains("../missing.nix"));
    }
    assert_eq!(fs::read_to_string(&input).unwrap(), source);
    assert!(!value.exists());
    fs::create_dir(dir.path().join("sub")).unwrap();
    fs::write(&value, "42").unwrap();

    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    for path in [&input, &output] {
        let result = Command::new("nix-instantiate")
            .args(["--store", "dummy://", "--eval", "--strict", "--json"])
            .arg(path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}: {}",
            path.display(),
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), "42");
    }
    assert_eq!(fs::read_to_string(&value).unwrap(), "42");
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
    fs::write(&input, "./path${x}").unwrap();
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
    // This source is accepted at exactly MAX_TOKENS. Canonical emission wraps
    // each negation, so conversion must report an output-limit error.
    fs::write(&input, format!("---{}", balanced_sum(nxc::MAX_TOKENS / 4))).unwrap();

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
