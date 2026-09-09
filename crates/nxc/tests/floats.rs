mod support;
use support::nxc;

use nxc::{emit, ir::Expr, nix, parse_nxc, syntax};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) -> Expr {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let value = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    assert_eq!(nix::import(native).unwrap(), value, "{native}");
    let nxc = emit::nxc(&value).unwrap();
    let reparsed = parse_nxc(&nxc).unwrap();
    assert_eq!(reparsed, value, "{nxc}");
    let native = nix::emit(&reparsed).unwrap();
    assert_eq!(nix::import(&native).unwrap(), value, "{native}");
    assert_eq!(emit::nxc(&reparsed).unwrap(), nxc);
    value
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

fn evaluate(source: &str) -> std::process::Output {
    Command::new("nix-instantiate")
        .args([
            "--store", "dummy://", "--eval", "--strict", "--json", "--expr",
        ])
        .arg(format!("({source})"))
        .output()
        .unwrap()
}

#[test]
fn float_spellings_normalize_by_value_and_remain_distinct_from_integers() {
    for (spellings, integer) in [
        (vec!["0.0", ".0", "0.000e+99999", "0.0e-99999"], 0),
        (vec!["1.", "1.0", "1.0000", "0.1E+1", ".01e2"], 1),
        (vec!["100.", "1.e2", "1.0E+02", "10.0e1"], 100),
    ] {
        let expected = nix::import(spellings[0]).unwrap();
        assert_ne!(expected, Expr::Integer(integer));
        for spelling in spellings {
            assert_eq!(roundtrip(spelling, spelling), expected);
        }
    }
    for source in [
        ".5",
        "123.456",
        "1.0e-100",
        "1.7976931348623157e308",
        "2.2250738585072014e-308",
    ] {
        roundtrip(source, source);
    }
    assert_ne!(parse_nxc("-0.0").unwrap(), parse_nxc("0.0").unwrap());
    assert_ne!(
        parse_nxc("1.0000000000000002").unwrap(),
        parse_nxc("1.0").unwrap()
    );
}

#[test]
fn floats_compose_with_existing_expression_and_lexical_boundaries() {
    for (source, native) in [
        ("-1.5 * 2 + .25", "(-1.5) * 2 + .25"),
        ("f(1.5, -2.5)", "f 1.5 (-2.5)"),
        ("[1.5, .5, -0.0]", "[1.5 .5 (-0.0)]"),
        ("fn({ x ? .5 }) => x / 2.", "{ x ? .5 }: x / 2."),
        ("let { x = 1.0; yield x; }", "let x = 1.0; in x"),
        (
            "if 1.0 < 2.0 then .5 else 3.",
            "if 1.0 < 2.0 then .5 else 3.",
        ),
        ("assert(1.0 == 1, 2.5)", "assert 1.0 == 1; 2.5"),
        ("s.a or 1.5", "s.a or 1.5"),
        ("(1).a", "(1).a"),
        ("1 . a", "1 . a"),
        ("0.e1", "(0).e1"),
        ("1.0.a", "1.0.a"),
        ("1.0 ? a", "1.0 ? a"),
        ("{ ${1.0} = 2; }", "{ ${1.0} = 2; }"),
        (r#""${1.5}""#, r#""${1.5}""#),
        ("1.0/2", "1.0/2"),
        ("1.0e+2/3", "1.0e+2/3"),
        ("1.0 / 2", "1.0 / 2"),
        ("1.5 // comment\n + .5", "1.5 # comment\n + .5"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn invalid_and_out_of_range_floats_fail_losslessly_without_losing_later_items() {
    for source in [
        "0.",
        "1.0e",
        "1.0e+",
        "1.0e-",
        "1.0e309",
        "1.0e-999",
        "1.0e-308",
        "5.0e-324",
        "2.2250738585072012e-308",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted nxc {source}");
        assert!(nix::import(source).is_err(), "accepted native {source}");
    }
    for source in [
        "f(1.0e+, good(2.5))",
        "[1.0e+, good(2.5)]",
        "{ x = 1.0e+; y = good(2.5); }",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(
            root.descendants()
                .any(|n| n.kind() == syntax::SyntaxKind::CallExpr && n.text() == "good(2.5)"),
            "{source}: {root:#?}"
        );
    }
}

#[test]
fn native_nix_confirms_float_types_rounding_arithmetic_and_errors() {
    if !nix_available() {
        return;
    }
    for source in [
        "builtins.isFloat 1.0",
        "builtins.isInt 1.0",
        "builtins.toJSON [ 0.0 (-0.0) 1.0 ]",
        "[ (1.0 / 2) (1 / 2) (0.1 + 0.2) (1.0 == 1) (1.5 < 2) ]",
        "[ 1.0000000000000001 1.0000000000000002 9007199254740993.0 1.7976931348623157e308 2.2250738585072014e-308 ]",
        "1.0 / 0.0",
        "1.0 ? a",
        "{ ${1.5} = 2; }",
        r#""${1.5}""#,
        "if true then 1.5 else abort \"unused\"",
    ] {
        let ir = nix::import(source).unwrap();
        roundtrip(&emit::nxc(&ir).unwrap(), source);
        let converted = parse_nxc(&emit::nxc(&ir).unwrap()).unwrap();
        let generated = nix::emit(&converted).unwrap();
        let original = evaluate(source);
        let actual = evaluate(&generated);
        assert_eq!(
            actual.status.success(),
            original.status.success(),
            "{source}: {actual:?}"
        );
        if original.status.success() {
            assert_eq!(actual.stdout, original.stdout, "{source}: {generated}");
        }
    }
}

#[test]
fn caller_float_values_and_exact_subnormals_survive_native_emission() {
    use nxc::ir::Float;
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, -0.0] {
        assert!(Float::new(invalid).is_err());
    }
    for spelling in [
        "1",
        "1e2",
        "+1.0",
        "-1.0",
        "NaN",
        "inf",
        "01.0",
        "0x1p0",
        "1.0 /* comment */",
    ] {
        assert!(spelling.parse::<Float>().is_err(), "{spelling}");
    }
    let native = nix_available();
    for value in [
        0.0,
        f64::from_bits(1),
        f64::from_bits(2),
        f64::MIN_POSITIVE / 2.0,
        f64::from_bits(f64::MIN_POSITIVE.to_bits() - 1),
        f64::MIN_POSITIVE,
        f64::from_bits(f64::MIN_POSITIVE.to_bits() + 1),
        1.0,
        1.0e-100,
        1.0e100,
        f64::MAX,
    ] {
        let literal = Float::new(value).unwrap();
        assert_eq!(literal.value().to_bits(), value.to_bits());
        let expected = Expr::Float(literal);
        let nxc = emit::nxc(&expected).unwrap();
        let nix = nix::emit(&parse_nxc(&nxc).unwrap()).unwrap();
        assert_eq!(roundtrip(&nxc, &nix), expected);
        if native {
            let result = evaluate(&nix);
            assert!(result.status.success(), "{nix}: {result:?}");
            let actual: f64 = String::from_utf8(result.stdout)
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert_eq!(actual.to_bits(), value.to_bits(), "{nix}");
        }
    }
}

#[test]
fn underflow_guard_compares_decimal_spellings_exactly() {
    use nxc::ir::Float;
    // The spelling just below the smallest normal is deliberately rejected:
    // acceptance depends on libc strtod's tininess detection here.
    for source in [
        "2.2250738585072013e-308",
        "22.250738585072013E-309",
        ".22250738585072013e-307",
    ] {
        assert!(source.parse::<Float>().is_err());
    }
    let exact = format!("{:.1074}", f64::from_bits(1));
    let trimmed = exact.trim_end_matches('0');
    for source in [
        trimmed.to_owned(),
        format!("{trimmed}0e0"),
        format!("{trimmed}000E+00000"),
    ] {
        let value = source.parse::<Float>().unwrap();
        assert_eq!(value.value().to_bits(), 1);
    }
    // These long mantissas differ only beyond binary64's rounding precision.
    for suffix in ["1", "00000001"] {
        assert!(format!("{trimmed}{suffix}").parse::<Float>().is_err());
    }
}

#[test]
fn float_literals_obey_source_token_depth_and_emitted_byte_limits() {
    use nxc::{MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, ir::Float};
    let zero = Expr::Float(Float::new(0.0).unwrap());
    let exact_size = format!("0.{}", "0".repeat(MAX_SOURCE_BYTES - 2));
    assert_eq!(parse_nxc(&exact_size).unwrap(), zero);
    assert_eq!(nix::import(&exact_size).unwrap(), zero);
    let oversized = format!("{exact_size}0");
    assert!(parse_nxc(&oversized).is_err());
    assert!(nix::import(&oversized).is_err());
    // The standalone literal constructor uses the library ceiling, independently
    // of the smaller conversion budgets used by this boundary fixture.
    assert!(
        format!("0.{}", "0".repeat(::nxc::MAX_SOURCE_BYTES))
            .parse::<Float>()
            .is_err()
    );

    let exact_tokens = format!("[{}]", "0.0 ".repeat(MAX_TOKENS - 2));
    assert_eq!(
        parse_nxc(&exact_tokens).unwrap(),
        nix::import(&exact_tokens).unwrap()
    );
    let over = exact_tokens.replacen(']', "0.0]", 1);
    let parsed = syntax::parse(&over);
    assert_eq!(parsed.syntax().unwrap().to_string(), over);
    assert_eq!(parsed.diagnostics().len(), 1);
    assert!(nix::import(&over).is_err());

    let depth = format!("{}0.0", "-".repeat(MAX_DEPTH - 1));
    roundtrip(&depth, &depth);
    assert!(parse_nxc(&format!("-{depth}")).is_err());
    assert!(nix::import(&format!("-{depth}")).is_err());

    let tiny = Expr::Float(Float::new(f64::from_bits(1)).unwrap());
    let too_many_bytes = Expr::List(vec![tiny; MAX_SOURCE_BYTES / 1076 + 1]);
    assert!(emit::nxc(&too_many_bytes).is_err());
    assert!(nix::emit(&too_many_bytes).is_err());
}
