use nxc::{MAX_DEPTH, emit, ir::Expr, nix, parse_nxc, syntax};
use std::{io::ErrorKind, process::Command};

fn roundtrip(nxc_source: &str, native_source: &str) {
    let native = nix::parse(native_source).unwrap();
    let native_clone = native.clone();
    drop(native);
    let expected = native_clone.lower().unwrap();
    let parsed = syntax::parse(nxc_source);
    assert_eq!(parsed.syntax().unwrap().to_string(), nxc_source);
    let clone = parsed.clone();
    drop(parsed);
    let actual = clone.lower().unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual.clone(), expected);
    let converted = emit::nxc(&expected).unwrap();
    let reparsed = parse_nxc(&converted).unwrap();
    assert_eq!(reparsed, expected);
    let generated = nix::emit(&reparsed).unwrap();
    assert_eq!(nix::import(&generated).unwrap(), expected);
}

fn mixed_assertion_and_concatenation_chain(count: usize) -> (String, String) {
    let mut nxc = "[]".to_owned();
    let mut native = "[]".to_owned();
    for index in 0..count {
        match index % 4 {
            0 => {
                nxc = format!("assert(true, {nxc})");
                native = format!("(assert true; {native})");
            }
            1 => {
                nxc = format!("(if true then {nxc} else [])");
                native = format!("(if true then {native} else [])");
            }
            2 => {
                nxc = format!("({nxc} ++ [])");
                native = format!("({native} ++ [])");
            }
            _ => {
                nxc = format!("([] ++ {nxc})");
                native = format!("([] ++ {native})");
            }
        }
    }
    (nxc, native)
}

fn assert_error_contains<T>(result: Result<T, Vec<nxc::Diagnostic>>, expected: &str) {
    let Err(errors) = result else {
        panic!("expected an error containing {expected:?}");
    };
    assert!(
        errors.iter().any(|error| error.message.contains(expected)),
        "expected an error containing {expected:?}, got {errors:?}"
    );
}

#[test]
fn long_assertion_chains_survive_conversion_on_a_two_mib_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            for count in [192, MAX_DEPTH - 1] {
                let native = format!("{}42", "assert true; ".repeat(count));
                let nxc = format!("{}42{}", "assert(true, ".repeat(count), ")".repeat(count));
                roundtrip(&nxc, &native);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn long_concatenation_chains_survive_conversion_on_a_two_mib_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            for count in [192, MAX_DEPTH - 1] {
                let source = (0..count)
                    .map(|value| format!("[{value}]"))
                    .collect::<Vec<_>>()
                    .join(" ++ ");
                roundtrip(&source, &source);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn mixed_recursive_shapes_survive_the_exact_depth_boundary_on_a_two_mib_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let (nxc, native) = mixed_assertion_and_concatenation_chain(MAX_DEPTH - 1);
            roundtrip(&nxc, &native);
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn deep_failures_clean_up_before_reuse_on_a_two_mib_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let over_depth = (0..MAX_DEPTH).fold("true".to_owned(), |inner, _| {
                format!("if true then {inner} else false")
            });
            let parsed = syntax::parse(&over_depth);
            assert!(parsed.diagnostics().is_empty());
            let parsed_clone = parsed.clone();
            drop(parsed);
            assert_error_contains(parsed_clone.lower(), "semantic depth limit exceeded");
            assert_error_contains(parse_nxc(&over_depth), "semantic depth limit exceeded");

            let parsed = nix::parse(&over_depth).unwrap();
            let parsed_clone = parsed.clone();
            drop(parsed);
            assert_error_contains(parsed_clone.lower(), "semantic depth limit exceeded");
            assert_error_contains(nix::import(&over_depth), "semantic depth limit exceeded");

            let (deep_nxc, deep_native) = mixed_assertion_and_concatenation_chain(MAX_DEPTH - 2);
            let failing_nxc = format!("assert({deep_nxc}, __nxc_bad)");
            let parsed = syntax::parse(&failing_nxc);
            assert!(parsed.diagnostics().is_empty());
            let parsed_clone = parsed.clone();
            drop(parsed);
            assert_error_contains(parsed_clone.lower(), "reserved form is not supported yet");
            assert_error_contains(
                parse_nxc(&failing_nxc),
                "reserved form is not supported yet",
            );

            let failing_native = format!("assert ({deep_native}); __nxc_bad");
            let parsed = nix::parse(&failing_native).unwrap();
            let parsed_clone = parsed.clone();
            drop(parsed);
            assert_error_contains(parsed_clone.lower(), "reserved form is not supported yet");
            assert_error_contains(
                nix::import(&failing_native),
                "reserved form is not supported yet",
            );

            let malformed_nxc = format!(
                "{}@{}",
                "assert(true, ".repeat(MAX_DEPTH - 1),
                ")".repeat(MAX_DEPTH - 1)
            );
            let parsed = syntax::parse(&malformed_nxc);
            assert_eq!(parsed.syntax().unwrap().to_string(), malformed_nxc);
            assert!(!parsed.diagnostics().is_empty());
            let parsed_clone = parsed.clone();
            drop(parsed);
            assert!(parsed_clone.lower().is_err());
            assert!(parse_nxc(&malformed_nxc).is_err());

            let malformed_native = format!("{}@", "assert true; ".repeat(MAX_DEPTH - 1));
            assert!(nix::parse(&malformed_native).is_err());
            assert!(nix::import(&malformed_native).is_err());

            let (supported_nxc, _) = mixed_assertion_and_concatenation_chain(MAX_DEPTH - 1);
            let excessive_ir = Expr::Assert {
                condition: Box::new(Expr::Variable("true".into())),
                body: Box::new(parse_nxc(&supported_nxc).unwrap()),
            };
            for error in [
                emit::nxc(&excessive_ir).unwrap_err(),
                nix::emit(&excessive_ir).unwrap_err(),
            ] {
                assert!(error.message.contains("semantic depth limit exceeded"));
            }
            drop(excessive_ir);

            roundtrip("assert(true, [] ++ [])", "assert true; [] ++ []");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn native_nix_preserves_long_chain_order_and_failure_behavior() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(output) => assert!(output.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    let assertions = "assert true; ".repeat(192);
    let ordered = (0..192)
        .map(|value| format!("[{value}]"))
        .collect::<Vec<_>>()
        .join(" ++ ");
    for (source, expected) in [
        (format!("{assertions}42"), "42".to_owned()),
        (
            format!("builtins.tryEval ({assertions}assert false; builtins.abort \"body forced\")"),
            r#"{"success":false,"value":false}"#.to_owned(),
        ),
        (
            format!("(_: 7) (assert false; {assertions}42)"),
            "7".to_owned(),
        ),
        (
            ordered,
            format!(
                "[{}]",
                (0..192)
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        ),
        (
            format!("builtins.length ({}[])", "[(1 / 0)] ++ ".repeat(192)),
            "192".to_owned(),
        ),
        (
            format!(
                "builtins.tryEval (builtins.head ([1] ++ {}(assert false; [])))",
                "[] ++ ".repeat(192)
            ),
            r#"{"success":false,"value":false}"#.to_owned(),
        ),
    ] {
        let original = nix::import(&source).unwrap();
        let converted = parse_nxc(&emit::nxc(&original).unwrap()).unwrap();
        let generated = nix::emit(&converted).unwrap();
        for value in [&source, &generated] {
            let output = Command::new("nix-instantiate")
                .args([
                    "--store", "dummy://", "--eval", "--strict", "--json", "--expr", value,
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{value}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
        }
    }
}
