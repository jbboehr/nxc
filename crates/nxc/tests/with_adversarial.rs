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

fn nix_command(mode: &str, source: &str) -> std::process::Output {
    Command::new("nix-instantiate")
        .args([
            "--store", "dummy://", mode, "--strict", "--json", "--expr", source,
        ])
        .output()
        .unwrap_or_else(|error| panic!("cannot run Nix for {source:?}: {error}"))
}

fn evaluate(source: &str) -> String {
    let output = nix_command("--eval", source);
    assert!(
        output.status.success(),
        "{source}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn delayed_with_lookup_keeps_lexical_priority_and_laziness() {
    if !nix_available() {
        return;
    }
    for (source, native, expected) in [
        (
            "let { f = with({ x = 1; }, ignored => x); yield with({ x = 2; }, f(0)); }",
            "let f = with { x = 1; }; ignored: x; in with { x = 2; }; f 0",
            "1",
        ),
        ("with(1 / 0, x => x)(4)", "(with (1 / 0); x: x) 4", "4"),
        (
            "with({ x = 1; }, let { y = x; x = 2; yield y; })",
            "with { x = 1; }; let y = x; x = 2; in y",
            "2",
        ),
        (
            "with({ x = 1; }, with({ y = x; }, y))",
            "with { x = 1; }; with { y = x; }; y",
            "1",
        ),
    ] {
        let ir = parse_nxc(source).unwrap_or_else(|errors| panic!("{source}: {errors:?}"));
        assert_eq!(
            ir,
            nix::import(native).unwrap_or_else(|errors| panic!("{native}: {errors:?}")),
            "dialects disagreed for {source}"
        );
        let generated = nix::emit(&parse_nxc(&emit::nxc(&ir).unwrap()).unwrap()).unwrap();
        assert_eq!(
            evaluate(native),
            expected,
            "native oracle changed for {native}"
        );
        assert_eq!(evaluate(&generated), expected, "generated {generated}");
    }
}

#[test]
fn native_with_body_boundaries_match_nix_parser() {
    if !nix_available() {
        return;
    }
    for source in [
        "let x = with {}; 1; in x",
        "{ a = with {}; 1; }",
        "with {}; let x = 1; in x",
        "with {}; {}.a or 2",
        "with {}; (x: x) 1",
        "with {} 1; 2",
        "with {}; 1; 2",
        "{ a = with {}; 1 }",
        "let x = with {}; 1 in x",
        "with {}; with; 1",
        "with {};",
        "with ; 1",
    ] {
        let parsed = nix_command("--parse", source).status.success();
        assert_eq!(
            nix::import(source).is_ok(),
            parsed,
            "native adapter disagreed with Nix for {source:?}"
        );
    }
}

#[test]
fn malformed_nested_with_argument_does_not_consume_the_next_outer_argument() {
    for source in [
        "f(with(scope, value, extra(1, 2)), h(3))",
        "f(with(scope, [bad(@), keep(1, 2)]), h(3))",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
        assert!(
            root.descendants().any(|node| {
                node.kind() == syntax::SyntaxKind::CallExpr && node.text() == "h(3)"
            }),
            "recovery consumed the next outer argument: {root:#?}"
        );
    }
}
