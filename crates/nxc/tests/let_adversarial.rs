use nxc::{emit, nix, parse_nxc, syntax};
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

fn evaluate(source: &str) -> String {
    let output = Command::new("nix-instantiate")
        .args([
            "--store", "dummy://", "--eval", "--strict", "--json", "--expr", source,
        ])
        .output()
        .unwrap_or_else(|error| panic!("cannot evaluate {source}: {error}"));
    assert!(
        output.status.success(),
        "{source}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn literal_set_grouping_keeps_the_scope_that_defines_each_leaf() {
    if !nix_available() {
        return;
    }
    for (nxc_source, native_source, expected) in [
        (
            "let { c = 10; a.b = c; a = rec { c = 2; }; yield a.b; }",
            "let c = 10; a.b = c; a = rec { c = 2; }; in a.b",
            "10",
        ),
        (
            "let { c = 10; a = rec { b = c; }; a = { c = 2; }; yield a.b; }",
            "let c = 10; a = rec { b = c; }; a = { c = 2; }; in a.b",
            "2",
        ),
    ] {
        let from_nxc = parse_nxc(nxc_source).unwrap();
        let from_native = nix::import(native_source).unwrap();
        assert_eq!(from_nxc, from_native, "binding grouping changed");

        let generated = nix::emit(&from_nxc).unwrap();
        assert_eq!(evaluate(native_source), expected);
        assert_eq!(evaluate(&generated), expected, "generated {generated}");
    }
}

#[test]
fn plain_and_sourced_inherit_keep_their_distinct_scopes() {
    if !nix_available() {
        return;
    }
    let nxc_source = "let { inherit x; inherit (src) y; src = { y = 8; }; yield [x, y]; }";
    let generated = nix::emit(&parse_nxc(nxc_source).unwrap()).unwrap();
    let wrapped_generated = format!("let x = 7; in {generated}");
    let native = "let x = 7; in let inherit x; inherit (src) y; src = { y = 8; }; in [x y]";

    assert_eq!(evaluate(native), "[7,8]");
    assert_eq!(
        evaluate(&wrapped_generated),
        "[7,8]",
        "generated {wrapped_generated}"
    );
}

#[test]
fn recovery_after_the_yield_keeps_later_outer_expressions() {
    for source in [
        "f(let { yield 1; a = @; }, h(3))",
        "f(let { yield 1; yield @; }, h(3))",
        "f(let { yield 1; trailing; }, h(3))",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
        assert!(
            root.descendants()
                .any(|node| node.kind() == syntax::SyntaxKind::CallExpr && node.text() == "h(3)"),
            "recovery consumed the later argument: {root:#?}"
        );
    }
}

#[test]
fn a_let_default_can_produce_the_callable_used_by_an_outer_call() {
    if !nix_available() {
        return;
    }
    let source = "let { f = {}.missing or let { yield x => x + 1; }; yield f; }(2)";
    let ir = parse_nxc(source).unwrap();
    let canonical_nxc = emit::nxc(&ir).unwrap();
    assert_eq!(parse_nxc(&canonical_nxc).unwrap(), ir);

    let generated = nix::emit(&ir).unwrap();
    assert_eq!(evaluate(&generated), "3", "generated {generated}");
}
