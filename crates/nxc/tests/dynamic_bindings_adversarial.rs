use nxc::{emit, nix, parse_nxc, syntax};
use std::{io::ErrorKind, process::Command};

fn roundtrip(nxc_source: &str, native_source: &str) -> String {
    let parsed = syntax::parse(nxc_source);
    assert_eq!(parsed.syntax().unwrap().to_string(), nxc_source);
    let ir = parsed
        .lower()
        .unwrap_or_else(|errors| panic!("{nxc_source}: {errors:?}"));
    assert_eq!(
        nix::import(native_source).unwrap(),
        ir,
        "dialects disagreed"
    );

    let canonical_nxc = emit::nxc(&ir).unwrap();
    let reparsed = parse_nxc(&canonical_nxc).unwrap();
    assert_eq!(reparsed, ir, "{canonical_nxc}");

    let emitted_native = nix::emit(&reparsed).unwrap();
    assert_eq!(
        nix::import(&emitted_native).unwrap(),
        ir,
        "{emitted_native}"
    );
    emitted_native
}

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

fn evaluate(source: &str) -> String {
    let output = Command::new("nix-instantiate")
        .args([
            "--store", "dummy://", "--eval", "--strict", "--json", "--expr", source,
        ])
        .output()
        .unwrap_or_else(|error| panic!("cannot evaluate {source}: {error}"));
    assert!(
        output.status.success(),
        "{source}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn computed_suffix_merge_keeps_order_dependent_key_and_value_scope() {
    let cases = [
        (
            r#"let { key = "outer"; value = 10; yield { a = rec { key = "inner"; value = 2; }; a.${key} = value; }.a; }"#,
            r#"let key = "outer"; value = 10; in { a = rec { key = "inner"; value = 2; }; a.${key} = value; }.a"#,
            r#"{"inner":2,"key":"inner","value":2}"#,
        ),
        (
            r#"let { key = "outer"; value = 10; yield { a.${key} = value; a = rec { key = "inner"; value = 2; }; }.a; }"#,
            r#"let key = "outer"; value = 10; in { a.${key} = value; a = rec { key = "inner"; value = 2; }; }.a"#,
            r#"{"key":"inner","outer":10,"value":2}"#,
        ),
    ];

    let converted: Vec<_> = cases
        .iter()
        .map(|(nxc_source, native_source, _)| roundtrip(nxc_source, native_source))
        .collect();
    if !nix_available() {
        return;
    }
    for ((_, native_source, expected), emitted_native) in cases.iter().zip(converted) {
        assert_eq!(evaluate(native_source), *expected, "native oracle changed");
        assert_eq!(evaluate(&emitted_native), *expected, "{emitted_native}");
    }
}

#[test]
fn direct_literal_expression_keeps_static_order_dependent_merge_scope() {
    let cases = [
        (
            r#"let { c = 10; yield { ${"a"} = rec { b = c; }; a = { c = 2; }; }.a.b; }"#,
            r#"let c = 10; in { ${"a"} = rec { b = c; }; a = { c = 2; }; }.a.b"#,
            "2",
        ),
        (
            r#"let { c = 10; yield { a = { c = 2; }; ${"a"} = rec { b = c; }; }.a.b; }"#,
            r#"let c = 10; in { a = { c = 2; }; ${"a"} = rec { b = c; }; }.a.b"#,
            "10",
        ),
    ];

    let converted: Vec<_> = cases
        .iter()
        .map(|(nxc_source, native_source, _)| roundtrip(nxc_source, native_source))
        .collect();
    if !nix_available() {
        return;
    }
    for ((_, native_source, expected), emitted_native) in cases.iter().zip(converted) {
        assert_eq!(evaluate(native_source), *expected, "native oracle changed");
        assert_eq!(evaluate(&emitted_native), *expected, "{emitted_native}");
    }
}

#[test]
fn indented_key_fragments_preserve_recursive_scope() {
    for (key, name, expected) in [
        ("''a'''b''", "a''b", "1"),
        ("(''a'''b'')", "a''b", "1"),
        ("''${''a'''b''}''", "a''b", "1"),
        ("''a''\\b''", "ab", "1"),
        ("''a''", "a", "2"),
        (r#"''${"a"}''"#, "a", "2"),
        ("''  ''\\a''", "a", "2"),
    ] {
        let native = format!("let {name} = 1; in rec {{ ${{{key}}} = 2; y = {name}; }}.y");
        let nxc = format!("let {{ {name} = 1; yield rec {{ ${{{key}}} = 2; y = {name}; }}.y; }}");
        let output = roundtrip(&nxc, &native);
        if nix_available() {
            assert_eq!(
                evaluate(&native),
                expected,
                "native oracle changed: {native}"
            );
            assert_eq!(evaluate(&output), expected, "{output}");
        }
    }
}

#[test]
fn indented_key_static_restrictions_match_native_nix() {
    let native_available = nix_available();
    for (key, is_static) in [
        ("''a'''b''", false),
        ("''a$''", false),
        ("''a'$''", false),
        ("''a''\\b''", false),
        ("''${''a'''b''}''", false),
        (r#"''${""}x''"#, false),
        ("''x''\\ ''", false),
        ("''a''", true),
        ("''$a''", true),
        ("''''\\x''", true),
        ("''  ''\\x''", true),
        ("''''$''", true),
        ("''''", true),
        (r#"''${"x"}''"#, true),
        (r#"''${""}''"#, true),
        (r#""a\nb""#, true),
    ] {
        let native = format!("let ${{{key}}} = 1; in 0");
        let nxc = format!("let {{ ${{{key}}} = 1; yield 0; }}");
        assert_eq!(nix::import(&native).is_ok(), is_static, "{native}");
        assert_eq!(parse_nxc(&nxc).is_ok(), is_static, "{nxc}");
        let inherit = format!("{{ inherit ({{}}) ${{{key}}}; }}");
        assert_eq!(nix::import(&inherit).is_ok(), is_static, "{inherit}");
        assert_eq!(parse_nxc(&inherit).is_ok(), is_static, "{inherit}");
        if native_available {
            let result = Command::new("nix-instantiate")
                .args([
                    "--store", "dummy://", "--eval", "--strict", "--json", "--expr", &native,
                ])
                .output()
                .unwrap();
            assert_eq!(result.status.success(), is_static, "{native}: {result:?}");
        }
    }
}

#[test]
fn escaped_indented_key_collisions_remain_runtime_errors() {
    for source in [
        r#"{ ${''a'''b''} = 1; "a''b" = 2; }"#,
        r#"{ ${''a'''b''}.x = 1; "a''b".y = 2; }"#,
        r#"{ ${''a$''} = 1; "a$" = 2; }"#,
        r#"{ ${''${''a'''b''}''}.x = 1; "a''b".y = 2; }"#,
    ] {
        let output = roundtrip(source, source);
        if !nix_available() {
            continue;
        }
        for native in [source, &output] {
            let result = Command::new("nix-instantiate")
                .args([
                    "--store", "dummy://", "--eval", "--strict", "--json", "--expr", native,
                ])
                .output()
                .unwrap();
            assert!(!result.status.success(), "{native}");
            assert!(
                String::from_utf8_lossy(&result.stderr).contains("dynamic attribute"),
                "{native}: {result:?}"
            );
        }
    }
}
