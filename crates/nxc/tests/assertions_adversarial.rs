use nxc::{MAX_DEPTH, MAX_SOURCE_BYTES, emit, ir::Expr, nix, parse_nxc, syntax};
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
        .unwrap_or_else(|error| panic!("cannot run Nix for {source:?}: {error}"))
        .status
        .success()
}

#[test]
fn native_assertion_boundaries_match_the_nix_parser() {
    if !nix_available() {
        return;
    }

    for source in [
        "assert true; 1 + 2",
        "assert true; with {}; 1",
        "assert true; let x = 1; in x",
        "assert true; if false then 1 else 2",
        "assert x: x; 1",
        "assert true; x: x",
        "let x = assert true; 1; in x",
        "{ a = assert true; 1; }",
        r#""${assert true; "ok"}""#,
        "(assert true; x: x) 1",
        "assert (assert true; true); 1",
        "[assert true; 1]",
        "f assert true; 1",
        "1 + assert true; 2",
        "-assert true; 1",
        "{}.a or assert true; 1",
        "assert true;",
        "assert; 1",
        "assert true; 1; 2",
    ] {
        assert_eq!(
            nix::import(source).is_ok(),
            native_nix_accepts(source),
            "native adapter disagreed with Nix for {source:?}"
        );
    }
}

fn application(arguments: usize) -> Expr {
    (0..arguments).fold(Expr::Variable("f".into()), |function, _| Expr::Apply {
        function: Box::new(function),
        argument: Box::new(Expr::Integer(1)),
    })
}

fn assertion_with(value: Expr, in_condition: bool) -> Expr {
    if in_condition {
        Expr::Assert {
            condition: Box::new(value),
            body: Box::new(Expr::Variable("true".into())),
        }
    } else {
        Expr::Assert {
            condition: Box::new(Expr::Variable("true".into())),
            body: Box::new(value),
        }
    }
}

#[test]
fn flattened_calls_in_both_assertion_children_use_semantic_depth() {
    let exact_arguments = MAX_DEPTH - 2;
    let nxc_call = format!(
        "f({})",
        (0..exact_arguments)
            .map(|_| "1")
            .collect::<Vec<_>>()
            .join(", ")
    );
    let native_call = format!("f{}", " 1".repeat(exact_arguments));
    let over_nxc_call = format!(
        "f({})",
        (0..=exact_arguments)
            .map(|_| "1")
            .collect::<Vec<_>>()
            .join(", ")
    );
    let over_native_call = format!("{native_call} 1");

    for in_condition in [true, false] {
        let expected = assertion_with(application(exact_arguments), in_condition);
        let nxc_source = if in_condition {
            format!("assert({nxc_call}, true)")
        } else {
            format!("assert(true, {nxc_call})")
        };
        let native_source = if in_condition {
            format!("assert {native_call}; true")
        } else {
            format!("assert true; {native_call}")
        };

        assert_eq!(parse_nxc(&nxc_source).unwrap(), expected);
        assert_eq!(nix::import(&native_source).unwrap(), expected);
        assert_eq!(parse_nxc(&emit::nxc(&expected).unwrap()).unwrap(), expected);
        assert_eq!(
            nix::import(&nix::emit(&expected).unwrap()).unwrap(),
            expected
        );

        let over = assertion_with(application(exact_arguments + 1), in_condition);
        let over_nxc = if in_condition {
            format!("assert({over_nxc_call}, true)")
        } else {
            format!("assert(true, {over_nxc_call})")
        };
        let over_native = if in_condition {
            format!("assert {over_native_call}; true")
        } else {
            format!("assert true; {over_native_call}")
        };

        assert!(parse_nxc(&over_nxc).is_err());
        assert!(nix::import(&over_native).is_err());
        assert!(emit::nxc(&over).is_err());
        assert!(nix::emit(&over).is_err());
    }
}

fn assertion_with_variable(length: usize, in_condition: bool) -> Expr {
    assertion_with(Expr::Variable("a".repeat(length)), in_condition)
}

#[test]
fn both_assertion_children_count_toward_the_generated_byte_limit() {
    for in_condition in [true, false] {
        let one_byte = assertion_with_variable(1, in_condition);

        let nxc_fixed = emit::nxc(&one_byte).unwrap().len() - 1;
        let nxc_exact = assertion_with_variable(MAX_SOURCE_BYTES - nxc_fixed, in_condition);
        let nxc_source = emit::nxc(&nxc_exact).unwrap();
        assert_eq!(nxc_source.len(), MAX_SOURCE_BYTES);
        assert_eq!(parse_nxc(&nxc_source).unwrap(), nxc_exact);
        assert!(
            emit::nxc(&assertion_with_variable(
                MAX_SOURCE_BYTES - nxc_fixed + 1,
                in_condition
            ))
            .is_err()
        );

        let native_fixed = nix::emit(&one_byte).unwrap().len() - 1;
        let native_exact = assertion_with_variable(MAX_SOURCE_BYTES - native_fixed, in_condition);
        let native_source = nix::emit(&native_exact).unwrap();
        assert_eq!(native_source.len(), MAX_SOURCE_BYTES);
        assert_eq!(nix::import(&native_source).unwrap(), native_exact);
        assert!(
            nix::emit(&assertion_with_variable(
                MAX_SOURCE_BYTES - native_fixed + 1,
                in_condition
            ))
            .is_err()
        );
    }
}

#[test]
fn malformed_nested_assertions_do_not_consume_the_next_outer_argument() {
    for source in [
        "f(assert(scope, value, extra(1, 2)), h(3))",
        "f(assert(scope, [bad(@), keep(1, 2)]), h(3))",
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
