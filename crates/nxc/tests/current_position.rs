use nxc::{Limits, emit, ir::Expr, nix, parse_nxc, syntax};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) -> String {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let actual = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    let expected = nix::import(native).unwrap_or_else(|e| panic!("{native}: {e:?}"));
    assert_eq!(actual, expected);
    let converted = emit::nxc(&expected).unwrap();
    let reparsed = parse_nxc(&converted).unwrap();
    assert_eq!(reparsed, expected);
    let generated = nix::emit(&reparsed).unwrap();
    assert_eq!(nix::import(&generated).unwrap(), expected);
    generated
}

#[test]
fn current_position_survives_conversion_in_expression_contexts() {
    for (source, native) in [
        ("/* position */ __curPos", "# position\n__curPos"),
        ("__curPos.file", "__curPos.file"),
        ("__curPos ? line", "__curPos ? line"),
        ("f(__curPos)", "f __curPos"),
        ("__curPos(x)", "__curPos x"),
        ("[__curPos, __curPos.file]", "[__curPos __curPos.file]"),
        ("s.a or __curPos", "s.a or __curPos"),
        ("__curPos == null", "__curPos == null"),
        (
            "if false then __curPos else 1",
            "if false then __curPos else 1",
        ),
        ("assert(false, __curPos)", "assert false; __curPos"),
        ("fn({ x ? __curPos }) => x", "{ x ? __curPos }: x"),
        (
            "{ inherit (__curPos) file; }",
            "{ inherit (__curPos) file; }",
        ),
        (r#""${__curPos.file}""#, r#""${__curPos.file}""#),
        ("''${__curPos.file}''", "''${__curPos.file}''"),
        ("./${__curPos}", "./${__curPos}"),
        ("{ ${__curPos} = 1; }", "{ ${__curPos} = 1; }"),
        ("{ ${null} = __curPos; }", "{ ${null} = __curPos; }"),
        ("s.${__curPos}", "s.${__curPos}"),
        ("s ? missing.${__curPos}", "s ? missing.${__curPos}"),
    ] {
        roundtrip(source, native);
    }
    let position = parse_nxc("__curPos").unwrap();
    assert_ne!(position, Expr::Variable("__curPos".into()));
    assert_eq!(emit::nxc(&position).unwrap(), "__curPos");
    assert_eq!(nix::emit(&position).unwrap(), "__curPos");
}

#[test]
fn current_position_bindings_and_attribute_names_keep_their_native_roles() {
    for (source, native) in [
        ("__curPos => __curPos", "__curPos: __curPos"),
        (
            "fn({ __curPos ? __curPos }) => __curPos",
            "{ __curPos ? __curPos }: __curPos",
        ),
        ("(__curPos@{}) => __curPos", "__curPos@{}: __curPos"),
        (
            "let { __curPos = 7; yield __curPos; }",
            "let __curPos = 7; in __curPos",
        ),
        (
            "let { \"__curPos\" = 7; yield { inherit \"__curPos\"; }; }",
            "let \"__curPos\" = 7; in { inherit \"__curPos\"; }",
        ),
        ("{ __curPos = 7; }.__curPos", "{ __curPos = 7; }.__curPos"),
        (
            "rec { __curPos = 7; x = __curPos; }",
            "rec { __curPos = 7; x = __curPos; }",
        ),
        (
            "with({ __curPos = 7; }, __curPos)",
            "with { __curPos = 7; }; __curPos",
        ),
        ("{ inherit (s) __curPos; }", "{ inherit (s) __curPos; }"),
        ("{ inherit __curPos; }", "{ inherit __curPos; }"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn current_position_recognition_requires_the_exact_identifier() {
    for name in ["__curPos0", "x__curPos"] {
        let expected = Expr::Variable(name.into());
        assert_eq!(parse_nxc(name).unwrap(), expected, "nxc identifier {name}");
        assert_eq!(
            nix::import(name).unwrap(),
            expected,
            "Nix identifier {name}"
        );
        assert_eq!(emit::nxc(&expected).unwrap(), name);
        assert_eq!(nix::emit(&expected).unwrap(), name);
    }
}

#[test]
fn current_position_uses_the_existing_resource_limits() {
    let position = parse_nxc("__curPos").unwrap();
    let exact = Limits::new(8, 1).unwrap();
    assert_eq!(
        nxc::parse_nxc_with_limits("__curPos", exact).unwrap(),
        position
    );
    assert_eq!(
        nix::import_with_limits("__curPos", exact).unwrap(),
        position
    );
    assert_eq!(emit::nxc_with_limits(&position, exact).unwrap(), "__curPos");
    assert_eq!(nix::emit_with_limits(&position, exact).unwrap(), "__curPos");
    for limits in [Limits::new(7, 1).unwrap(), Limits::new(8, 0).unwrap()] {
        assert!(nxc::parse_nxc_with_limits("__curPos", limits).is_err());
        assert!(nix::import_with_limits("__curPos", limits).is_err());
        assert!(emit::nxc_with_limits(&position, limits).is_err());
        assert!(nix::emit_with_limits(&position, limits).is_err());
    }
    let invalid = Expr::Variable("__curPos".into());
    assert!(emit::nxc(&invalid).is_err());
    assert!(nix::emit(&invalid).is_err());
}

#[test]
fn native_nix_keeps_inline_positions_distinct_from_lexical_bindings() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(output) => assert!(output.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    for (source, expected) in [
        ("__curPos", "null"),
        ("let null = 9; __curPos = 7; in __curPos", "null"),
        ("(__curPos: __curPos) 7", "null"),
        (
            "let __curPos = 7; in { inherit __curPos; }",
            r#"{"__curPos":7}"#,
        ),
        ("(__curPos: { inherit __curPos; }) 7", r#"{"__curPos":7}"#),
        (
            "({ __curPos ? 7 }: [ __curPos { inherit __curPos; } ]) {}",
            r#"[null,{"__curPos":7}]"#,
        ),
        (
            "(__curPos@{}: { inherit __curPos; }) {}",
            r#"{"__curPos":{}}"#,
        ),
        (
            "with { __curPos = 7; }; { inherit __curPos; }",
            r#"{"__curPos":7}"#,
        ),
        ("with { __curPos = 7; }; __curPos", "null"),
        (
            "rec { __curPos = 7; x = __curPos; }",
            r#"{"__curPos":7,"x":null}"#,
        ),
        ("{ ${null} = __curPos.file; }", "{}"),
        ("{} ? missing.${__curPos.file}", "false"),
        ("if true then 7 else __curPos.file", "7"),
        (
            "builtins.functionArgs ({ __curPos ? 7 }: 1)",
            r#"{"__curPos":true}"#,
        ),
    ] {
        let original = nix::import(source).unwrap();
        let generated = nix::emit(&parse_nxc(&emit::nxc(&original).unwrap()).unwrap()).unwrap();
        for value in [source, &generated] {
            let output = Command::new("nix-instantiate")
                .args([
                    "--store", "dummy://", "--eval", "--strict", "--json", "--expr", value,
                ])
                .output()
                .unwrap();
            assert!(output.status.success(), "{value}: {output:?}");
            assert_eq!(
                String::from_utf8(output.stdout).unwrap().trim(),
                expected,
                "{value}"
            );
        }
    }
}
