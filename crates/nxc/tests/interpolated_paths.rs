mod support;
use support::nxc;

use nxc::{
    MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, emit,
    ir::{Expr, StringPart},
    nix, parse_nxc, syntax,
};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) -> String {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let ir = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    assert_eq!(
        nix::import(native).unwrap_or_else(|e| panic!("{native}: {e:?}")),
        ir,
        "{native}"
    );
    let converted = emit::nxc(&ir).unwrap();
    let reparsed = parse_nxc(&converted).unwrap();
    assert_eq!(reparsed, ir);
    let generated = nix::emit(&reparsed).unwrap();
    assert_eq!(nix::import(&generated).unwrap(), ir, "{generated}");
    assert_eq!(emit::nxc(&reparsed).unwrap(), converted);
    generated
}

#[test]
fn interpolated_paths_preserve_fragments_and_expression_boundaries() {
    for source in [
        "./${x}",
        "/${x}",
        "~/${x}",
        "dir/${x}/default.nix",
        "./a${x}b${y}/c",
        "./${x}${y}",
        "../a/../${x}/./b",
        "/a/../${x}",
        "~/a/../${x}",
        "./${x}.a",
        "./${x}+",
        "./${x}-1",
        "./${x} / 2",
        "./${x} + ./a",
        "-./${x}",
        "!./${x}",
        "(./${x}).a",
        "s.a or ./${x}",
        "./${x}?a",
        "s.${./${x}}",
        "s ? ${./${x}}",
        "{ x = ./${y}; }",
        "[./${x}] ++ [./${y}]",
        r#"./${"${x}"}/a"#,
        r#""${./${x}}""#,
        "''${./${x}}''",
        r#"./${{ x = "a"; }.x}/b"#,
        "./${/* } */ x}/a",
        "./${x # }\n}/a",
        "./${./${x}}/a",
        "./${~/a${x}}",
        "./${x} /* c */ + ./a",
    ] {
        roundtrip(source, source);
    }
    for (source, native) in [
        ("f(./${x}, /${y})", "f ./${x} /${y}"),
        ("[./${x}, ~/${y}]", "[./${x} ~/${y}]"),
        ("./${x}(~/${y})", "./${x} ~/${y}"),
        (
            "let { x = \"a\"; yield ./${x}; }",
            "let x = \"a\"; in ./${x}",
        ),
        ("./${f(x)}", "./${f x}"),
        ("./${x => x}", "./${x: x}"),
        ("fn({ x ? ./${y} }) => x", "{ x ? ./${y} }: x"),
        ("assert(true, ./${x})", "assert true; ./${x}"),
        ("with({}, ./${x})", "with {}; ./${x}"),
        ("./${x // }\n}/a", "./${x # }\n}/a"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn invalid_interpolated_paths_are_lossless_and_recover_later_items() {
    for source in [
        "./${}",
        "./${x",
        "./${x}/",
        "./${x}/*c*/",
        "./${x}// comment",
        "foo${x}/a",
        "./${x}~/${y}",
        "~user/${x}",
        "~//${x}",
        "./${x} ${y}",
        "./${x} /* gap */ ${y}",
        "./${x}/é",
        "./${x}/a\0",
        "./${__nxc_unsupported}",
        // Empty literal components remain outside the supported path subset.
        "./a//${x}",
        "./${x}//${y}",
        "./${x}//a",
        ".../${x}",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted nxc {source}");
        assert!(nix::import(source).is_err(), "accepted native {source}");
    }
    for source in [
        "f(./${}, good(1))",
        "[./${x}/, good(1)]",
        "{ x = ./${}; y = good(1); }",
        "f(./${{ x = ; }}, good(1))",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(
            root.descendants()
                .any(|n| n.kind() == syntax::SyntaxKind::CallExpr && n.text() == "good(1)"),
            "{source}: {root:#?}"
        );
    }
}

#[test]
fn native_nix_confirms_interpolated_path_values_and_errors() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(e) => panic!("cannot start Nix: {e}"),
    }
    for (source, expected) in [
        (r#"builtins.typeOf /a${"b"}"#, Some(r#""path""#)),
        (r#"toString /a/${"b"}/../c"#, Some(r#""/a/c""#)),
        (r#"toString /a/../${"b"}"#, Some(r#""/b""#)),
        (r#"toString /${""}"#, Some(r#""/""#)),
        (r#"toString /a${""}b"#, Some(r#""/ab""#)),
        (r#"toString /a/${"x/y"}"#, Some(r#""/a/x/y""#)),
        (r#"toString /a/${"x y"}"#, Some(r#""/a/x y""#)),
        (r#"toString /a/${"é"}"#, Some(r#""/a/é""#)),
        (r#"toString /a/${"/b"}"#, Some(r#""/a/b""#)),
        (r#"toString /a/${/b}"#, Some(r#""/a/b""#)),
        (
            r#"toString /a/${{ __toString = _: "b"; }}"#,
            Some(r#""/a/b""#),
        ),
        (r#"toString /a/${{ outPath = "b"; }}"#, Some(r#""/a/b""#)),
        (r#"toString ~/a/../${"b"}"#, Some(r#""/nxc-path-home/b""#)),
        (r#"builtins.hasContext (toString /a/${/b})"#, Some("false")),
        (r#"if true then 7 else /a/${abort "unused"}"#, Some("7")),
        (r#"builtins.head [ 1 /a/${abort "unused"} ]"#, Some("1")),
        (r#"({ x ? /a/${abort "unused"} }: 1) {}"#, Some("1")),
        (r#"toString /a/${1}"#, None),
        (r#"toString /a/${null}"#, None),
        (r#"toString /a/${true}"#, None),
        (r#"toString /a/${[ "b" ]}"#, None),
        (
            r#"toString /a/${builtins.appendContext "b" { "${builtins.storeDir}/00000000000000000000000000000000-ctx" = { path = true; }; }}"#,
            None,
        ),
    ] {
        let ir = nix::import(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let generated = nix::emit(&parse_nxc(&emit::nxc(&ir).unwrap()).unwrap()).unwrap();
        for value in [source, &generated] {
            let output = Command::new("nix-instantiate")
                .env("HOME", "/nxc-path-home")
                .args([
                    "--store", "dummy://", "--eval", "--strict", "--json", "--expr",
                ])
                .arg(format!("({value})"))
                .output()
                .unwrap();
            if let Some(expected) = expected {
                assert!(output.status.success(), "{value}: {output:?}");
                assert_eq!(
                    String::from_utf8(output.stdout).unwrap().trim(),
                    expected,
                    "{value}"
                );
            } else {
                assert!(!output.status.success(), "{value}: {output:?}");
                if source.contains("appendContext") {
                    assert!(
                        String::from_utf8(output.stderr).unwrap().contains(
                            "a string that refers to a store path cannot be appended to a path"
                        ),
                        "{value}"
                    );
                }
            }
        }
    }
}

#[test]
fn public_path_ir_preserves_interpolations_and_enforces_resource_limits() {
    let literal = |s: &str| StringPart::Literal(s.into());
    let variable = StringPart::Interpolation(Expr::Variable("x".into()));
    let ir = Expr::InterpolatedPath(vec![literal("~/a/../"), variable.clone(), literal("/b")]);
    assert_eq!(parse_nxc("~/a/../${x}/b").unwrap(), ir);
    assert_eq!(nix::import("~/a/../${x}/b").unwrap(), ir);
    // A string literal inside ${...} still triggers runtime path coercion and
    // normalization. It must not collapse into a plain path literal.
    assert_ne!(
        nix::import(r#"~/a/../${"b"}"#).unwrap(),
        nix::import("~/a/../b").unwrap()
    );
    for parts in [
        vec![],
        vec![literal("./a")],
        vec![variable.clone()],
        vec![variable.clone(), literal("/a")],
        vec![literal("foo"), variable.clone(), literal("/bar")],
        vec![literal("./"), literal("a"), variable.clone()],
        vec![literal("./"), variable.clone(), literal("")],
        vec![literal("./"), variable.clone(), literal("/")],
        vec![literal("./"), variable.clone(), literal("//a")],
        vec![literal("./"), variable.clone(), literal("${x}")],
        vec![literal("./"), variable.clone(), literal("/a); abort \"x\"")],
        vec![literal("~//"), variable.clone()],
        vec![literal(".../"), variable.clone()],
        vec![
            literal("./"),
            StringPart::Interpolation(Expr::Variable("__curPos".into())),
        ],
    ] {
        let invalid = Expr::InterpolatedPath(parts);
        assert!(emit::nxc(&invalid).is_err(), "{invalid:?}");
        assert!(nix::emit(&invalid).is_err(), "{invalid:?}");
    }
    // Parentheses, ./ and ${x} use eight bytes in either output dialect.
    let prefix = format!("./{}", "a".repeat(MAX_SOURCE_BYTES - 8));
    let exact = Expr::InterpolatedPath(vec![StringPart::Literal(prefix.clone()), variable.clone()]);
    for output in [emit::nxc(&exact).unwrap(), nix::emit(&exact).unwrap()] {
        assert_eq!(output.len(), MAX_SOURCE_BYTES);
        assert_eq!(parse_nxc(&output).unwrap(), exact);
        assert_eq!(nix::import(&output).unwrap(), exact);
    }
    let too_large =
        Expr::InterpolatedPath(vec![StringPart::Literal(prefix + "a"), variable.clone()]);
    assert!(emit::nxc(&too_large).is_err());
    assert!(nix::emit(&too_large).is_err());
    let aggregate = Expr::List(vec![exact, Expr::String(vec![literal("123456789")])]);
    assert!(emit::nxc(&aggregate).is_err());
    assert!(nix::emit(&aggregate).is_err());
    let source = format!("./{}", "${x}".repeat((MAX_TOKENS - 1) / 3));
    assert!(parse_nxc(&source).is_ok());
    assert!(nix::import(&source).is_ok());
    for source in [
        format!("{source}${{x}}"),
        format!("./{}${{x}}", "a".repeat(MAX_SOURCE_BYTES)),
    ] {
        assert!(parse_nxc(&source).is_err());
        assert!(nix::import(&source).is_err());
    }
    let huge_parts = Expr::InterpolatedPath(vec![variable; MAX_TOKENS + 1]);
    assert!(emit::nxc(&huge_parts).is_err());
    assert!(nix::emit(&huge_parts).is_err());
    let nested =
        (0..(MAX_DEPTH - 1) / 2).fold("x".to_owned(), |value, _| format!("./${{{value}}}"));
    roundtrip(&nested, &nested);
    let deepest = (0..MAX_DEPTH - 1).fold("x".to_owned(), |value, _| format!("./${{{value}}}"));
    assert!(parse_nxc(&deepest).is_ok());
    assert!(nix::import(&deepest).is_ok());
    let too_deep = (0..MAX_DEPTH + 1).fold("x".to_owned(), |value, _| format!("./${{{value}}}"));
    let parsed = syntax::parse(&too_deep);
    assert_eq!(parsed.syntax().unwrap().to_string(), too_deep);
    assert!(parsed.lower().is_err());
    assert!(nix::import(&too_deep).is_err());
}
