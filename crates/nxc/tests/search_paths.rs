mod support;
use support::nxc;

use nxc::{emit, nix, parse_nxc, syntax};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let expected = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    assert_eq!(nix::import(native).unwrap(), expected, "{native}");
    let nxc = emit::nxc(&expected).unwrap();
    let reparsed = parse_nxc(&nxc).unwrap();
    assert_eq!(reparsed, expected, "{nxc}");
    let generated = nix::emit(&reparsed).unwrap();
    assert_eq!(nix::import(&generated).unwrap(), expected, "{generated}");
    assert_eq!(emit::nxc(&reparsed).unwrap(), nxc);
}

fn nix_available() -> bool {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(output) => {
            assert!(output.status.success());
            true
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            false
        }
        Err(e) => panic!("cannot start Nix: {e}"),
    }
}

#[test]
fn search_paths_preserve_names_and_compose_with_expressions() {
    for source in [
        "<nixpkgs>",
        "<nixpkgs/lib>",
        "<.>",
        "<..>",
        "<...>",
        "<a/../b>",
        "<+/-/0/_/.>",
        "<fn/yield/if>",
        "<__nxc_name>",
        "<a>+<b>",
        "-<a>",
        "!<a>",
        "<a>.b or <fallback>",
        "<a>?b",
        "<a> < <b>",
        "<a> <= <b>",
        "<a> > <b>",
        "<a> >= <b>",
        "<a>==<b>",
        "<a> / 2",
        "if true then <a> else <b>",
        "{ x = <a>; ${<key>} = <b>; }",
        "\"${<a>}\"",
        "''${<a>}''",
        "1 < 2",
        "1 <= 2",
        "2 > 1",
        "2 >= 1",
    ] {
        roundtrip(source, source);
    }
    for (source, native) in [
        ("import(<nixpkgs>)", "import <nixpkgs>"),
        ("f(<a>, <b>)", "f <a> <b>"),
        ("[<a>, <b>]", "[<a> <b>]"),
        ("fn(x) => <a>", "x: <a>"),
        ("fn({ x ? <a> }) => x", "{ x ? <a> }: x"),
        ("let { x = <a>; yield x; }", "let x = <a>; in x"),
        ("with(<a>, 1)", "with <a>; 1"),
        ("assert(true, <a>)", "assert true; <a>"),
        ("<a> // comment\n + <b>", "<a> # comment\n + <b>"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn caller_search_paths_require_valid_literal_spelling() {
    use nxc::ir::Expr;
    for path in ["<nixpkgs>", "<a/../b>", "<a/./b>", "<...>", "<+/->"] {
        let expected = Expr::SearchPath(path.into());
        assert_eq!(parse_nxc(path).unwrap(), expected);
        assert_eq!(nix::import(path).unwrap(), expected);
        assert_eq!(parse_nxc(&emit::nxc(&expected).unwrap()).unwrap(), expected);
        assert_eq!(
            nix::import(&nix::emit(&expected).unwrap()).unwrap(),
            expected
        );
    }
    assert_ne!(parse_nxc("<a/../b>").unwrap(), parse_nxc("<b>").unwrap());
    for path in [
        "", "nixpkgs", "<>つ", "<>", "</a>", "<a/>", "<a//b>", "<a b>", "<a'b>", "<é>", "<a${b}>",
        "<a> + 1", "<<a>>", "<a\0b>",
    ] {
        let expr = Expr::SearchPath(path.into());
        assert!(emit::nxc(&expr).is_err(), "{path}");
        assert!(nix::emit(&expr).is_err(), "{path}");
    }
}

#[test]
fn search_paths_obey_source_node_depth_and_output_limits() {
    use nxc::{MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, ir::Expr};
    let path = format!("<{}>", "a".repeat(MAX_SOURCE_BYTES - 2));
    assert_eq!(parse_nxc(&path).unwrap(), Expr::SearchPath(path.clone()));
    assert_eq!(nix::import(&path).unwrap(), Expr::SearchPath(path.clone()));
    let oversized = format!("<a{}>", "a".repeat(MAX_SOURCE_BYTES - 2));
    assert!(parse_nxc(&oversized).is_err());
    assert!(nix::import(&oversized).is_err());
    assert!(emit::nxc(&Expr::SearchPath(oversized)).is_err());
    // Parentheses emitted around each lookup count against output size too.
    assert!(emit::nxc(&Expr::SearchPath(path.clone())).is_err());
    assert!(nix::emit(&Expr::SearchPath(path)).is_err());
    let fits = format!("<{}>", "a".repeat(MAX_SOURCE_BYTES - 4));
    roundtrip(&fits, &fits);
    let aggregate = Expr::List(vec![Expr::SearchPath(fits); 2]);
    assert!(emit::nxc(&aggregate).is_err());
    assert!(nix::emit(&aggregate).is_err());

    let tokens = format!("[{}]", "<a> ".repeat(MAX_TOKENS - 2));
    assert_eq!(parse_nxc(&tokens).unwrap(), nix::import(&tokens).unwrap());
    let over = tokens.replacen(']', "<a>]", 1);
    let parsed = syntax::parse(&over);
    assert_eq!(parsed.syntax().unwrap().to_string(), over);
    assert_eq!(parsed.diagnostics().len(), 1);
    assert!(nix::import(&over).is_err());
    let too_many_nodes = Expr::List(vec![Expr::SearchPath("<a>".into()); MAX_TOKENS]);
    assert!(emit::nxc(&too_many_nodes).is_err());
    assert!(nix::emit(&too_many_nodes).is_err());

    let nested = format!("{}<a>", "-".repeat(MAX_DEPTH - 1));
    roundtrip(&nested, &nested);
    assert!(parse_nxc(&format!("-{nested}")).is_err());
    assert!(nix::import(&format!("-{nested}")).is_err());
}

#[test]
fn malformed_search_paths_are_lossless_and_recover_at_item_boundaries() {
    for source in [
        "<>", "<a/>", "</a>", "<a//b>", "<a b>", "<a'b>", "<é>", "<a:${x}>", "<a${x}>", "<a",
        "<a\\b>", "<a\0b>",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted nxc {source}");
        assert!(nix::import(source).is_err(), "accepted native {source}");
    }
    for source in [
        "f(<bad/>, good(<ok>))",
        "[<bad/>, good(<ok>)]",
        "{ x = <bad/>; y = good(<ok>); }",
        "f(<bad, good(<ok>))",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(
            root.descendants()
                .any(|n| n.kind() == syntax::SyntaxKind::CallExpr && n.text() == "good(<ok>)"),
            "{source}: {root:#?}"
        );
    }
}

#[test]
fn native_nix_confirms_lookup_scope_verbatim_names_and_laziness() {
    if !nix_available() {
        return;
    }
    for (source, expected) in [
        (
            "let __findFile = p: n: [ p n ]; __nixPath = [ \"local\" ]; in <a/../b>",
            Some("[[\"local\"],\"a/../b\"]"),
        ),
        (
            "let __findFile = p: n: n; __nixPath = abort \"unused\"; in <x>",
            Some("\"x\""),
        ),
        (
            "let builtins = {}; __findFile = p: n: n; in <x>",
            Some("\"x\""),
        ),
        (
            "({ __findFile ? p: n: n, __nixPath ? [] }: <name>) {}",
            Some("\"name\""),
        ),
        (
            "let __findFile = p: n: p; in rec { __nixPath = 3; x = <x>; }.x",
            Some("3"),
        ),
        ("if true then 7 else <nxc-search-missing>", Some("7")),
        ("with <nxc-search-missing>; 1", Some("1")),
        ("({ x ? <nxc-search-missing> }: 1) {}", Some("1")),
        ("builtins.head [ 1 <nxc-search-missing> ]", Some("1")),
        ("let __nixPath = []; in <nxc-search-missing>", None),
        (
            "with { __findFile = p: n: 1; __nixPath = []; }; <nxc-search-missing>",
            None,
        ),
    ] {
        let ir = nix::import(source).unwrap();
        let converted = parse_nxc(&emit::nxc(&ir).unwrap()).unwrap();
        let generated = nix::emit(&converted).unwrap();
        assert_eq!(converted, ir);
        assert_eq!(nix::import(&generated).unwrap(), ir);
        for value in [source, &generated] {
            let output = Command::new("nix-instantiate")
                .env("NIX_PATH", "")
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
