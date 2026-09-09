mod support;
use support::nxc;

use nxc::{MAX_DEPTH, MAX_TOKENS, emit, ir::Expr, nix, parse_nxc, syntax};
use std::{io::ErrorKind, process::Command};

fn roundtrip(native: &str, normalized: &str) {
    let expected = parse_nxc(normalized).unwrap_or_else(|e| panic!("{normalized}: {e:?}"));
    let actual = nix::import(native).unwrap_or_else(|e| panic!("{native}: {e:?}"));
    assert_eq!(actual.canonical(), expected.canonical(), "{native}");
    let converted = emit::nxc(&actual).unwrap();
    let reparsed = syntax::parse(&converted);
    assert_eq!(reparsed.syntax().unwrap().to_string(), converted);
    assert_eq!(reparsed.lower().unwrap(), expected);
    let generated = nix::emit(&actual).unwrap();
    assert_eq!(nix::import(&generated).unwrap(), expected);
    assert_eq!(emit::nxc(&expected).unwrap(), converted);
    assert_eq!(nix::emit(&expected).unwrap(), generated);
}

#[test]
fn implication_normalizes_without_folding_or_changing_lexical_boundaries() {
    for (native, normalized) in [
        ("a -> b", "(!a) || b"),
        ("a ->b", "(!a) || b"),
        ("(a)->b", "(!a) || b"),
        ("false -> true", "(!false) || true"),
        ("1->2", "(!1) || 2"),
        ("/* α */ a/* β */-># γ\nb", "(!a) || b"),
        // Hyphens are identifier characters in both dialects.
        ("a->b", "a- > b"),
        ("a-> b", "a- > b"),
    ] {
        roundtrip(native, normalized);
    }
    assert!(matches!(
        nix::import("false -> true").unwrap(),
        Expr::Binary { .. }
    ));
    assert_eq!(parse_nxc("a->b").unwrap(), parse_nxc("a- > b").unwrap());
}

#[test]
fn implication_preserves_grouping_and_composes_with_the_supported_subset() {
    for (native, normalized) in [
        ("a -> b -> c", "(!a) || ((!b) || c)"),
        ("(a -> b) -> c", "!((!a) || b) || c"),
        ("a || b -> c && d", "!(a || b) || (c && d)"),
        ("a && b -> c || d", "!(a && b) || (c || d)"),
        ("!a -> !b", "!(!a) || (!b)"),
        ("a == b -> c < d", "!(a == b) || (c < d)"),
        ("a + b * c -> -d", "!(a + b * c) || (-d)"),
        ("a ++ b -> c // d", "!(a ++ b) || __nxc_update(c, d)"),
        ("f a -> g b", "!(f(a)) || g(b)"),
        ("f (a -> b)", "f((!a) || b)"),
        ("(a -> b) c", "((!a) || b)(c)"),
        ("(a -> b).x", "((!a) || b).x"),
        ("s.a or a -> b", "!(s.a or a) || b"),
        ("s.a or (a -> b)", "s.a or ((!a) || b)"),
        ("[(a -> b) (c -> d)]", "[(!a) || b, (!c) || d]"),
        ("{ x = a -> b; }", "{ x = (!a) || b; }"),
        ("{ inherit (a -> b) x; }", "{ inherit ((!a) || b) x; }"),
        ("x: x -> a", "x => (!x) || a"),
        ("{ x ? a -> b }: x", "({ x ? (!a) || b }) => x"),
        ("a -> (x: x)", "(!a) || (x => x)"),
        ("a -> f (x: x)", "(!a) || f(x => x)"),
        ("if a -> b then c else d", "if (!a) || b then c else d"),
        ("a -> (if b then c else d)", "(!a) || (if b then c else d)"),
        ("assert a -> b; c", "assert((!a) || b, c)"),
        ("with s; a -> b", "with(s, (!a) || b)"),
        ("let x = a -> b; in x", "let { x = (!a) || b; yield x; }"),
        (
            r#""${if a -> b then "yes" else "no"}""#,
            r#""${if (!a) || b then "yes" else "no"}""#,
        ),
    ] {
        roundtrip(native, normalized);
    }
}

#[test]
fn implication_keeps_nxc_arrow_reserved_and_validates_unevaluated_operands() {
    roundtrip("false -> true", "(!false) || true");
    for source in [
        "a -> b",
        "a ->b",
        "1->2",
        "(a)->b",
        "fn(x) -> T => x",
        "[a -> b]",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
    }
    for source in [
        "false -> ./path${__nxc_unsupported}",
        "false -> __nxc_bad",
        "false -> 9223372036854775808",
        "true -> ./path${__nxc_unsupported}",
        "./path${__nxc_unsupported} -> true",
        "__nxc_unsupported -> true",
        "false -> 1.5e+",
        "false -> (a |> b)",
    ] {
        assert!(nix::import(source).is_err(), "accepted {source}");
    }
}

#[test]
fn implication_expansion_obeys_semantic_and_emission_limits() {
    // Each right-hand implication adds one level; its left-hand Not adds two.
    let right = format!("{}a", "a -> ".repeat(MAX_DEPTH - 2));
    let ir = nix::import(&right).unwrap();
    assert!(emit::nxc(&ir).is_ok());
    assert!(nix::emit(&ir).is_ok());
    assert!(nix::import(&format!("a -> {right}")).is_err());
    let left = (0..(MAX_DEPTH - 1) / 2).fold("a".to_owned(), |e, _| format!("({e} -> a)"));
    assert!(nix::import(&format!("!{left}")).is_ok());
    assert!(nix::import(&format!("({left} -> a)")).is_err());
    let longest = format!("{}a", "a -> ".repeat((MAX_TOKENS - 1) / 2));
    assert!(nix::import(&longest).is_err());
    assert!(nix::parse(&format!("a -> {longest}")).is_err());

    // Input has 5 tokens per item; normalized output has 8 plus nxc commas.
    let nxc_items = (MAX_TOKENS - 1) / 9;
    let native_items = (MAX_TOKENS - 2) / 8;
    for (items, nxc_ok, native_ok) in [
        (nxc_items, true, true),
        (nxc_items + 1, false, true),
        (native_items, false, true),
        (native_items + 1, false, false),
    ] {
        let source = format!("[{}]", "(a -> b) ".repeat(items));
        let ir = nix::import(&source).unwrap();
        assert_eq!(emit::nxc(&ir).is_ok(), nxc_ok, "{items} items");
        assert_eq!(nix::emit(&ir).is_ok(), native_ok, "{items} items");
    }
    // Reach exactly MAX_TOKENS and MAX_TOKENS + 1 in each target dialect.
    // Leave at least three bare items in nxc; replacing three with `a (!a)`
    // adds one output token. An odd implication count leaves an even remainder.
    let mut nxc_items = (MAX_TOKENS - 7) / 9;
    if nxc_items.is_multiple_of(2) {
        nxc_items -= 1;
    }
    let bare_items = (MAX_TOKENS - 1 - 9 * nxc_items) / 2;
    for (items, at_limit, over_limit, native) in [
        (
            nxc_items,
            "a ".repeat(bare_items),
            format!("{}a (!a)", "a ".repeat(bare_items - 3)),
            false,
        ),
        (
            native_items,
            "a ".repeat((MAX_TOKENS - 2) % 8),
            "a ".repeat((MAX_TOKENS - 2) % 8 + 1),
            true,
        ),
    ] {
        for (suffix, accepted) in [(at_limit, true), (over_limit, false)] {
            let source = format!("[{}{suffix}]", "(a -> b) ".repeat(items));
            let ir = nix::import(&source).unwrap();
            let output = if native {
                nix::emit(&ir)
            } else {
                emit::nxc(&ir)
            };
            assert_eq!(output.is_ok(), accepted, "native={native}, {suffix}");
            if let Ok(output) = output {
                assert_eq!(
                    syntax::lexer::lex(&output)
                        .iter()
                        .filter(|token| !token.kind.is_trivia())
                        .count(),
                    MAX_TOKENS
                );
            }
        }
    }
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

#[test]
fn native_implication_operand_boundaries_match_nix() {
    roundtrip("a -> b", "(!a) || b");
    if !nix_available() {
        return;
    }
    for source in [
        "a -> b -> c",
        "!a -> !b",
        "a -> b || c",
        "s.a or a -> b",
        "s.a or (a -> b)",
        "[(a -> b)]",
        "[a -> b]",
        "f (a -> b)",
        "a -> (x: x)",
        "(x: x) -> a",
        "a -> x: x",
        "a -> { x }: x",
        "a -> f x: x",
        "(f x: x) -> a",
        "a -> f (x: x)",
        "a -> s.a or x: x",
        "a -> s.a or (x: x)",
        "a -> if b then c else a",
        "a -> assert true; b",
        "a -> let in b",
        "-> a",
        "a ->",
        "a -> -> b",
        "a ->> b",
    ] {
        let wrapped = format!("a: b: c: f: s: ({source})");
        let output = Command::new("nix-instantiate")
            .args(["--store", "dummy://", "--parse", "--expr", &wrapped])
            .output()
            .unwrap();
        assert_eq!(
            nix::import(source).is_ok(),
            output.status.success(),
            "{source}"
        );
    }
}

#[test]
fn native_nix_confirms_implication_values_forcing_and_scope() {
    if !nix_available() {
        return;
    }
    for (source, expected) in [
        ("false -> false", Some("true")),
        ("false -> true", Some("true")),
        ("true -> false", Some("false")),
        ("true -> true", Some("true")),
        ("false -> true -> false", Some("true")),
        ("(false -> true) -> false", Some("false")),
        ("true || false -> false", Some("false")),
        ("false -> false || true", Some("true")),
        ("false -> (1 / 0)", Some("true")),
        ("false -> 1", Some("true")),
        ("false -> (x: x)", Some("true")),
        ("true -> 1", None),
        ("1 -> true", None),
        ("null -> false", None),
        ("[] -> false", None),
        ("true -> (assert false; true)", None),
        ("(assert false; false) -> true", None),
        ("false -> missing", None),
        ("(x: 1) (true -> 1)", Some("1")),
        ("builtins.head [1 (true -> 1)]", Some("1")),
        ("let false = true; in false -> false", Some("true")),
        ("let true = false; in true -> (1 / 0)", Some("true")),
        ("let builtins = 1; in false -> true", Some("true")),
        ("let x = false; in x -> (let x = true; in x)", Some("true")),
        ("let x = false -> x; in x", Some("true")),
        ("with { a = false; }; a -> (1 / 0)", Some("true")),
        (
            "({ a ? false -> true, b ? a -> false }: b) {}",
            Some("false"),
        ),
        ("assert 1 < 2 -> 3 == 3; 5", Some("5")),
        (
            r#""${if false -> (1 / 0) then "yes" else "no"}""#,
            Some(r#""yes""#),
        ),
        (
            "(builtins.tryEval ((builtins.throw \"left\") -> (builtins.abort \"right\"))).success",
            Some("false"),
        ),
        (
            "(builtins.tryEval ((builtins.abort \"left\") -> (builtins.throw \"right\"))).success",
            None,
        ),
    ] {
        let ir = nix::import(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let converted = emit::nxc(&ir).unwrap();
        let generated = nix::emit(&parse_nxc(&converted).unwrap()).unwrap();
        for value in [source, generated.as_str()] {
            let output = Command::new("nix-instantiate")
                .args([
                    "--store", "dummy://", "--eval", "--strict", "--json", "--expr", value,
                ])
                .output()
                .unwrap();
            if let Some(expected) = expected {
                assert!(
                    output.status.success(),
                    "{value}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(
                    String::from_utf8(output.stdout).unwrap().trim(),
                    expected,
                    "{value}"
                );
            } else {
                assert!(!output.status.success(), "{value} must fail");
            }
        }
    }
}
