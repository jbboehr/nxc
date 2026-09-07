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

#[test]
fn lambdas_preserve_scope_currying_lazy_defaults_and_argument_checks() {
    if !nix_available() {
        return;
    }
    for (source, argument, expected) in [
        ("x: y: x * 10 + y", "2 3", Some("23")),
        ("x: (x: x + 1) 4 + x", "10", Some("15")),
        ("x: 7", "(1 / 0)", Some("7")),
        ("true: true", "5", Some("5")),
        ("{ x, y ? x + 1 }: y", "{ x = 4; }", Some("5")),
        ("{ x ? y + 1, y ? 4 }: x", "{}", Some("5")),
        ("{ x ? 1 / 0 }: x", "{ x = 8; }", Some("8")),
        ("{ x ? 1 / 0 }: 7", "{}", Some("7")),
        ("{ x ? 1, ... }: x", "{ extra = 2; }", Some("1")),
        ("args@{ x ? 7 }: args", "{}", Some("{}")),
        ("args@{ x ? args, ... }: x", "{ y = 3; }", Some("{\"y\":3}")),
        ("{ f ? x: x + 1 }: f 2", "{}", Some("3")),
        ("{ x }: x", "{}", None),
        ("{ x }: 7", "{}", None),
        ("{ x }: x", "{ x = 1; extra = 2; }", None),
        ("{ x ? 1 / 0 }: x", "{}", None),
        ("{ ... }: 1", "2", None),
    ] {
        let original = nix::import(source).unwrap();
        let converted = nxc::emit::nxc(&original).unwrap();
        let generated = nix::emit(&parse_nxc(&converted).unwrap()).unwrap();
        for lambda in [source, generated.as_str()] {
            let expression = format!("({lambda}) {argument}");
            let result = Command::new("nix-instantiate")
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
            if let Some(expected) = expected {
                assert!(
                    result.status.success(),
                    "{expression}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(
                    String::from_utf8(result.stdout).unwrap().trim(),
                    expected,
                    "{expression}"
                );
            } else {
                assert!(!result.status.success(), "{expression} must fail");
            }
        }
    }
}

#[test]
fn native_selection_default_syntax_matches_nix() {
    if !nix_available() {
        return;
    }
    for (source, accepted) in [
        ("{}.a or x: x", false),
        ("{}.a or { x }: x", false),
        ("{}.a or args@{ x }: x", false),
        ("{}.a or { x }@args: x", false),
        ("{}.a or {}.b or x: x", false),
        ("{}.a or (x: x)", true),
        ("{}.a or ({ x }: x)", true),
        ("{}.a or {}.b or (x: x)", true),
        ("{}.a or { f = x: x; }", true),
        ("{ f ? x: x }: f", true),
    ] {
        let parsed = Command::new("nix-instantiate")
            .args(["--store", "dummy://", "--parse", "--expr", source])
            .output()
            .unwrap();
        assert_eq!(parsed.status.success(), accepted, "native Nix: {source}");
        assert_eq!(nix::import(source).is_ok(), accepted, "importer: {source}");
    }
}

#[test]
fn attrsets_preserve_merging_recursion_inheritance_and_lazy_selection() {
    if !nix_available() {
        return;
    }
    for (source, expected) in [
        ("(rec { a = b + 1; b = 2; }).a", Some("3")),
        ("({ a.b = 1; a.c = 2; }).a", Some("{\"b\":1,\"c\":2}")),
        ("({ a = rec { b = c; }; a.c = 2; }).a.b", Some("2")),
        ("({ a.b = c; a = rec { c = 2; }; }).a.b", Some("10")),
        ("({ a = { b = c; }; a = rec { c = 2; }; }).a.b", Some("10")),
        ("({ a = rec { b = c; }; a = { c = 2; }; }).a.b", Some("2")),
        ("(rec { inherit x; y = x; }).y", Some("7")),
        ("(rec { inherit (src) x; src = { x = 8; }; }).x", Some("8")),
        ("{ inherit (1 / 0); }", Some("{}")),
        ("{ unused = 1 / 0; a = 4; }.a", Some("4")),
        ("{ a = 5; }.a or (1 / 0)", Some("5")),
        ("{}.a.b or 6", Some("6")),
        ("{ a = 1; }.a.b or 6", Some("6")),
        ("{ a = 5; }.a or 2 + 3", Some("8")),
        ("{ a = f; }.a or 0 2", Some("3")),
        ("{ a = 5; }.a or (f 2)", Some("5")),
        ("({ x, y ? x + 1 }: y) { x = 2; }", Some("3")),
        ("({ fn = 1; yield = 2; or = 3; }).or", Some("3")),
        ("{}.a", None),
        ("{ a = 1 / 0; }.a or 4", None),
        ("(1 / 0).a or 4", None),
        ("(rec { a = a; }).a", None),
    ] {
        let original = nix::import(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let converted = nxc::emit::nxc(&original).unwrap();
        let generated = nix::emit(&parse_nxc(&converted).unwrap()).unwrap();
        for value in [source, generated.as_str()] {
            let expression = format!("let x = 7; c = 10; f = x: x + 1; in {value}");
            let result = Command::new("nix-instantiate")
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
            if let Some(expected) = expected {
                assert!(
                    result.status.success(),
                    "{expression}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(
                    String::from_utf8(result.stdout).unwrap().trim(),
                    expected,
                    "{expression}"
                );
            } else {
                assert!(!result.status.success(), "{expression} must fail");
            }
        }
    }
}

#[test]
fn strings_preserve_values_coercion_and_lazy_interpolation() {
    if !nix_available() {
        return;
    }
    for (source, expected) in [
        (r#""""#, Some(r#""""#)),
        (r#""hello 🦀""#, Some(r#""hello 🦀""#)),
        (r#""\n\r\t\"\\""#, Some(r#""\n\r\t\"\\""#)),
        (r#""\q\0\x41\u0041""#, Some(r#""q0x41u0041""#)),
        ("\"a\rb\"", Some(r#""a\nb""#)),
        ("\"a\r\nb\"", Some(r#""a\nb""#)),
        ("\"a\\\rb\"", Some(r#""a\rb""#)),
        ("\"a\\\r\nb\"", Some(r#""a\r\nb""#)),
        (r#""\${x}""#, Some(r#""${x}""#)),
        (r#""$${x}""#, Some(r#""$${x}""#)),
        (r#""$$${x}""#, Some(r#""$$X""#)),
        (r#""\$${x}""#, Some(r#""$X""#)),
        (r#""${"inner ${x}"}""#, Some(r#""inner X""#)),
        (r#"(x: "${x}") "value""#, Some(r#""value""#)),
        (r#""${{ __toString = self: "ok"; }}""#, Some(r#""ok""#)),
        (r#"{ unused = "${1 / 0}"; a = "ok"; }.a"#, Some(r#""ok""#)),
        (r#"{ a = "ok"; }.a or "${1 / 0}""#, Some(r#""ok""#)),
        (r#""${1}""#, None),
        (r#""${null}""#, None),
        (r#""${{}}""#, None),
        (r#""${x: x}""#, None),
        (r#""${1 / 0}""#, None),
    ] {
        let original = nix::import(source).unwrap();
        let converted = nxc::emit::nxc(&original).unwrap();
        let generated = nix::emit(&parse_nxc(&converted).unwrap()).unwrap();
        for value in [source, generated.as_str()] {
            let expression = format!("let x = \"X\"; in {value}");
            let result = Command::new("nix-instantiate")
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
            if let Some(expected) = expected {
                assert!(
                    result.status.success(),
                    "{expression}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(
                    String::from_utf8(result.stdout).unwrap().trim(),
                    expected,
                    "{expression}"
                );
            } else {
                assert!(!result.status.success(), "{expression} must fail");
            }
        }
    }
}

#[test]
fn interpolation_retains_nix_string_context() {
    if !nix_available() {
        return;
    }
    for source in [
        r#""a${x}b""#,
        "''a${x}b''",
        "''\n  a${x}b''",
        r#"with { y = x; }; "a${y}b""#,
        r#"if true then "a${x}b" else "unused""#,
        r#"if false then "unused" else "a${x}b""#,
        r#"assert true; "a${x}b""#,
        r#"assert x == x && !(x != x); "a${x}b""#,
        r#"({ value = "unused"; } // { value = "a${x}b"; }).value"#,
        r#"builtins.head ([] ++ ["a${x}b"])"#,
        r#"assert x == x -> x != ""; "a${x}b""#,
    ] {
        let converted = nxc::emit::nxc(&nix::import(source).unwrap()).unwrap();
        let generated = nix::emit(&parse_nxc(&converted).unwrap()).unwrap();
        for value in [source, generated.as_str()] {
            let expression = format!(
                r#"
            let x = builtins.appendContext "payload" {{
                "/nix/store/00000000000000000000000000000000-fixture" = {{ path = true; }};
            }}; value = {value};
            in builtins.getContext value == builtins.getContext x && value == "apayloadb"
        "#
            );
            let result = Command::new("nix-instantiate")
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
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), "true");
        }
    }
}

#[test]
fn lists_preserve_order_boundaries_and_lazy_elements() {
    if !nix_available() {
        return;
    }
    for (source, native, expected) in [
        ("[]", "[]", Some("[]")),
        ("[3, 1, 3, 2]", "[3 1 3 2]", Some("[3,1,3,2]")),
        ("[[] [1, 2] [3]]", "[[] [1 2] [3]]", Some("[[],[1,2],[3]]")),
        (
            r#"[1 "two" null { a = 3; }]"#,
            r#"[1 "two" null { a = 3; }]"#,
            Some(r#"[1,"two",null,{"a":3}]"#),
        ),
        ("[4 - 2]", "[(4 - 2)]", Some("[2]")),
        ("[4, -2]", "[4 (-2)]", Some("[4,-2]")),
        ("[f (2)]", "[(f 2)]", Some("[3]")),
        (
            "builtins.length([f, (2)])",
            "builtins.length [f (2)]",
            Some("2"),
        ),
        (
            "builtins.length([f(2)])",
            "builtins.length [(f 2)]",
            Some("1"),
        ),
        (
            "builtins.head([1, 1 / 0])",
            "builtins.head [1 (1 / 0)]",
            Some("1"),
        ),
        (
            "builtins.length([1 / 0, 2])",
            "builtins.length [(1 / 0) 2]",
            Some("2"),
        ),
        (
            "builtins.tail([1 / 0, 2])",
            "builtins.tail [(1 / 0) 2]",
            Some("[2]"),
        ),
        (
            "builtins.elemAt([1, 1 / 0, 3], 2)",
            "builtins.elemAt [1 (1 / 0) 3] 2",
            Some("3"),
        ),
        (
            "builtins.map(x => x + 1, [1, 2])",
            "builtins.map (x: x + 1) [1 2]",
            Some("[2,3]"),
        ),
        (
            "builtins.length([x => x, ({a}) => a])",
            "builtins.length [(x: x) ({a}: a)]",
            Some("2"),
        ),
        ("(({}) => [1])({})", "({}: [1]) {}", Some("[1]")),
        (
            "{ a = [1]; }.a or [1 / 0]",
            "{ a = [1]; }.a or [(1 / 0)]",
            Some("[1]"),
        ),
        ("{}.a or [1, 2]", "{}.a or [1 2]", Some("[1,2]")),
        (
            "(rec { xs = [1, builtins.head(xs)]; }).xs",
            "(rec { xs = [1 (builtins.head xs)]; }).xs",
            Some("[1,1]"),
        ),
        ("[1, 1 / 0]", "[1 (1 / 0)]", None),
        ("builtins.head([])", "builtins.head []", None),
        ("[1](2)", "[1] 2", None),
    ] {
        let original = nix::import(native).unwrap();
        let actual = parse_nxc(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        assert_eq!(actual, original, "{source}");
        let converted = nxc::emit::nxc(&original).unwrap();
        let generated = nix::emit(&parse_nxc(&converted).unwrap()).unwrap();
        for value in [native, generated.as_str()] {
            let expression = format!("let f = x: x + 1; in {value}");
            let result = Command::new("nix-instantiate")
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
            if let Some(expected) = expected {
                assert!(
                    result.status.success(),
                    "{expression}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(
                    String::from_utf8(result.stdout).unwrap().trim(),
                    expected,
                    "{expression}"
                );
            } else {
                assert!(!result.status.success(), "{expression} must fail");
            }
        }
    }
}

#[test]
fn native_list_lambda_syntax_matches_nix() {
    if !nix_available() {
        return;
    }
    for (source, valid) in [
        ("[x: x]", false),
        ("[{}: 1]", false),
        ("[{x}: x]", false),
        ("[args@{}: args]", false),
        ("[{}@args: args]", false),
        ("[{}.a or x: x]", false),
        ("[1 + 2]", false),
        ("[-1]", false),
        ("[(x: x)]", true),
        ("[({x}: x)]", true),
        ("[{}.a or (x: x)]", true),
    ] {
        let native = Command::new("nix-instantiate")
            .args(["--store", "dummy://", "--parse", "--expr", source])
            .output()
            .unwrap();
        assert_eq!(
            native.status.success(),
            valid,
            "{source}: {}",
            String::from_utf8_lossy(&native.stderr)
        );
        assert_eq!(nix::import(source).is_ok(), valid, "{source}");
    }
}
