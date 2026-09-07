use nxc::{emit, nix, parse_nxc, syntax};

fn roundtrip(source: &str, native: &str) {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let actual = parsed
        .lower()
        .unwrap_or_else(|e| panic!("{source:?}: {e:?}"));
    let expected = nix::import(native).unwrap_or_else(|e| panic!("{native:?}: {e:?}"));
    assert_eq!(actual, expected);
    let generated = emit::nxc(&expected).unwrap();
    assert_eq!(parse_nxc(&generated).unwrap(), expected, "{generated:?}");
    let generated = nix::emit(&actual).unwrap();
    assert_eq!(nix::import(&generated).unwrap(), actual, "{generated:?}");
}

#[test]
fn quoted_strings_and_nested_interpolations_roundtrip() {
    for source in [
        r#""""#,
        r#""hello 🦀""#,
        r##""# // /* */ ( [ { ; , fn => or }""##,
        r#""\n\r\t\"\\\q\0\x41\u0041""#,
        r#""\${literal}""#,
        r#""$${literal}""#,
        r#""$$${x}""#,
        r#""$\${literal}""#,
        r#""\$${x}""#,
        r#""${x}${y}""#,
        r#""before ${x} after""#,
        r#""${{ x = "}"; }.x}""#,
        r#""${"inner ${x}"}""#,
        r#""${ /* } \" */ x # } \"
        }""#,
        r#"{ a = "value"; }.a or "fallback""#,
    ] {
        roundtrip(source, source);
    }
    roundtrip(r#""${f("x", 2)}""#, r#""${f "x" 2}""#);
    roundtrip(r#""${(x => x)("value")}""#, r#""${(x: x) "value"}""#);
    roundtrip(
        r#""${x // } " remains a comment
        }""#,
        r#""${x # } " remains a comment
        }""#,
    );
}

#[test]
fn string_escape_spellings_share_the_same_ir() {
    for (source, canonical) in [
        ("\"a\rb\"", r#""a\nb""#),
        ("\"a\r\nb\"", r#""a\nb""#),
        ("\"a\\\rb\"", r#""a\rb""#),
        ("\"a\\\r\nb\"", r#""a\r\nb""#),
        ("\"a\\\nb\"", r#""a\nb""#),
        (r#""\q\0\x41\u0041""#, r#""q0x41u0041""#),
        (r#""$${literal}""#, r#""\$\${literal}""#),
    ] {
        roundtrip(source, canonical);
        assert_eq!(
            nix::import(source).unwrap(),
            nix::import(canonical).unwrap()
        );
    }
}

#[test]
fn malformed_and_unsupported_strings_remain_lossless_errors() {
    for source in [
        "\"",
        "\"unterminated",
        "\"dangling\\",
        r#""${}""#,
        r#""${x""#,
        r#""${@}""#,
        r#""${"nested}""#,
        "\"a\0b\"",
        r#"{ "quoted" = 1; }"#,
        r#"s."quoted""#,
        r#""${assert true; 1}""#,
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source:?}");
        assert!(nix::import(source).is_err(), "native accepted {source:?}");
    }
}

#[test]
fn recovery_skips_strings_and_interpolations_as_nested_groups() {
    for source in [
        r#"f(@ + "commas, ; ${g(1, 2)}", h(3), 4)"#,
        r#"f("${@}", h(3), 4)"#,
        r#"{ a = @ + "${{ inner = "; } ,"; }}"; b = h(3); }"#,
    ] {
        let parsed = syntax::parse(source);
        assert!(parsed.lower().is_err());
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(
            root.descendants()
                .any(|node| node.kind() == syntax::SyntaxKind::CallExpr && node.text() == "h(3)"),
            "lost later call: {root:#?}"
        );
    }
}

#[test]
fn string_delimiters_and_interpolations_obey_resource_limits() {
    let nested = (0..nxc::MAX_DEPTH / 2 - 1)
        .fold(r#""x""#.to_owned(), |inner, _| format!("\"${{{inner}}}\""));
    let at_depth = format!("({nested})");
    roundtrip(&at_depth, &at_depth);
    let over_depth = format!("\"${{{nested}}}\"");
    let parsed = syntax::parse(&over_depth);
    assert_eq!(parsed.syntax().unwrap().to_string(), over_depth);
    assert_eq!(parsed.diagnostics().len(), 1);
    assert!(parsed.lower().is_err());
    assert!(nix::import(&over_depth).is_err());

    let at_tokens = format!("\"a{}b\"", "${x}".repeat((nxc::MAX_TOKENS - 4) / 3));
    assert_eq!(syntax::lexer::lex(&at_tokens).len(), nxc::MAX_TOKENS);
    roundtrip(&at_tokens, &at_tokens);
    let over_tokens = at_tokens.replacen('a', "a${x}", 1);
    let parsed = syntax::parse(&over_tokens);
    assert_eq!(parsed.syntax().unwrap().to_string(), over_tokens);
    assert_eq!(parsed.diagnostics().len(), 1);
    assert!(parsed.lower().is_err());
    assert!(nix::import(&over_tokens).is_err());
}

#[test]
fn emitters_validate_string_parts_and_escaped_output_size() {
    use nxc::ir::{Expr, StringPart};
    let exact = Expr::String(vec![StringPart::Literal(
        "\"".repeat((nxc::MAX_SOURCE_BYTES - 2) / 2),
    )]);
    for source in [emit::nxc(&exact).unwrap(), nix::emit(&exact).unwrap()] {
        assert_eq!(source.len(), nxc::MAX_SOURCE_BYTES);
        assert_eq!(parse_nxc(&source).unwrap(), exact);
        assert_eq!(nix::import(&source).unwrap(), exact);
    }
    for parts in [
        vec![StringPart::Literal("\"".repeat(nxc::MAX_SOURCE_BYTES / 2))],
        vec![StringPart::Literal("x".repeat(nxc::MAX_SOURCE_BYTES + 1))],
        vec![StringPart::Literal(String::new())],
        vec![
            StringPart::Literal("a".into()),
            StringPart::Literal("b".into()),
        ],
        vec![StringPart::Literal("\0".into())],
        vec![StringPart::Interpolation(Expr::Variable("__curPos".into()))],
    ] {
        let expr = Expr::String(parts);
        assert!(emit::nxc(&expr).is_err());
        assert!(nix::emit(&expr).is_err());
    }
}
