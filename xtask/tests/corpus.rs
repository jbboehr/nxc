use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn corpus(root: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("corpus")
        .arg(root)
        .args(extra)
        .output()
        .unwrap()
}

fn count(output: &str, label: &str) -> usize {
    output
        .lines()
        .find_map(|line| {
            line.strip_prefix(&format!("{label}: "))
                .map(|value| value.parse().unwrap())
        })
        .unwrap_or_else(|| panic!("missing {label:?} in {output}"))
}

fn fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join("nested")).unwrap();
    fs::write(dir.path().join("00-valid.nix"), "f (1 + 2) x").unwrap();
    fs::write(dir.path().join("01-unsupported.nix"), "{ a = 1; }").unwrap();
    fs::write(dir.path().join("nested/02-invalid.nix"), "f (").unwrap();
    fs::write(dir.path().join("nested/03-valid.nix"), "1 + 2 * 3").unwrap();
    fs::write(dir.path().join("ignored.txt"), "this is not Nix").unwrap();
    dir
}

#[test]
fn corpus_reports_every_stage_and_continues_after_failures() {
    let dir = fixture();
    let output = corpus(dir.path(), &[]);
    assert_eq!(output.status.code(), Some(1));
    let summary = String::from_utf8(output.stdout).unwrap();
    assert_eq!(count(&summary, "files discovered"), 4);
    assert_eq!(count(&summary, "files selected"), 4);
    assert_eq!(count(&summary, "files processed"), 4);
    for (stage, successes, failures) in [
        ("read", 4, 0),
        ("native parse", 3, 1),
        ("native lower", 2, 1),
        ("nxc emit", 2, 0),
        ("nxc parse", 2, 0),
        ("nxc lower", 2, 0),
        ("nxc equality", 2, 0),
        ("nix emit", 2, 0),
        ("generated native parse", 2, 0),
        ("generated native lower", 2, 0),
        ("native equality", 2, 0),
    ] {
        assert_eq!(count(&summary, &format!("{stage} successes")), successes);
        assert_eq!(count(&summary, &format!("{stage} failures")), failures);
    }
    let failures = String::from_utf8(output.stderr).unwrap();
    assert!(
        failures.contains("01-unsupported.nix [native lower]"),
        "{failures}"
    );
    assert!(
        failures.contains("nested/02-invalid.nix [native parse]"),
        "{failures}"
    );
    assert!(failures.contains("bytes "), "{failures}");
    assert_eq!(failures.lines().count(), 2);
}

#[test]
fn filter_selects_relative_paths_and_successful_roundtrips_exit_zero() {
    let dir = fixture();
    let output = corpus(dir.path(), &["--filter", "03-valid"]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let summary = String::from_utf8(output.stdout).unwrap();
    assert_eq!(count(&summary, "files discovered"), 4);
    assert_eq!(count(&summary, "files selected"), 1);
    assert_eq!(count(&summary, "files processed"), 1);
    assert_eq!(count(&summary, "native equality successes"), 1);
}

#[test]
fn fail_fast_stops_after_first_failed_file_in_sorted_order() {
    let dir = fixture();
    let output = corpus(dir.path(), &["--fail-fast"]);
    assert_eq!(output.status.code(), Some(1));
    let summary = String::from_utf8(output.stdout).unwrap();
    assert_eq!(count(&summary, "files discovered"), 4);
    assert_eq!(count(&summary, "files selected"), 4);
    assert_eq!(count(&summary, "files processed"), 2);
    assert_eq!(count(&summary, "native equality successes"), 1);
    let failures = String::from_utf8(output.stderr).unwrap();
    assert!(failures.contains("01-unsupported.nix [native lower]"));
    assert!(!failures.contains("02-invalid.nix"));
}

#[test]
fn empty_selection_and_missing_roots_are_explicit_failures() {
    let dir = fixture();
    let empty = corpus(dir.path(), &["--filter", "does-not-exist"]);
    assert_eq!(empty.status.code(), Some(1));
    assert!(
        String::from_utf8(empty.stderr)
            .unwrap()
            .contains("no .nix files selected")
    );
    let missing = corpus(&dir.path().join("missing"), &[]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(
        String::from_utf8(missing.stderr)
            .unwrap()
            .contains("[discovery]")
    );
}

#[test]
fn invalid_utf8_and_oversized_files_fail_at_read_without_stopping_the_run() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("0-encoding.nix"), [0xff]).unwrap();
    let exact_limit = format!("1{}", " ".repeat(nxc::MAX_SOURCE_BYTES - 1));
    fs::write(dir.path().join("1-exact-limit.nix"), exact_limit).unwrap();
    fs::write(
        dir.path().join("2-over-limit.nix"),
        vec![b' '; nxc::MAX_SOURCE_BYTES + 1],
    )
    .unwrap();
    fs::write(dir.path().join("3-valid.nix"), "1").unwrap();
    let output = corpus(dir.path(), &[]);
    assert_eq!(output.status.code(), Some(1));
    let summary = String::from_utf8(output.stdout).unwrap();
    assert_eq!(count(&summary, "read successes"), 2);
    assert_eq!(count(&summary, "read failures"), 2);
    assert_eq!(count(&summary, "native equality successes"), 2);
    let failures = String::from_utf8(output.stderr).unwrap();
    assert!(failures.contains("0-encoding.nix [read]"));
    assert!(failures.contains("2-over-limit.nix [read]"));
}

#[test]
fn emission_limit_failure_is_distinct_from_parse_or_lowering_failure() {
    fn sum(leaves: usize) -> String {
        if leaves == 1 {
            return "1".into();
        }
        let half = leaves / 2;
        format!("({} + {})", sum(half), sum(leaves - half))
    }
    let dir = TempDir::new().unwrap();
    // Exactly 1,024 input tokens; canonical parentheses expand past the limit.
    fs::write(dir.path().join("large.nix"), format!("---{}", sum(256))).unwrap();
    let output = corpus(dir.path(), &[]);
    assert_eq!(output.status.code(), Some(1));
    let summary = String::from_utf8(output.stdout).unwrap();
    assert_eq!(count(&summary, "native parse successes"), 1);
    assert_eq!(count(&summary, "native lower successes"), 1);
    assert_eq!(count(&summary, "nxc emit failures"), 1);
    assert_eq!(count(&summary, "nxc parse failures"), 0);
    assert_eq!(count(&summary, "nxc parse successes"), 0);
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("large.nix [nxc emit]")
    );
}

#[test]
fn discovery_ignores_git_metadata_and_symbolic_links() {
    let dir = TempDir::new().unwrap();
    fs::create_dir(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join(".git/hidden.nix"), "f (").unwrap();
    fs::write(dir.path().join("valid.nix"), "1").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.path(), dir.path().join("loop")).unwrap();
        std::os::unix::fs::symlink("missing", dir.path().join("dangling.nix")).unwrap();
        std::os::unix::fs::symlink("valid.nix", dir.path().join("alias.nix")).unwrap();
    }
    let output = corpus(dir.path(), &[]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let summary = String::from_utf8(output.stdout).unwrap();
    assert_eq!(count(&summary, "files discovered"), 1);
    assert_eq!(count(&summary, "native equality successes"), 1);
}

#[cfg(unix)]
#[test]
fn symlinked_root_uses_case_sensitive_literal_filters_on_relative_paths() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("target-Case[1]");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("Case[1].nix"), "1").unwrap();
    fs::write(target.join("Case1.nix"), "f (").unwrap();
    fs::write(target.join("case[1].nix"), "f (").unwrap();
    let root = dir.path().join("corpus-link");
    std::os::unix::fs::symlink(&target, &root).unwrap();

    let selected = corpus(&root, &["--filter", "Case[1]"]);
    assert!(selected.status.success(), "{selected:?}");
    assert!(selected.stderr.is_empty());
    let summary = String::from_utf8(selected.stdout).unwrap();
    assert_eq!(count(&summary, "files discovered"), 3);
    assert_eq!(count(&summary, "files selected"), 1);
    assert_eq!(count(&summary, "native equality successes"), 1);

    let root_name = corpus(&root, &["--filter", "corpus-link"]);
    assert_eq!(root_name.status.code(), Some(1));
    let summary = String::from_utf8(root_name.stdout).unwrap();
    assert_eq!(count(&summary, "files discovered"), 3);
    assert_eq!(count(&summary, "files selected"), 0);
}

#[cfg(unix)]
#[test]
fn corpus_is_read_only_and_does_not_invoke_nix_per_file() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let source_path = dir.path().join("input.nix");
    let source = b"# keep this exact spelling\nf (1 + 2) x\n";
    fs::write(&source_path, source).unwrap();

    let trap_dir = TempDir::new().unwrap();
    let marker = trap_dir.path().join("called");
    for executable in ["nix", "nix-instantiate"] {
        let path = trap_dir.path().join(executable);
        fs::write(
            &path,
            "#!/bin/sh\nprintf called >> \"$NXC_NIX_MARKER\"\nexit 99\n",
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("corpus")
        .arg(dir.path())
        .env("PATH", trap_dir.path())
        .env("NXC_NIX_MARKER", &marker)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(fs::read(&source_path).unwrap(), source);
    assert!(!marker.exists(), "corpus runner invoked a Nix subprocess");
}

#[cfg(target_os = "linux")]
#[test]
fn diagnostic_write_failure_exits_without_panicking() {
    let dir = TempDir::new().unwrap();
    let source_path = dir.path().join("unsupported.nix");
    let source = b"{ a = 1; }";
    fs::write(&source_path, source).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("corpus")
        .arg(dir.path())
        .stderr(
            fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap(),
        )
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(fs::read(&source_path).unwrap(), source);
}

#[cfg(target_os = "linux")]
#[test]
fn report_write_failure_exits_without_panicking_when_stderr_also_fails() {
    let dir = TempDir::new().unwrap();
    let source_path = dir.path().join("valid.nix");
    let source = b"1";
    fs::write(&source_path, source).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("corpus")
        .arg(dir.path())
        .stdout(
            fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap(),
        )
        .stderr(
            fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap(),
        )
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(fs::read(&source_path).unwrap(), source);
}
