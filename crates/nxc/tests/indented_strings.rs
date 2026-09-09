mod support;
use support::nxc;

use nxc::{
    MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, emit,
    ir::{Expr, StringPart},
    nix, parse_nxc, syntax,
};
use std::{io::ErrorKind, process::Command};

fn literal_cases() -> Vec<(&'static str, &'static str)> {
    vec![
        ("''''", ""),
        ("''   ''", ""),
        ("''\n''", ""),
        ("''\n  a\n    b\n ''", "a\n  b\n"),
        ("''  a\n    b''", "a\n  b"),
        ("''\n\n  a\n \n    \n  b\n  ''", "\na\n\n  \nb\n"),
        ("''a ''", "a "),
        ("''\t\n  a\n''", "\t\n  a\n"),
        ("''\n  \ta\n    b\n''", "\ta\n  b\n"),
        ("''\r\n  a\r\n ''", "\r\n  a\r\n"),
        ("''a\rb''", "a\rb"),
        ("''a\r\nb''", "a\r\nb"),
        ("''a''\\\rb''", "a\rb"),
        ("''\n  a\n    ''\\ ''", "a\n   "),
        ("''\n  a\n    ''\\n    b\n  ''", "a\n  \n  b\n"),
        ("''\n  a\n    ''\\n    ''", "a\n  \n  "),
        (r#"''\n\t\r\q"hi"''"#, "\\n\\t\\r\\q\"hi\""),
        (r#"''a''\n''\r''\t''\q''\🦀''"#, "a\n\r\tq🦀"),
        ("''a'''b''", "a''b"),
        ("''a''''b''", "a'''b"),
        ("''$${x}''", "$${x}"),
        ("''''${x}''", "${x}"),
    ]
}

fn roundtrip(source: &str, native: &str) -> Expr {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let actual = parsed
        .lower()
        .unwrap_or_else(|e| panic!("{source:?}: {e:?}"));
    let expected = nix::import(native).unwrap_or_else(|e| panic!("{native:?}: {e:?}"));
    assert_eq!(actual, expected, "{source:?}");
    assert_eq!(parse_nxc(&emit::nxc(&expected).unwrap()).unwrap(), expected);
    assert_eq!(nix::import(&nix::emit(&actual).unwrap()).unwrap(), actual);
    actual
}

#[test]
fn indentation_and_escapes_decode_to_exact_literal_values() {
    for (source, value) in literal_cases() {
        let expected = Expr::String(if value.is_empty() {
            vec![]
        } else {
            vec![StringPart::Literal(value.into())]
        });
        assert_eq!(roundtrip(source, source), expected, "{source:?}");
    }
    let source = format!("''{}''", " ".repeat(1_000_004));
    assert_eq!(
        roundtrip(&source, &source),
        Expr::String(vec![StringPart::Literal("    ".into())])
    );
}

#[test]
fn interpolations_and_nested_quote_styles_preserve_ir() {
    for (source, native) in [
        ("''\n  a\n  ${x}\n    b\n''", "''\n  a\n  ${x}\n    b\n''"),
        ("''${''inner ${x}''}''", "''${''inner ${x}''}''"),
        (
            r#""${''inner ${"nested"}''}""#,
            r#""${''inner ${"nested"}''}""#,
        ),
        ("''${f(''x'', [1, 2])}''", "''${f ''x'' [1 2]}''"),
        (
            "[x'' ''value'' ''${{ a = ''nested''; }.a}'']",
            "[x'' ''value'' ''${{ a = ''nested''; }.a}'']",
        ),
        (
            "''. # /* \" ${x // '' }\n}''",
            "''. # /* \" ${x # '' }\n}''",
        ),
        ("s.a or ''fallback''", "s.a or ''fallback''"),
    ] {
        roundtrip(source, native);
    }
    let expected = Expr::String(vec![
        StringPart::Literal("a\n".into()),
        StringPart::Interpolation(Expr::Variable("x".into())),
        StringPart::Literal("\n  b\n".into()),
    ]);
    assert_eq!(parse_nxc("''\n  a\n  ${x}\n    b\n''").unwrap(), expected);
}

#[test]
fn malformed_indented_strings_remain_lossless_errors_and_recover() {
    for source in [
        "''",
        "'''",
        "''x",
        "''x'''",
        "''x''\\",
        "''${}''",
        "''${x''",
        "''a\0b''",
        "''a''\\\0b''",
        "{ ''name'' = 1; }",
        "s.''name''",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source:?}");
        assert!(nix::import(source).is_err(), "native accepted {source:?}");
    }
    for source in [
        "f(@ + ''commas, ; ${g(1, 2)}'', h(3))",
        "[''${@}'', h(3)]",
        "{ a = @ + ''${{ inner = ''}; ,''; }}''; b = h(3); }",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(
            root.descendants()
                .any(|n| n.kind() == syntax::SyntaxKind::CallExpr && n.text() == "h(3)"),
            "lost later call: {root:#?}"
        );
    }
}

#[test]
fn indented_strings_obey_source_token_and_depth_limits() {
    let nested =
        (0..MAX_DEPTH / 2 - 1).fold("''x''".into(), |inner, _| format!("''${{{inner}}}''"));
    roundtrip(&format!("({nested})"), &format!("({nested})"));
    let tokens = format!("''a{}b''", "${x}".repeat((MAX_TOKENS - 4) / 3));
    roundtrip(&tokens, &tokens);
    for source in [
        format!("''${{{nested}}}''"),
        tokens.replacen('a', "a${x}", 1),
    ] {
        let parsed = syntax::parse(&source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert_eq!(parsed.diagnostics().len(), 1);
        assert!(parsed.lower().is_err());
        assert!(nix::import(&source).is_err());
    }
    let source = format!("''{}''", "x".repeat(MAX_SOURCE_BYTES - 4));
    roundtrip(&source, &source);
    assert!(parse_nxc(&format!("{source} ")).is_err());
    assert!(nix::import(&format!("{source} ")).is_err());
}

#[test]
fn native_nix_confirms_indentation_escapes_and_lazy_interpolation() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(output) => assert!(output.status.success()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(e) => panic!("cannot start Nix: {e}"),
    }
    let mut cases: Vec<_> = literal_cases()
        .into_iter()
        .map(|(s, v)| (s, Some(v)))
        .collect();
    cases.extend([
        ("''a'${\"X\"}''", Some("a'X")),
        ("''\n  a\n  ${x}\n    b\n''", Some("a\nX\n  b\n")),
        ("''  ${x}  ''", Some("X  ")),
        ("''$$${x}''", Some("$$X")),
        ("''${''inner ${x}''}''", Some("inner X")),
        ("{ a = ''ok''; unused = ''${1 / 0}''; }.a", Some("ok")),
        ("{ a = ''ok''; }.a or ''${1 / 0}''", Some("ok")),
        ("''${{ __toString = self: ''ok''; }}''", Some("ok")),
        ("''${1}''", None),
        ("''${null}''", None),
        ("''${1 / 0}''", None),
    ]);
    for (source, expected) in cases {
        let ir = nix::import(source).unwrap_or_else(|e| panic!("{source:?}: {e:?}"));
        let generated = nix::emit(&parse_nxc(&emit::nxc(&ir).unwrap()).unwrap()).unwrap();
        for value in [source, generated.as_str()] {
            let expression = format!("let x = \"X\"; in {value}");
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
            if let Some(expected) = expected {
                assert!(
                    output.status.success(),
                    "{value:?}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let quoted = nix::emit(&Expr::String(if expected.is_empty() {
                    vec![]
                } else {
                    vec![StringPart::Literal(expected.into())]
                }))
                .unwrap();
                // Nix's JSON output has standard escaping; obtain that from a
                // plain string whose value was specified independently above.
                let expected_output = Command::new("nix-instantiate")
                    .args([
                        "--store", "dummy://", "--eval", "--strict", "--json", "--expr", &quoted,
                    ])
                    .output()
                    .unwrap();
                assert!(expected_output.status.success());
                assert_eq!(output.stdout, expected_output.stdout, "{value:?}");
            } else {
                assert!(!output.status.success(), "{value:?} must fail");
            }
        }
    }
}
