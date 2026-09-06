use nxc::{emit, nix, parse_nxc, syntax};
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

fn evaluate_strings(strings: &[String]) -> Vec<u8> {
    let expression = format!("let x = \"X\"; in [ {} ]", strings.join(" "));
    let output = Command::new("nix-instantiate")
        .args([
            "--store",
            "dummy://",
            "--eval",
            "--strict",
            "--json",
            "--expr",
            &expression,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn mixed_dollar_and_backslash_runs_before_interpolation_match_native_nix() {
    if !nix_available() {
        return;
    }

    let mut native_sources = Vec::new();
    for length in 0..=6 {
        for bits in 0..(1usize << length) {
            let prefix: String = (0..length)
                .map(|index| if bits & (1 << index) == 0 { '$' } else { '\\' })
                .collect();
            native_sources.push(format!("\"{prefix}${{x}}\""));
        }
    }

    let generated_sources: Vec<_> = native_sources
        .iter()
        .map(|source| {
            let native_ir = nix::import(source)
                .unwrap_or_else(|errors| panic!("native adapter rejected {source:?}: {errors:?}"));
            let nxc_ir = parse_nxc(source)
                .unwrap_or_else(|errors| panic!("nxc adapter rejected {source:?}: {errors:?}"));
            assert_eq!(nxc_ir, native_ir, "adapters disagreed for {source:?}");

            let nxc_source = emit::nxc(&native_ir).unwrap();
            let reparsed = parse_nxc(&nxc_source)
                .unwrap_or_else(|errors| panic!("nxc adapter rejected {nxc_source:?}: {errors:?}"));
            assert_eq!(reparsed, native_ir, "canonical nxc changed {source:?}");
            nix::emit(&nxc_ir).unwrap()
        })
        .collect();

    let native_values = evaluate_strings(&native_sources);
    let generated_values = evaluate_strings(&generated_sources);
    if generated_values != native_values {
        for (native, generated) in native_sources.iter().zip(&generated_sources) {
            assert_eq!(
                evaluate_strings(std::slice::from_ref(generated)),
                evaluate_strings(std::slice::from_ref(native)),
                "conversion changed the value of {native:?} to {generated:?}"
            );
        }
        panic!("batched native evaluation changed without a per-string mismatch");
    }
}

#[test]
fn invalid_interpolation_in_a_binding_does_not_hide_later_bindings() {
    let source = r##"{ a = "${@; "}"}"; b = h(3); }"##;
    let parsed = syntax::parse(source);

    assert!(parsed.lower().is_err());
    let root = parsed.syntax().unwrap();
    assert_eq!(root.to_string(), source);
    assert!(
        root.descendants()
            .any(|node| node.kind() == syntax::SyntaxKind::CallExpr && node.text() == "h(3)"),
        "lost the later binding after recovering from an invalid interpolation: {root:#?}"
    );
}
