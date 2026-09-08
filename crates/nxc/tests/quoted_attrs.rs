use nxc::{
    MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, emit,
    ir::{Binding, Expr},
    nix, parse_nxc, syntax,
};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) -> Expr {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let ir = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    assert_eq!(nix::import(native).unwrap(), ir, "{native}");
    let converted = emit::nxc(&ir).unwrap();
    let reparsed = parse_nxc(&converted).unwrap();
    assert_eq!(reparsed, ir, "{converted}");
    let output = nix::emit(&reparsed).unwrap();
    assert_eq!(nix::import(&output).unwrap(), ir, "{output}");
    assert_eq!(emit::nxc(&reparsed).unwrap(), converted);
    ir
}

#[test]
fn quoted_names_decode_to_static_keys_and_round_trip() {
    for (spelling, name) in [
        (r#""foo.bar""#, "foo.bar"),
        (r#""""#, ""),
        (r#""a b""#, "a b"),
        (r#""if""#, "if"),
        (r#""fn""#, "fn"),
        (r#""yield""#, "yield"),
        (r#""or""#, "or"),
        (r#""__nxc_update""#, "__nxc_update"),
        (r#""__curPos""#, "__curPos"),
        (r#""012""#, "012"),
        (r#""日本語""#, "日本語"),
        (r#""\"\\\n\r\t""#, "\"\\\n\r\t"),
        (r#""\${literal}""#, "${literal}"),
        (r#""\a""#, "a"),
        ("\"a\r\nb\rc\"", "a\nb\nc"),
    ] {
        let source = format!("{{ {spelling} = 1; }}");
        let expected = Expr::AttrSet {
            recursive: false,
            bindings: vec![Binding::Assign {
                path: vec![name.into()],
                value: Expr::Integer(1),
            }],
        };
        assert_eq!(roundtrip(&source, &source), expected);
        let selection = format!("s.{spelling} or 2");
        assert_eq!(
            roundtrip(&selection, &selection),
            Expr::Select {
                value: Box::new(Expr::Variable("s".into())),
                path: vec![name.into()],
                default: Some(Box::new(Expr::Integer(2))),
            }
        );
        let inherit = format!("{{ inherit (s) {spelling}; }}");
        assert_eq!(
            roundtrip(&inherit, &inherit),
            Expr::AttrSet {
                recursive: false,
                bindings: vec![Binding::Inherit {
                    source: Some(Expr::Variable("s".into())),
                    names: vec![name.into()],
                }],
            }
        );
    }
    assert_eq!(
        roundtrip(r#"{ "a"."b" = 1; }."a".b"#, "{ a.b = 1; }.a.b"),
        parse_nxc("{ a.b = 1; }.a.b").unwrap()
    );
}

#[test]
fn quoted_names_compose_with_scope_and_expression_forms() {
    for (source, native) in [
        (r#"let { "x" = 1; yield x; }"#, r#"let "x" = 1; in x"#),
        (
            r#"let { "a b" = 1; yield { inherit "a b"; }; }"#,
            r#"let "a b" = 1; in { inherit "a b"; }"#,
        ),
        (
            r#"let { "if" = 1; yield { inherit "if"; }; }"#,
            r#"let "if" = 1; in { inherit "if"; }"#,
        ),
        (
            r#"let { "" = 1; yield { inherit ""; }; }"#,
            r#"let "" = 1; in { inherit ""; }"#,
        ),
        (
            r#"let { inherit (s) "a b"; yield { inherit "a b"; }; }"#,
            r#"let inherit (s) "a b"; in { inherit "a b"; }"#,
        ),
        (
            r#"rec { "a.b" = 1; x = { inherit "a.b"; }; }.x"#,
            r#"rec { "a.b" = 1; x = { inherit "a.b"; }; }.x"#,
        ),
        (r#"{ "a.b" = 1; a.b = 2; }"#, r#"{ "a.b" = 1; a.b = 2; }"#),
        (
            r#"{ a."b.c" = 1; "a" = { d = 2; }; }"#,
            r#"{ a."b.c" = 1; "a" = { d = 2; }; }"#,
        ),
        (r#"s."f"(1)."result""#, r#"(s."f" 1)."result""#),
        (r#"s."f" or g(1)"#, r#"(s."f" or g) 1"#),
        (r#"s."f" or (g(1))"#, r#"s."f" or (g 1)"#),
        (r#"s."a" or t."b" or 2"#, r#"s."a" or t."b" or 2"#),
        (r#"[s."x", s."y"]"#, r#"[s."x" s."y"]"#),
        (r#"fn({ x ? s."x" }) => x"#, r#"{ x ? s."x" }: x"#),
        (r#""${s."x"}""#, r#""${s."x"}""#),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn emitters_quote_caller_supplied_names_without_changing_keys() {
    for name in ["", "a.b", "a b", "rec", "x; y", "${x}", "\"\\\n\r\t", "é"] {
        for ir in [
            Expr::AttrSet {
                recursive: false,
                bindings: vec![Binding::Assign {
                    path: vec![name.into()],
                    value: Expr::Integer(1),
                }],
            },
            Expr::Select {
                value: Box::new(Expr::Variable("s".into())),
                path: vec![name.into()],
                default: None,
            },
        ] {
            assert_eq!(parse_nxc(&emit::nxc(&ir).unwrap()).unwrap(), ir);
            assert_eq!(nix::import(&nix::emit(&ir).unwrap()).unwrap(), ir);
        }
    }
}

#[test]
fn decoded_aliases_conflict_but_dots_inside_names_do_not_split_paths() {
    for source in [
        r#"{ "a" = 1; a = 2; }"#,
        r#"{ "\a" = 1; a = 2; }"#,
        r#"{ a."b" = 1; "a".b = 2; }"#,
        r#"{ "a" = 1; a.b = 2; }"#,
        r#"{ a.b = 1; "a" = 2; }"#,
        r#"{ a = { "b" = 1; }; "a".b = 2; }"#,
        r#"{ inherit (s) "a" a; }"#,
        r#"{ inherit a; "a" = 1; }"#,
        r#"{ "" = 1; "" = 2; }"#,
        "{ \"a\r\nb\" = 1; \"a\\nb\" = 2; }",
    ] {
        assert!(parse_nxc(source).is_err(), "nxc accepted {source}");
        assert!(nix::import(source).is_err(), "native accepted {source}");
    }
    let source = r#"{ "a.b" = 1; a.b = 2; "".c = 3; }"#;
    roundtrip(source, source);
}

#[test]
fn unsupported_names_remain_lossless_errors_and_recover_later_items() {
    for source in [
        r#"{ "${}" = 1; }"#,
        r#"s."${}""#,
        r#"{ inherit (s) "${x}"; }"#,
        r#"{ ${} = 1; }"#,
        "{ ''x'' = 1; }",
        "s.''x''",
        "{ inherit (s) ''x''; }",
        r#"{ "unterminated = 1; }"#,
        "{ \"a\0b\" = 1; }",
        r#"s."x" or"#,
        r#"s."x" or -1"#,
        r#"s."x" or x => x"#,
        r#"fn({ "x" }) => x"#,
        r#""x" => x"#,
        r#"f("x": 1)"#,
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source:?}");
    }
    for native in [
        r#"{ "${}" = 1; }"#,
        r#"s."${}""#,
        r#"{ inherit (s) "${x}"; }"#,
        "{ ''x'' = 1; }",
        "s.''x''",
        "{ inherit (s) ''x''; }",
        r#"{ "x" }: x"#,
        r#"s."x" or x: x"#,
    ] {
        assert!(nix::import(native).is_err(), "accepted native {native}");
    }
    for name in ["fn", "yield", "or", "__curPos", "__nxc_update"] {
        for (source, native) in [
            (
                format!("let {{ \"{name}\" = 1; yield 2; }}"),
                format!("let \"{name}\" = 1; in 2"),
            ),
            (
                format!("{{ inherit \"{name}\"; }}"),
                format!("{{ inherit \"{name}\"; }}"),
            ),
        ] {
            assert!(parse_nxc(&source).is_err(), "unreserved {source}");
            assert!(nix::import(&native).is_err(), "unreserved {native}");
        }
    }
    for source in [
        r#"{ "x" = ; good = 3; }"#,
        r#"{ "x"..y = 1; good = 3; }"#,
        r#"let { "x" = @; good = 3; yield good; }"#,
        r#"f(s."x" or , { good = 3; })"#,
    ] {
        let parsed = syntax::parse(source);
        assert!(parsed.lower().is_err());
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(
            root.descendants()
                .any(|node| node.kind() == syntax::SyntaxKind::AssignBinding
                    && node.text() == "good = 3"),
            "lost recovery: {root:#?}"
        );
    }
}

#[test]
fn quoted_names_obey_byte_token_and_nesting_limits() {
    for name in ["\0".to_owned(), "a".repeat(MAX_SOURCE_BYTES + 1)] {
        for binding in [
            Binding::Assign {
                path: vec![name.clone().into()],
                value: Expr::Integer(1),
            },
            Binding::Inherit {
                source: Some(Expr::Variable("s".into())),
                names: vec![name.clone()],
            },
        ] {
            let ir = Expr::AttrSet {
                recursive: false,
                bindings: vec![binding],
            };
            assert!(emit::nxc(&ir).is_err());
            assert!(nix::emit(&ir).is_err());
        }
    }
    // Numeric names require quotes: { "<name>" = 1; } has eleven other bytes.
    let key = "0".repeat(MAX_SOURCE_BYTES - 11);
    let source = format!("{{ \"{key}\" = 1; }}");
    assert_eq!(source.len(), MAX_SOURCE_BYTES);
    let ir = roundtrip(&source, &source);
    assert_eq!(emit::nxc(&ir).unwrap().len(), MAX_SOURCE_BYTES);
    assert_eq!(nix::emit(&ir).unwrap().len(), MAX_SOURCE_BYTES);
    assert!(parse_nxc(&(source.clone() + " ")).is_err());
    assert!(nix::import(&(source + " ")).is_err());

    let bindings = (0..(MAX_TOKENS - 2) / 6)
        .map(|i| format!("\"{i}\" = 1; "))
        .collect::<String>();
    // Each quoted binding takes six tokens; the empty inherit adds the last two.
    let source = format!("{{ {bindings}inherit; }}");
    assert_eq!(
        syntax::lexer::lex(&source)
            .iter()
            .filter(|t| !t.kind.is_trivia())
            .count(),
        MAX_TOKENS
    );
    roundtrip(&source, &source);
    let over = format!("{{ {bindings}inherit; inherit; }}");
    assert!(parse_nxc(&over).is_err());
    assert!(nix::import(&over).is_err());

    let path = vec![r#""a.b""#; MAX_DEPTH].join(".");
    let source = format!("s.{path}");
    roundtrip(&source, &source);
    assert!(parse_nxc(&(source.clone() + ".a")).is_err());
    assert!(nix::import(&(source + ".a")).is_err());
    let nested = (0..MAX_DEPTH - 1).fold("1".to_owned(), |value, _| {
        format!("{{ \"a.b\" = {value}; }}")
    });
    roundtrip(&nested, &nested);
    let over = format!("{{ \"a.b\" = {nested}; }}");
    assert!(parse_nxc(&over).is_err());
    assert!(nix::import(&over).is_err());
}

#[test]
fn native_nix_confirms_quoted_names_scope_laziness_and_values() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    for (source, expected) in [
        (r#"{ "a.b" = 1; a.b = 2; }."a.b""#, Some("1")),
        (r#"{ "a.b" = 1; a.b = 2; }.a.b"#, Some("2")),
        (r#"{ "" = 3; }."""#, Some("3")),
        (r#"{ "if" = 4; }."if""#, Some("4")),
        (r#"{ "\${x}" = 5; }."\${x}""#, Some("5")),
        (r#"{ "a\nb" = 6; }."a\nb""#, Some("6")),
        ("{ \"a\r\nb\" = 6; }.\"a\\nb\"", Some("6")),
        (r#"{ "x" = 1; }."x" or (abort "unused")"#, Some("1")),
        (r#"{}."a.b" or 7"#, Some("7")),
        (r#"{ "x" = abort "forced"; }."x" or 7"#, None),
        (r#"{ "a" = 1; }."a"."b" or 7"#, Some("7")),
        (r#"let "x" = y; "y" = 8; in x"#, Some("8")),
        (r#"let "a b" = 9; in { inherit "a b"; }."a b""#, Some("9")),
        (
            r#"let "a b" = 9; in rec { "a b" = 10; x = { inherit "a b"; }; }.x."a b""#,
            Some("10"),
        ),
        (
            r#"let "a b" = 9; in rec { inherit "a b"; }."a b""#,
            Some("9"),
        ),
        (
            r#"let inherit ({ "a b" = 11; }) "a b"; in { inherit "a b"; }."a b""#,
            Some("11"),
        ),
        (r#"let "if" = 12; in { inherit "if"; }."if""#, Some("12")),
        (r#"let "" = 13; in { inherit ""; }."""#, Some("13")),
        (
            r#"{ "a" = rec { b = c; }; a = { "c" = 14; }; }.a.b"#,
            Some("14"),
        ),
        (
            r#"let c = 15; in { a = { b = c; }; "a" = rec { c = 14; }; }.a.b"#,
            Some("15"),
        ),
        (
            r#"{ inherit (abort "unused") "a b"; }.missing or 16"#,
            Some("16"),
        ),
        (
            r#"{ "a b" = 17; } // { "a b" = 18; }"#,
            Some(r#"{"a b":18}"#),
        ),
        (r#"{ "f" = x: x; }."f" 19"#, Some("19")),
        (r#"({ x ? {}."missing" or 20 }: x) {}"#, Some("20")),
        (
            r#"builtins.attrNames { "b" = 1; "a.b" = 2; "" = 3; }"#,
            Some(r#"["","a.b","b"]"#),
        ),
        (r#"let inherit "absent name"; in 1"#, None),
    ] {
        let ir = nix::import(source).unwrap();
        let nxc = emit::nxc(&ir).unwrap();
        let reparsed = parse_nxc(&nxc).unwrap();
        assert_eq!(ir, reparsed);
        let output = nix::emit(&reparsed).unwrap();
        assert_eq!(nix::import(&output).unwrap(), ir);
        for native in [source, &output] {
            let result = Command::new("nix-instantiate")
                .args([
                    "--store",
                    "dummy://",
                    "--eval",
                    "--strict",
                    "--json",
                    "--expr",
                    &format!("({native})"),
                ])
                .output()
                .unwrap();
            if let Some(expected) = expected {
                assert!(
                    result.status.success(),
                    "{native}: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert_eq!(
                    String::from_utf8(result.stdout).unwrap().trim(),
                    expected,
                    "{native}"
                );
            } else {
                assert!(!result.status.success(), "{native} must fail");
            }
        }
    }
}
