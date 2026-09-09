use nxc::ir::{Expr, Formal, Pattern};
use nxc::{MAX_DEPTH, emit, nix, parse_nxc, syntax};

fn roundtrip(source: &str, native: &str) {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    assert!(
        parsed.diagnostics().is_empty(),
        "{source}: {:?}",
        parsed.diagnostics()
    );
    let actual = parsed.lower().unwrap();
    let expected = nix::import(native).unwrap_or_else(|e| panic!("{native}: {e:?}"));
    assert_eq!(actual.canonical(), expected.canonical(), "{source}");
    let emitted = emit::nxc(&expected).unwrap();
    assert_eq!(parse_nxc(&emitted).unwrap(), expected, "{emitted}");
    assert_eq!(nix::import(&nix::emit(&actual).unwrap()).unwrap(), expected);
    assert_eq!(emit::nxc(&parse_nxc(&emitted).unwrap()).unwrap(), emitted);
}

#[test]
fn simple_lambda_spellings_share_semantics_and_preserve_trivia() {
    for source in [
        "x => x + 1",
        "(x) => x + 1",
        "fn(x) => x + 1",
        "# α\nfn /* β */ (x) /* γ */ => x + 1 // end\n",
    ] {
        roundtrip(source, "x: x + 1");
    }
    roundtrip("true => true", "true: true");
}

#[test]
fn lambda_bodies_extend_right_and_calls_remain_curried() {
    for (source, native) in [
        ("x => y => x + y * 2", "x: y: x + y * 2"),
        ("(x => x + 1)(2)", "(x: x + 1) 2"),
        ("fn(x) => f(x, y => g(y))", "x: f x (y: g y)"),
        ("(x => y => x + y)(1, 2)", "(x: y: x + y) 1 2"),
        ("f(x => x + 1, 2)", "f (x: x + 1) 2"),
        ("1 + (x => x)(2) * 3", "1 + ((x: x) 2) * 3"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn attribute_patterns_preserve_fields_defaults_and_variadic_acceptance() {
    for (source, native) in [
        ("({}) => 1", "{}: 1"),
        ("fn({ x, y, }) => x + y", "{ x, y, }: x + y"),
        ("({ pkgs, lib, ... }) => pkgs", "{ pkgs, lib, ... }: pkgs"),
        ("({...}) => 1", "{...}: 1"),
        (
            "({ x ? 1 + 2, y ? x, ... }) => y",
            "{ x ? 1 + 2, y ? x, ... }: y",
        ),
        ("({ f ? x => x + 1 }) => f(2)", "{ f ? x: x + 1 }: f 2"),
        ("({ f ? ({ x ? 2 }) => x }) => f", "{ f ? { x ? 2 }: x }: f"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn nested_default_delimiters_do_not_split_pattern_fields() {
    for (source, native) in [
        (
            "({ x ? f(1, 2), y ? g(x), ... }) => y",
            "{ x ? f 1 2, y ? g x, ... }: y",
        ),
        (
            "({ x ? (a => a)(1), y ? x }) => y",
            "{ x ? (a: a) 1, y ? x }: y",
        ),
        (
            "({ x ? f(g(1, 2), 3), y }) => x",
            "{ x ? f (g 1 2) 3, y }: x",
        ),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn whole_argument_capture_works_on_either_side_of_the_pattern() {
    for source in [
        "(args@{ x ? 1, ... }) => args",
        "({ x ? 1, ... }@args) => args",
        "fn(args@{ x ? 1, ... }) => args",
        "fn({ x ? 1, ... }@args) => args",
    ] {
        roundtrip(source, "args@{ x ? 1, ... }: args");
    }
    roundtrip("({ x }@args) => x", "{ x }@args: x");
}

#[test]
fn invalid_parameters_and_unsupported_lambda_forms_are_rejected_losslessly() {
    for source in [
        "x: x",
        "fn x => x",
        "fn(x)",
        "fn() => 1",
        "(x, y) => x",
        "1 => 1",
        "(x + y) => x",
        "f(x) => x",
        "1 + x => x",
        "-x => x",
        "x =>",
        "({ x, x }) => x",
        "(x@{ x }) => x",
        "({ x }@x) => x",
        "(a@{ x }@b) => x",
        "({ x ? }) => x",
        "({ ..., x }) => x",
        "({ ..., ... }) => 1",
        "({ x,, y }) => x",
        "({ x = 1; }) => x",
        "({ ..., }) => 1",
        "({ x ... }) => x",
        "({ x y }) => x",
        "{ x } => x",
        "({ fn }) => 1",
        "(__curPos) => 1",
        "(__nxc_x) => 1",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source, "{source}");
        assert!(parsed.lower().is_err(), "accepted {source}");
    }
    for source in [
        "{ x, x }: x",
        "x@{ x }: x",
        "{ x }@x: x",
        "__nxc_private: 1",
        "{ __curPos }: 1",
    ] {
        assert!(nix::import(source).is_err(), "accepted {source}");
    }
}

#[test]
fn lambda_depth_limits_cover_bodies_and_defaults() {
    let source = format!("{}1", "x => ".repeat(MAX_DEPTH - 1));
    let native = format!("{}1", "x: ".repeat(MAX_DEPTH - 1));
    roundtrip(&source, &native);
    for source in [
        format!("x => {source}"),
        format!("{}1", "x => ".repeat(400)),
    ] {
        let parsed = syntax::parse(&source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err());
    }
    assert!(nix::import(&format!("x: {native}")).is_err());
}

#[test]
fn pattern_default_delimiters_enforce_the_exact_source_depth_limit() {
    fn nested_default(levels: usize) -> String {
        (0..levels).fold("1".to_owned(), |default, _| {
            format!("({{ x ? {default} }}) => 1")
        })
    }

    let exact = nested_default(MAX_DEPTH / 2);
    assert_eq!(
        exact
            .chars()
            .filter(|character| matches!(character, '(' | '{'))
            .count(),
        MAX_DEPTH
    );
    assert!(parse_nxc(&exact).is_ok());

    let over = nested_default(MAX_DEPTH / 2 + 1);
    let parsed = syntax::parse(&over);
    assert_eq!(parsed.syntax().unwrap().to_string(), over);
    assert_eq!(parsed.diagnostics().len(), 1);
    assert!(parsed.diagnostics()[0].message.contains("limit"));
    assert!(parsed.lower().is_err());
}

#[test]
fn emission_checks_delimiter_depth_in_nested_pattern_defaults() {
    // Each emitted nxc pattern default adds two parentheses and one brace.
    let mut expr = Expr::Integer(1);
    for _ in 0..MAX_DEPTH / 3 {
        expr = Expr::Lambda {
            parameter: Pattern::AttrSet {
                fields: vec![Formal {
                    name: "x".into(),
                    default: Some(expr),
                }],
                ellipsis: false,
                bind: None,
            },
            body: Box::new(Expr::Integer(1)),
        };
    }
    let source = emit::nxc(&expr).unwrap();
    assert_eq!(parse_nxc(&source).unwrap(), expr);
    let over = Expr::Lambda {
        parameter: Pattern::AttrSet {
            fields: vec![Formal {
                name: "x".into(),
                default: Some(expr),
            }],
            ellipsis: false,
            bind: None,
        },
        body: Box::new(Expr::Integer(1)),
    };
    // The IR and native source still fit their own bounds.
    assert_eq!(nix::import(&nix::emit(&over).unwrap()).unwrap(), over);
    assert!(
        emit::nxc(&over).is_err(),
        "emitter accepted output beyond the nxc nesting limit"
    );
}

#[test]
fn emitters_validate_caller_supplied_patterns_and_default_expressions() {
    for parameter in [
        Pattern::Ident("x: 1".into()),
        Pattern::Ident("__nxc_ident_fn".into()),
        Pattern::AttrSet {
            fields: vec![
                Formal {
                    name: "x".into(),
                    default: None
                };
                2
            ],
            ellipsis: false,
            bind: None,
        },
        Pattern::AttrSet {
            fields: vec![Formal {
                name: "x".into(),
                default: None,
            }],
            ellipsis: false,
            bind: Some("x".into()),
        },
        Pattern::AttrSet {
            fields: vec![Formal {
                name: "x".into(),
                default: Some(Expr::Integer(u64::MAX)),
            }],
            ellipsis: false,
            bind: None,
        },
    ] {
        let expr = Expr::Lambda {
            parameter,
            body: Box::new(Expr::Integer(1)),
        };
        assert!(emit::nxc(&expr).is_err());
        assert!(nix::emit(&expr).is_err());
    }
}
