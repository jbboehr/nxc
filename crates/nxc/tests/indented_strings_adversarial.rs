use nxc::{
    emit,
    ir::{Expr, StringPart},
    nix, parse_nxc, syntax,
};
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

fn assert_frontends_and_emitters(source: &str, expected: &Expr) -> String {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    assert_eq!(
        parsed
            .lower()
            .unwrap_or_else(|errors| panic!("nxc rejected {source:?}: {errors:?}")),
        *expected,
        "nxc decoded {source:?} incorrectly"
    );
    assert_eq!(
        nix::import(source)
            .unwrap_or_else(|errors| panic!("native adapter rejected {source:?}: {errors:?}")),
        *expected,
        "native adapter decoded {source:?} incorrectly"
    );

    let nxc_source = emit::nxc(expected).unwrap();
    assert!(
        nxc_source.starts_with('"') && nxc_source.ends_with('"'),
        "nxc emitter retained the input quote style: {nxc_source:?}"
    );
    assert_eq!(parse_nxc(&nxc_source).unwrap(), *expected);
    let nix_source = nix::emit(expected).unwrap();
    assert!(
        nix_source.starts_with('"') && nix_source.ends_with('"'),
        "Nix emitter retained the input quote style: {nix_source:?}"
    );
    assert_eq!(nix::import(&nix_source).unwrap(), *expected);
    nix_source
}

fn assert_native_nix(comparisons: &[String]) {
    if !nix_available() {
        return;
    }
    let expression = comparisons.join(" && ");
    let output = Command::new("nix-instantiate")
        .args([
            "--store",
            "dummy://",
            "--eval",
            "--strict",
            "--json",
            "--expr",
            &format!("let x = \"X\"; in {expression}"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "true");
}

#[test]
fn adjacent_indented_escape_fragments_decode_independently() {
    let fragments = [
        ("'''", "''"),
        ("''$", "$"),
        (r"''\n", "\n"),
        (r"''\ ", " "),
        (r"''\🦀", "🦀"),
    ];
    let mut comparisons = Vec::new();

    for (left_source, left_value) in fragments {
        for (right_source, right_value) in fragments {
            let source = format!("''{left_source}{right_source}''");
            let expected = Expr::String(vec![StringPart::Literal(format!(
                "{left_value}{right_value}"
            ))]);
            let emitted = assert_frontends_and_emitters(&source, &expected);
            comparisons.push(format!("({source}) == ({emitted})"));
        }
    }

    assert_native_nix(&comparisons);
}

#[test]
fn final_line_trimming_respects_interpolation_and_escape_fragments() {
    let x = || StringPart::Interpolation(Expr::Variable("x".into()));
    let cases = [
        (
            "''\n  a${x}\n    ''",
            Expr::String(vec![
                StringPart::Literal("a".into()),
                x(),
                StringPart::Literal("\n".into()),
            ]),
            "aX\n",
        ),
        (
            "''\n  a\n    ${x}''",
            Expr::String(vec![StringPart::Literal("a\n  ".into()), x()]),
            "a\n  X",
        ),
        (
            "''\n  a${x}\n    ''\\ ''",
            Expr::String(vec![
                StringPart::Literal("a".into()),
                x(),
                StringPart::Literal("\n   ".into()),
            ]),
            "aX\n   ",
        ),
        (
            "''''$${x}''",
            Expr::String(vec![StringPart::Literal("$".into()), x()]),
            "$X",
        ),
        (
            r#"''''\\${x}''"#,
            Expr::String(vec![StringPart::Literal("\\".into()), x()]),
            "\\X",
        ),
    ];
    let mut comparisons = Vec::new();

    for (source, expected, value) in cases {
        let emitted = assert_frontends_and_emitters(source, &expected);
        let value = nix::emit(&Expr::String(vec![StringPart::Literal(value.into())])).unwrap();
        comparisons.push(format!("({source}) == ({value})"));
        comparisons.push(format!("({emitted}) == ({value})"));
    }

    assert_native_nix(&comparisons);
}
