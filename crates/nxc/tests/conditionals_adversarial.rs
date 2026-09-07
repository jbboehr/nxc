use nxc::{MAX_SOURCE_BYTES, emit, ir::Expr, nix, parse_nxc, syntax};
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

fn native_nix_accepts(source: &str) -> bool {
    Command::new("nix-instantiate")
        .args(["--store", "dummy://", "--parse", "--expr", source])
        .output()
        .unwrap()
        .status
        .success()
}

#[test]
fn conditional_expression_boundaries_match_the_native_nix_parser() {
    if !nix_available() {
        return;
    }

    for source in [
        "if true then if false then 1 else 2 else 3",
        "if (if false then true else false) then 1 else 2",
        "if x: x then 1 else 2",
        "if true then with {}; 1 else let in 2",
        "if true then 1 else 2 3",
        "(if true then x: x else x: x + 1) 4",
        "(if true then { a = 1; } else {}).a",
        "{}.a or (if true then 1 else 2)",
        "let x = if true then 1 else 2; in x",
        "{ a = if true then 1 else 2; }",
        r#""${if true then "yes" else "no"}""#,
        "with {}; if true then 1 else 2",
        "f if true then 1 else 2",
        "[if true then 1 else 2]",
        "1 + if true then 2 else 3",
        "-if true then 1 else 2",
        "{}.a or if true then 1 else 2",
        "{ a = if true then 1 else 2 }",
        "let x = if true then 1 else 2 in x",
        "if true then 1 else 2; 3",
    ] {
        let native_accepted = native_nix_accepts(source);
        let adapted = nix::import(source);
        assert_eq!(
            adapted.is_ok(),
            native_accepted,
            "native adapter disagreed with Nix for {source:?}"
        );

        if let Ok(ir) = adapted {
            let emitted_nix = nix::emit(&ir).unwrap();
            assert!(
                native_nix_accepts(&emitted_nix),
                "emitted invalid native Nix for {source:?}: {emitted_nix:?}"
            );
            let emitted_nxc = emit::nxc(&ir).unwrap();
            assert_eq!(
                parse_nxc(&emitted_nxc).unwrap(),
                ir,
                "canonical nxc changed {source:?}"
            );
        }
    }
}

#[test]
fn structurally_incomplete_conditionals_do_not_hide_later_items() {
    for source in [
        "f(if true then 1, h(3))",
        "f(if true else 1, h(3))",
        "[if true then 1, h(3)]",
        "{ x = if true then 1; y = h(3); }",
        "let { x = if true then 1; yield h(3); }",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source:?}");
        assert!(
            root.descendants()
                .any(|node| node.kind() == syntax::SyntaxKind::CallExpr && node.text() == "h(3)"),
            "recovery consumed the later item in {source:?}: {root:#?}"
        );
    }
}

fn conditional_with_else_length(length: usize) -> Expr {
    Expr::If {
        condition: Box::new(Expr::Variable("true".into())),
        then_branch: Box::new(Expr::Integer(0)),
        else_branch: Box::new(Expr::Variable("a".repeat(length))),
    }
}

#[test]
fn an_unselected_branch_still_counts_toward_the_generated_size_limit() {
    let one_byte = conditional_with_else_length(1);
    let fixed_bytes = emit::nxc(&one_byte).unwrap().len() - 1;
    let exact = conditional_with_else_length(MAX_SOURCE_BYTES - fixed_bytes);

    for source in [emit::nxc(&exact).unwrap(), nix::emit(&exact).unwrap()] {
        assert_eq!(source.len(), MAX_SOURCE_BYTES);
        assert_eq!(parse_nxc(&source).unwrap(), exact);
        assert_eq!(nix::import(&source).unwrap(), exact);
    }

    let over = conditional_with_else_length(MAX_SOURCE_BYTES - fixed_bytes + 1);
    assert!(emit::nxc(&over).is_err());
    assert!(nix::emit(&over).is_err());
}
