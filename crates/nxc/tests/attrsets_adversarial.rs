use nxc::{
    emit,
    ir::{Binding, Expr},
    nix, parse_nxc,
};

fn both_dialects_accept(source: &str) {
    let nxc_expr =
        parse_nxc(source).unwrap_or_else(|errors| panic!("nxc rejected {source}: {errors:?}"));
    let nix_expr = nix::import(source)
        .unwrap_or_else(|errors| panic!("native adapter rejected {source}: {errors:?}"));
    assert_eq!(nxc_expr, nix_expr, "dialects disagreed for {source}");
    assert_eq!(parse_nxc(&emit::nxc(&nix_expr).unwrap()).unwrap(), nix_expr);
    assert_eq!(
        nix::import(&nix::emit(&nxc_expr).unwrap()).unwrap(),
        nxc_expr
    );
}

#[test]
fn attribute_names_do_not_inherit_variable_name_restrictions() {
    for name in ["fn", "yield", "or", "__curPos", "__nxc_internal"] {
        both_dialects_accept(&format!("{{ {name} = 1; }}.{name}"));
        both_dialects_accept(&format!("{{ inherit (source) {name}; }}.{name}"));

        let plain_inherit = format!("{{ inherit {name}; }}");
        if matches!(name, "fn" | "yield" | "__curPos") {
            both_dialects_accept(&plain_inherit);
            continue;
        }
        assert!(
            parse_nxc(&plain_inherit).is_err(),
            "plain nxc inherit accepted variable-reserved name {name}"
        );
        assert!(
            nix::import(&plain_inherit).is_err(),
            "plain native inherit accepted variable-reserved name {name}"
        );
    }
}

#[test]
fn native_keywords_are_rejected_in_every_static_attribute_position() {
    for name in [
        "assert", "else", "if", "in", "inherit", "let", "rec", "then", "with",
    ] {
        for source in [
            format!("{{ {name} = 1; }}"),
            format!("value.{name}"),
            format!("{{ inherit (source) {name}; }}"),
        ] {
            assert!(parse_nxc(&source).is_err(), "nxc accepted {source}");
            assert!(
                nix::import(&source).is_err(),
                "native adapter accepted {source}"
            );
        }
    }
}

#[test]
fn literal_set_merges_still_reject_leaf_and_descendant_conflicts() {
    for source in [
        "{ a.b = {}; a.b.c = 1; }",
        "{ a.b.c = 1; a.b = { d = 2; }; }",
        "{ a = { b = {}; }; a.b.c = 1; }",
        "{ a.b = {}; a = { b.c = 1; }; }",
    ] {
        both_dialects_accept(source);
    }

    for source in [
        "{ a.b = { c = 1; }; a.b.c = 2; }",
        "{ a.b.c = 1; a = { b.c = 2; }; }",
        "{ a = { inherit b; }; a.b.c = 1; }",
        "{ a.b.c = 1; a = { inherit b; }; }",
    ] {
        assert!(parse_nxc(source).is_err(), "nxc accepted conflict {source}");
        assert!(
            nix::import(source).is_err(),
            "native adapter accepted conflict {source}"
        );
    }
}

#[test]
fn caller_ir_uses_the_same_merge_and_inherit_name_rules() {
    let valid = Expr::AttrSet {
        recursive: false,
        bindings: vec![
            Binding::Assign {
                path: vec!["a".into(), "b".into()],
                value: Expr::AttrSet {
                    recursive: false,
                    bindings: vec![],
                },
            },
            Binding::Assign {
                path: vec!["a".into(), "b".into(), "c".into()],
                value: Expr::Integer(1),
            },
            Binding::Inherit {
                source: Some(Expr::Variable("source".into())),
                names: vec!["fn".into(), "yield".into(), "or".into()],
            },
        ],
    };
    assert_eq!(parse_nxc(&emit::nxc(&valid).unwrap()).unwrap(), valid);
    assert_eq!(nix::import(&nix::emit(&valid).unwrap()).unwrap(), valid);

    let invalid = Expr::AttrSet {
        recursive: false,
        bindings: vec![
            Binding::Assign {
                path: vec!["a".into()],
                value: Expr::AttrSet {
                    recursive: false,
                    bindings: vec![Binding::Inherit {
                        source: None,
                        names: vec!["b".into()],
                    }],
                },
            },
            Binding::Assign {
                path: vec!["a".into(), "b".into(), "c".into()],
                value: Expr::Integer(1),
            },
        ],
    };
    assert!(emit::nxc(&invalid).is_err());
    assert!(nix::emit(&invalid).is_err());
}
