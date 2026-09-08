use nxc::{MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, emit, ir::Expr, nix, parse_nxc, syntax};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let expected = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    assert_eq!(nix::import(native).unwrap(), expected, "{native}");
    let nxc = emit::nxc(&expected).unwrap();
    let reparsed = parse_nxc(&nxc).unwrap();
    assert_eq!(reparsed, expected);
    let emitted = nix::emit(&reparsed).unwrap();
    assert_eq!(nix::import(&emitted).unwrap(), expected, "{emitted}");
    assert_eq!(emit::nxc(&reparsed).unwrap(), nxc);
}

#[test]
fn absolute_paths_preserve_spelling_and_expression_boundaries() {
    for source in [
        "/.",
        "/..",
        "/a/../b",
        "/a/./b",
        "/a.b",
        "/.hidden",
        "/-",
        "/+",
        "/a-",
        "/a+",
        "/a--",
        "/a++",
        "/a_0-+.b",
        "/+/-/0/_/...",
        "/fn/yield/if",
        "/__nxc_name",
        "/1.0e+2",
        "/a+/b",
        "/a++/b",
        "/a . x",
        "(/a).x",
        "s.a or /fallback",
        "/a?b",
        "/a / 2",
        "/a / /b",
        "/a + /b",
        "/a == /b",
        "/a < /b",
        "! /a",
        "- /a",
        "s.${/key}",
        "s ? ${/key}",
        "false && /a",
        "[/a] ++ [/b]",
        "if true then /a else /b",
        "{ x = /a; ${/key} = /b; }",
        "\"${/a}\"",
        "''${/a}''",
        "/a /* comment */ + /b",
        "/a#comment\n",
        "1 / 2",
        "-/a",
        "+/a",
    ] {
        roundtrip(source, source);
    }
    for (source, native) in [
        ("import(/missing/module.nix)", "import /missing/module.nix"),
        ("1(/2)", "1 /2"),
        ("/a(/b)", "/a /b"),
        ("/a-(/b)", "/a- /b"),
        ("/a+(/b)", "/a+ /b"),
        ("<x>(/2)", "<x>/2"),
        ("f(/a, /b)", "f /a /b"),
        ("[/a, /b]", "[/a /b]"),
        ("fn({ x ? /a }) => x", "{ x ? /a }: x"),
        ("let { x = /a; yield x; }", "let x = /a; in x"),
        ("with(/a, 1)", "with /a; 1"),
        ("assert(true, /a)", "assert true; /a"),
        ("__nxc_update({}, /a)", "{} // /a"),
        ("/a // comment\n + /b", "/a # comment\n + /b"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn malformed_and_deferred_paths_fail_losslessly_and_recover() {
    for source in [
        "/",
        "/a/",
        "/a//b",
        "/a/*comment*/",
        "/a\\b",
        "/é",
        "/a${x}/",
        "/${x}/",
        "~/a/",
        "/a\0",
        "/a'b",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted nxc {source}");
        assert!(nix::import(source).is_err(), "accepted native {source}");
    }
    for source in [
        "f(/bad/, good(/ok))",
        "[/bad/, good(/ok)]",
        "{ x = /bad/; y = good(/ok); }",
        "f(/bad//path, good(/ok))",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(
            root.descendants()
                .any(|n| n.kind() == syntax::SyntaxKind::CallExpr && n.text() == "good(/ok)"),
            "{source}: {root:#?}"
        );
    }
    // C/Rust-style line comments still begin with // outside a path token.
    assert_eq!(parse_nxc("1// comment\n").unwrap(), parse_nxc("1").unwrap());
}

#[test]
fn public_ir_preserves_literal_spelling_and_enforces_resource_limits() {
    for path in ["/.", "/..", "/a/../b", "/a/./b", "/+/-/_/..."] {
        let ir = Expr::AbsolutePath(path.into());
        assert_eq!(parse_nxc(path).unwrap(), ir);
        assert_eq!(nix::import(path).unwrap(), ir);
        assert_eq!(parse_nxc(&emit::nxc(&ir).unwrap()).unwrap(), ir);
        assert_eq!(nix::import(&nix::emit(&ir).unwrap()).unwrap(), ir);
    }
    assert_ne!(parse_nxc("/a/../b").unwrap(), parse_nxc("/b").unwrap());
    for path in [
        "",
        "/",
        "a/b",
        "./a",
        "~/a",
        "<a>",
        "//a",
        "/a/",
        "/a//b",
        "/a${x}",
        "/a b",
        "/a\0",
        "/é",
        "/a#comment",
        "/a/*comment*/",
        "/a); abort \"x\"",
    ] {
        let ir = Expr::AbsolutePath(path.into());
        assert!(emit::nxc(&ir).is_err(), "accepted {path:?}");
        assert!(nix::emit(&ir).is_err(), "accepted {path:?}");
    }
    let path = format!("/{}", "a".repeat(MAX_SOURCE_BYTES - 3));
    let ir = Expr::AbsolutePath(path.clone());
    for output in [emit::nxc(&ir).unwrap(), nix::emit(&ir).unwrap()] {
        assert_eq!(output.len(), MAX_SOURCE_BYTES);
        assert_eq!(parse_nxc(&output).unwrap(), ir);
        assert_eq!(nix::import(&output).unwrap(), ir);
    }
    let too_large = Expr::AbsolutePath(format!("{path}a"));
    assert!(emit::nxc(&too_large).is_err());
    assert!(nix::emit(&too_large).is_err());
    let too_large = format!("{path}aaa");
    assert!(syntax::parse(&too_large).syntax().is_none());
    assert!(nix::import(&too_large).is_err());
    let combined = Expr::List(vec![ir, Expr::RelativePath("./b".into())]);
    assert!(emit::nxc(&combined).is_err());
    assert!(nix::emit(&combined).is_err());

    let path = Expr::AbsolutePath("/a".into());
    let mut items = vec![path.clone(); (MAX_TOKENS - 1) / 4];
    // Parenthesized path plus comma: four tokens. The final empty list fills
    // three more, reaching the nxc output ceiling including the outer brackets.
    items.push(Expr::List(vec![]));
    let exact = Expr::List(items);
    let output = emit::nxc(&exact).unwrap();
    assert_eq!(parse_nxc(&output).unwrap(), exact);
    assert!(emit::nxc(&Expr::List(vec![path.clone(); (MAX_TOKENS - 1) / 4 + 1])).is_err());
    let mut items = vec![path.clone(); (MAX_TOKENS - 2) / 3];
    items.extend(vec![Expr::Integer(1); (MAX_TOKENS - 2) % 3]);
    let exact = Expr::List(items.clone());
    assert_eq!(nix::import(&nix::emit(&exact).unwrap()).unwrap(), exact);
    items.push(Expr::Integer(1));
    assert!(nix::emit(&Expr::List(items)).is_err());
    let nested = (0..MAX_DEPTH - 1).fold(path, |inner, _| Expr::Not(Box::new(inner)));
    roundtrip(&emit::nxc(&nested).unwrap(), &nix::emit(&nested).unwrap());
    assert!(emit::nxc(&Expr::Not(Box::new(nested))).is_err());
}

#[test]
fn native_nix_confirms_absolute_path_values_laziness_and_errors() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(e) => panic!("cannot start Nix: {e}"),
    }
    for (source, expected) in [
        ("builtins.typeOf /nxc-absolute-missing", Some("\"path\"")),
        ("toString /.", Some("\"/\"")),
        ("toString /..", Some("\"/\"")),
        ("toString /a/../b", Some("\"/b\"")),
        ("toString /a/./b", Some("\"/a/b\"")),
        ("builtins.baseNameOf /a/b.nix", Some("\"b.nix\"")),
        ("toString (builtins.dirOf /a/b)", Some("\"/a\"")),
        (
            "builtins.hasContext (toString /nxc-absolute-missing)",
            Some("false"),
        ),
        ("/a/../b == /b", Some("true")),
        ("/a == /b", Some("false")),
        ("toString (/a + \"/b\")", Some("\"/a/b\"")),
        (
            "if true then 7 else import /nxc-absolute-missing",
            Some("7"),
        ),
        ("with /nxc-absolute-missing; 1", Some("1")),
        ("false -> /nxc-absolute-missing", Some("true")),
        ("({ x ? /nxc-absolute-missing }: 1) {}", Some("1")),
        ("builtins.head [ 1 /nxc-absolute-missing ]", Some("1")),
        ("1 / 2", Some("0")),
        ("1 /2", None),
        ("- /a", None),
        ("/a / 2", None),
        ("assert /a; 1", None),
    ] {
        let ir = nix::import(source).unwrap();
        let converted = parse_nxc(&emit::nxc(&ir).unwrap()).unwrap();
        assert_eq!(converted, ir);
        let generated = nix::emit(&converted).unwrap();
        assert_eq!(nix::import(&generated).unwrap(), ir);
        for value in [source, &generated] {
            let output = Command::new("nix-instantiate")
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
                assert!(!output.status.success(), "{value}");
            }
        }
    }
}
