use nxc::{emit, nix, parse_nxc, syntax};

fn roundtrip(source: &str, native: &str) {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let actual = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    let expected = nix::import(native).unwrap_or_else(|e| panic!("{native}: {e:?}"));
    assert_eq!(actual.canonical(), expected.canonical(), "{source}");
    let generated = emit::nxc(&expected).unwrap();
    assert_eq!(parse_nxc(&generated).unwrap(), expected, "{generated}");
    assert_eq!(nix::import(&nix::emit(&actual).unwrap()).unwrap(), expected);
}

#[test]
fn static_attrsets_dotted_bindings_and_inheritance_roundtrip() {
    for source in [
        "{}",
        "{ a = 1; b = 2; }",
        "rec { a = b + 1; b = 2; }",
        "{ a.b = 1; a.c = 2; }",
        "{ a = { b = 1; }; a.c = 2; }",
        "{ a.c = 2; a = { b = 1; }; }",
        "{ a = {}; a = {}; }",
        "{ a = rec { b = c; }; a = { c = 2; }; }",
        "{ inherit x y; }",
        "{ inherit (src) x y; }",
        "{ inherit; inherit (1 / 0); }",
        "{ fn = 1; yield = 2; or = 3; __curPos = 4; }",
        "# α\nrec { /* β */ a.b = 1; inherit /* γ */ (src) c; } # end",
    ] {
        roundtrip(source, source);
    }
    roundtrip(
        "{ f = x => x + 1; value = g(2); }",
        "{ f = x: x + 1; value = g 2; }",
    );
    roundtrip(
        "(fn({ x }) => x + 1)({ x = 2; })",
        "({ x }: x + 1) { x = 2; }",
    );
}

#[test]
fn selection_precedence_and_defaults_follow_nix() {
    for (source, native) in [
        ("s.a.b", "s.a.b"),
        ("s.a or 2 + 3", "s.a or 2 + 3"),
        ("s.a or f(2)", "(s.a or f) 2"),
        ("s.a or (f(2))", "s.a or (f 2)"),
        ("s.a or t.b or 3", "s.a or t.b or 3"),
        ("f(x).a", "(f x).a"),
        ("f(x.a)", "f x.a"),
        ("s.f(1, 2).result", "(s.f 1 2).result"),
        ("-s.a * 2 + 1", "-s.a * 2 + 1"),
        ("(s.a or t).b", "(s.a or t).b"),
        ("{ a = 1; }.a", "{ a = 1; }.a"),
        ("s.or", "s.or"),
        ("s.fn", "s.fn"),
        ("({ x ? s.a or 3 }) => x", "{ x ? s.a or 3 }: x"),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn native_selection_lambda_defaults_require_parentheses() {
    for (native_lambda, nxc_lambda) in [
        ("x: x", "x => x"),
        ("{ x }: x", "({ x }) => x"),
        ("args@{ x }: x", "(args@{ x }) => x"),
        ("{ x }@args: x", "({ x }@args) => x"),
    ] {
        for prefix in ["{}.a or ", "{}.a or {}.b or "] {
            let bare = format!("{prefix}{native_lambda}");
            assert!(nix::import(&bare).is_err(), "accepted invalid Nix: {bare}");
            roundtrip(
                &format!("{prefix}({nxc_lambda})"),
                &format!("{prefix}({native_lambda})"),
            );
        }
    }
}

#[test]
fn malformed_and_dynamic_attributes_are_rejected_losslessly() {
    for source in [
        "{ a = 1 }",
        "{ a = ; b = 2; }",
        "{ a = 1, b = 2; }",
        "{ a; }",
        "{ a..b = 1; }",
        "{ = 1; }",
        "{ inherit (x; }",
        "s.",
        "s.a or",
        "s.a or -1",
        "s.a or x => x",
        "s..a",
        "s.1",
        "{ 1 = 2; }",
        "{ rec = 1; }",
        "{ \"${a}\" = 1; }",
        "{ ${x} = 1; }",
        "s.${x}",
        "s.\"${x}\"",
        "s ? x",
        "let x = 1; in x",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
    }
}

#[test]
fn binding_conflicts_are_rejected_in_both_dialects() {
    for source in [
        "{ a = 1; a = 2; }",
        "{ a.b = 1; a.b = 2; }",
        "{ a = 1; a.b = 2; }",
        "{ a.b = 1; a = 2; }",
        "{ a = { b = 1; }; a.b = 2; }",
        "{ a.b = 2; a = { b = 1; }; }",
        "{ inherit a; a.b = 1; }",
        "{ a = {}; inherit a; }",
        "{ inherit (src) a a; }",
        "{ a = {}; a = 1; }",
    ] {
        assert!(parse_nxc(source).is_err(), "nxc accepted {source}");
        assert!(
            nix::import(source).is_err(),
            "native adapter accepted {source}"
        );
    }
}

#[test]
fn recovery_retains_later_bindings_and_arguments_after_nested_errors() {
    for source in [
        "{ a = @; b = 3; }",
        "{ a = f(@, { inner = @; good = 2; }); b = 3; }",
        "f(@ + { inner = g(1, 2); }, { b = 3; }, 4)",
    ] {
        let parsed = syntax::parse(source);
        assert!(parsed.lower().is_err());
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(
            root.descendants()
                .any(|node| node.kind() == syntax::SyntaxKind::AssignBinding
                    && node.text() == "b = 3"),
            "{root:#?}"
        );
    }
}

#[test]
fn attribute_paths_and_nested_sets_obey_resource_limits() {
    let path = vec!["a"; nxc::MAX_DEPTH].join(".");
    roundtrip(&format!("s.{path}"), &format!("s.{path}"));
    for source in [format!("s.{path}.a"), format!("{{ {path} = 1; }}")] {
        let parsed = syntax::parse(&source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(nix::import(&source).is_err());
    }
    let nested =
        (0..nxc::MAX_DEPTH - 1).fold("1".to_owned(), |value, _| format!("{{ a = {value}; }}"));
    roundtrip(&nested, &nested);
    let over = format!("{{ a = {nested}; }}");
    assert!(parse_nxc(&over).is_err());
    assert!(nix::import(&over).is_err());
}

#[test]
fn emitters_reject_invalid_paths_and_bindings_from_callers() {
    use nxc::ir::{Binding, Expr};
    for expr in [
        Expr::Select {
            value: Box::new(Expr::Integer(1)),
            path: vec![],
            default: None,
        },
        Expr::Select {
            value: Box::new(Expr::Integer(1)),
            path: vec!["x;\0y".into()],
            default: None,
        },
        Expr::AttrSet {
            recursive: false,
            bindings: vec![Binding::Assign {
                path: vec![],
                value: Expr::Integer(1),
            }],
        },
        Expr::AttrSet {
            recursive: false,
            bindings: vec![Binding::Assign {
                path: vec!["rec\0".into()],
                value: Expr::Integer(1),
            }],
        },
        Expr::AttrSet {
            recursive: false,
            bindings: vec![
                Binding::Assign {
                    path: vec!["x".into()],
                    value: Expr::Integer(1)
                };
                2
            ],
        },
        Expr::AttrSet {
            recursive: true,
            bindings: vec![Binding::Inherit {
                source: None,
                names: vec!["__curPos".into()],
            }],
        },
    ] {
        assert!(emit::nxc(&expr).is_err());
        assert!(nix::emit(&expr).is_err());
    }
}
