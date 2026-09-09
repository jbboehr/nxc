mod support;
use support::nxc;

use nxc::{MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, emit, ir::Expr, nix, parse_nxc, syntax};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let ir = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    assert_eq!(nix::import(native).unwrap(), ir, "{native}");
    let nxc = emit::nxc(&ir).unwrap();
    let converted = parse_nxc(&nxc).unwrap();
    assert_eq!(converted, ir);
    let generated = nix::emit(&converted).unwrap();
    assert_eq!(nix::import(&generated).unwrap(), ir, "{generated}");
    assert_eq!(emit::nxc(&converted).unwrap(), nxc);
}

#[test]
fn home_paths_preserve_spelling_and_expression_boundaries() {
    for source in [
        "~/.",
        "~/..",
        "~/a/../b",
        "~/a/./b",
        "~/...",
        "~/.hidden",
        "~/+/-/0/_/...",
        "~/fn/yield/if",
        "~/__nxc_name",
        "~/1.0e+2",
        "~/a-",
        "~/a+",
        "~/a++",
        "~/a+/b",
        "~/a++/b",
        "~/a.b",
        "~/a . x",
        "(~/a).x",
        "s.a or ~/fallback",
        "~/a?b",
        "~/a / 2",
        "~/a / ~/b",
        "~/a + ~/b",
        "~/a == ~/b",
        "~/a < ~/b",
        "!~/a",
        "-~/a",
        "s.${~/key}",
        "s ? ${~/key}",
        "false && ~/a",
        "[~/a] ++ [~/b]",
        "if true then ~/a else ~/b",
        "{ x = ~/a; ${~/key} = ~/b; }",
        "\"${~/a}\"",
        "''${~/a}''",
        "~/a /* comment */ + ~/b",
        "~/a#comment\n",
    ] {
        roundtrip(source, source);
    }
    for (source, native) in [
        ("import(~/missing.nix)", "import ~/missing.nix"),
        ("1(~/a)", "1~/a"),
        ("f(~/a)", "f~/a"),
        ("~/a+(~/b)", "~/a+~/b"),
        ("~/a-(~/b)", "~/a-~/b"),
        ("<x>(~/a)", "<x>~/a"),
        ("f(~/a, ~/b)", "f ~/a ~/b"),
        ("[~/a, ~/b]", "[~/a ~/b]"),
        ("fn({ x ? ~/a }) => x", "{ x ? ~/a }: x"),
        ("let { x = ~/a; yield x; }", "let x = ~/a; in x"),
        ("with(~/a, 1)", "with ~/a; 1"),
        ("assert(true, ~/a)", "assert true; ~/a"),
        ("__nxc_update({}, ~/a)", "{} // ~/a"),
        ("~/a // comment\n + ~/b", "~/a # comment\n + ~/b"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn malformed_and_interpolated_home_paths_fail_losslessly_and_recover() {
    for source in [
        "~",
        "~/",
        "~//a",
        "~user/a",
        "~/a/",
        "~/a//b",
        "~/a/*comment*/",
        "~/a\\b",
        "~/é",
        "~/a${x}/",
        "~/${x}/",
        "~/a\0",
        "~/a'b",
        "+~/a",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted nxc {source}");
        assert!(nix::import(source).is_err(), "accepted native {source}");
    }
    for source in [
        "f(~/, good(1))",
        "[~/bad/, good(1)]",
        "{ x = ~/bad/; y = good(1); }",
        "f(~/bad//path, good(1))",
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
fn public_ir_preserves_home_path_spelling_and_enforces_limits() {
    for path in ["~/.", "~/..", "~/a/../b", "~/a/./b", "~/+/-/_/..."] {
        let ir = Expr::HomePath(path.into());
        assert_eq!(parse_nxc(path).unwrap(), ir);
        assert_eq!(nix::import(path).unwrap(), ir);
        assert_eq!(parse_nxc(&emit::nxc(&ir).unwrap()).unwrap(), ir);
        assert_eq!(nix::import(&nix::emit(&ir).unwrap()).unwrap(), ir);
    }
    assert_ne!(parse_nxc("~/a/../b").unwrap(), parse_nxc("~/b").unwrap());
    for path in [
        "",
        "~",
        "~/",
        "/a",
        "./a",
        "a/b",
        "<a>",
        "~user/a",
        "~//a",
        "~/a/",
        "~/a//b",
        "~/a${x}",
        "~/a b",
        "~/a\0",
        "~/é",
        "~/a#comment",
        "~/a/*comment*/",
        "~/a); abort \"x\"",
    ] {
        let ir = Expr::HomePath(path.into());
        assert!(emit::nxc(&ir).is_err(), "accepted {path:?}");
        assert!(nix::emit(&ir).is_err(), "accepted {path:?}");
    }
    let path = format!("~/{}", "a".repeat(MAX_SOURCE_BYTES - 4));
    let ir = Expr::HomePath(path.clone());
    for output in [emit::nxc(&ir).unwrap(), nix::emit(&ir).unwrap()] {
        assert_eq!(output.len(), MAX_SOURCE_BYTES);
        assert_eq!(parse_nxc(&output).unwrap(), ir);
        assert_eq!(nix::import(&output).unwrap(), ir);
    }
    let too_large = Expr::HomePath(format!("{path}a"));
    assert!(emit::nxc(&too_large).is_err());
    assert!(nix::emit(&too_large).is_err());
    let too_large = format!("{path}aaa");
    assert!(syntax::parse(&too_large).syntax().is_none());
    assert!(nix::import(&too_large).is_err());
    let combined = Expr::List(vec![ir, Expr::AbsolutePath("/ab".into())]);
    assert!(emit::nxc(&combined).is_err());
    assert!(nix::emit(&combined).is_err());

    let path = Expr::HomePath("~/a".into());
    let mut items = vec![path.clone(); (MAX_TOKENS - 1) / 4];
    // Each path plus comma contributes four tokens; the final empty list
    // fills three more, reaching the nxc ceiling with the outer brackets.
    items.push(Expr::List(vec![]));
    let exact = Expr::List(items);
    assert_eq!(parse_nxc(&emit::nxc(&exact).unwrap()).unwrap(), exact);
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
fn native_nix_confirms_home_path_values_and_pure_mode_rejection() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(e) => panic!("cannot start Nix: {e}"),
    }
    for (source, expected) in [
        ("builtins.typeOf ~/missing", Some("\"path\"")),
        ("toString ~/.", Some("\"/nxc-home-fixture/.\"")),
        ("toString ~/..", Some("\"/nxc-home-fixture/..\"")),
        ("toString ~/a/../b", Some("\"/nxc-home-fixture/a/../b\"")),
        ("toString ~/a/./b", Some("\"/nxc-home-fixture/a/./b\"")),
        ("~/a/../b == ~/b", Some("false")),
        ("~/. == /nxc-home-fixture", Some("false")),
        (
            "toString (~/a/../b + \"\")",
            Some("\"/nxc-home-fixture/b\""),
        ),
        ("builtins.baseNameOf ~/a/b.nix", Some("\"b.nix\"")),
        (
            "toString (builtins.dirOf ~/a/../b)",
            Some("\"/nxc-home-fixture/a/..\""),
        ),
        ("builtins.hasContext (toString ~/missing)", Some("false")),
        ("if true then 7 else import ~/missing", Some("7")),
        ("with ~/missing; 1", Some("1")),
        ("false -> ~/missing", Some("true")),
        ("({ x ? ~/missing }: 1) {}", Some("1")),
        ("builtins.head [ 1 ~/missing ]", Some("1")),
        ("1~/a", None),
        ("-~/a", None),
        ("~/a / 2", None),
        ("assert ~/a; 1", None),
    ] {
        let ir = nix::import(source).unwrap();
        let converted = parse_nxc(&emit::nxc(&ir).unwrap()).unwrap();
        assert_eq!(converted, ir);
        let generated = nix::emit(&converted).unwrap();
        assert_eq!(nix::import(&generated).unwrap(), ir);
        for value in [source, &generated] {
            for pure in [false, true] {
                let mut command = Command::new("nix-instantiate");
                command
                    .env("HOME", "/nxc-home-fixture")
                    .args(["--store", "dummy://", "--eval", "--strict", "--json"]);
                if pure {
                    command.arg("--pure-eval");
                }
                let output = command
                    .arg("--expr")
                    .arg(format!("({value})"))
                    .output()
                    .unwrap();
                if pure {
                    assert!(!output.status.success(), "{value}");
                    assert!(
                        String::from_utf8(output.stderr)
                            .unwrap()
                            .contains("can not be resolved in pure mode")
                    );
                } else if let Some(expected) = expected {
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
}
