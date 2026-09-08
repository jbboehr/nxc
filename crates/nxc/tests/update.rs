use nxc::{
    MAX_DEPTH, MAX_TOKENS, emit,
    ir::{BinaryOp, Expr},
    nix, parse_nxc, syntax,
};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let actual = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    let expected = nix::import(native).unwrap_or_else(|e| panic!("{native}: {e:?}"));
    assert_eq!(actual, expected, "{source}");
    assert_eq!(parse_nxc(&emit::nxc(&actual).unwrap()).unwrap(), expected);
    assert_eq!(nix::import(&nix::emit(&actual).unwrap()).unwrap(), expected);
}

#[test]
fn reserved_update_form_preserves_both_operands_and_native_grouping() {
    for (source, native) in [
        ("__nxc_update(a, b)", "a // b"),
        ("__nxc_update(a, b,)", "a // b"),
        ("__nxc_update({}, {})", "{}//{}"),
        ("__nxc_update(a, __nxc_update(b, c))", "a // b // c"),
        ("__nxc_update(__nxc_update(a, b), c)", "(a // b) // c"),
        ("__nxc_update(a + b, c * d)", "a + b // c * d"),
        ("__nxc_update(!a, b)", "!a // b"),
        ("__nxc_update(a, !b)", "a // !b"),
        ("!__nxc_update(a, b)", "!(a // b)"),
        ("__nxc_update(a, b) == c && d", "a // b == c && d"),
        ("a < __nxc_update(b, c)", "a < b // c"),
        ("__nxc_update(s.a or b, c)", "s.a or b // c"),
        ("__nxc_update(f(1, 2), g(3))", "f 1 2 // g 3"),
        ("/* α */ __nxc_update /* β */ (a, // γ\nb,)", "a // b"),
    ] {
        roundtrip(source, native);
    }
    let ir = nix::import("a // b").unwrap();
    assert_eq!(
        ir,
        Expr::Binary {
            op: BinaryOp::Update,
            left: Box::new(Expr::Variable("a".into())),
            right: Box::new(Expr::Variable("b".into())),
        }
    );
    assert_eq!(emit::nxc(&ir).unwrap(), "__nxc_update(a, b)");
    assert_eq!(parse_nxc("a // b").unwrap(), Expr::Variable("a".into()));
    assert_eq!(
        parse_nxc("a // comment\n + b").unwrap(),
        parse_nxc("a + b").unwrap()
    );
}

#[test]
fn update_composes_with_expression_positions_and_reserved_attribute_names() {
    for (source, native) in [
        ("__nxc_update(a, b).x", "(a // b).x"),
        ("__nxc_update(a, b)(x)", "(a // b) x"),
        ("s.a or __nxc_update(a, b)", "s.a or (a // b)"),
        ("f(__nxc_update(a, b), c)", "f (a // b) c"),
        (
            "[__nxc_update(a, b) __nxc_update(c, d)]",
            "[(a // b) (c // d)]",
        ),
        ("x => __nxc_update(x, {})", "x: x // {}"),
        ("({ x ? __nxc_update(a, b) }) => x", "{ x ? a // b }: x"),
        (
            "__nxc_update(x => x, if c then a else b)",
            "(x: x) // (if c then a else b)",
        ),
        (
            "if c then __nxc_update(a, b) else d",
            "if c then a // b else d",
        ),
        ("assert(c, __nxc_update(a, b))", "assert c; a // b"),
        ("with(__nxc_update(a, b), x)", "with a // b; x"),
        (
            "let { a = __nxc_update(b, c); yield a; }",
            "let a = b // c; in a",
        ),
        (
            "{ inherit (__nxc_update(a, b)) x; }",
            "{ inherit (a // b) x; }",
        ),
        (r#""${__nxc_update(a, b)}""#, r#""${a // b}""#),
        (
            "{ __nxc_update = 1; }.__nxc_update",
            "{ __nxc_update = 1; }.__nxc_update",
        ),
        ("s.__nxc_update(1, 2)", "s.__nxc_update 1 2"),
        ("s.__nxc_update-suffix", "s.__nxc_update-suffix"),
        (
            "{ inherit (s) __nxc_update; }",
            "{ inherit (s) __nxc_update; }",
        ),
        (
            "let { s.__nxc_update = 1; yield s.__nxc_update; }",
            "let s.__nxc_update = 1; in s.__nxc_update",
        ),
        (
            "with({ __nxc_update = 1; }, __nxc_update({}, {}))",
            "with { __nxc_update = 1; }; {} // {}",
        ),
    ] {
        roundtrip(source, native);
    }
}

#[test]
fn malformed_updates_are_lossless_and_preserve_later_enclosing_items() {
    roundtrip("__nxc_update(a, b)", "a // b");
    for source in [
        "__nxc_update",
        "__nxc_update()",
        "__nxc_update(a)",
        "__nxc_update(a,)",
        "__nxc_update(a)(b)",
        "(__nxc_update)(a, b)",
        "__nxc_update(a, b, c)",
        "__nxc_update(, b)",
        "__nxc_update(a,, b)",
        "__nxc_update(a; b)",
        "__nxc_update(a, b",
        "__nxc_update(a, @)",
        "__nxc_update => 1",
        "(__nxc_update) => 1",
        "({ __nxc_update }) => 1",
        "({ __nxc_update ? 1 }) => 1",
        "({ x } @ __nxc_update) => x",
        "let { __nxc_update = 1; yield 1; }",
        "{ inherit __nxc_update; }",
        "let { inherit (s) __nxc_update; yield 1; }",
        "__nxc_update_extra(a, b)",
        "__nxc_update-suffix(a, b)",
        "__nxc_update({}, ./path${__curPos})",
        "__nxc_update(__nxc_bad, {})",
        "__nxc_update({ a = 1; a = 2; }, { a = 3; })",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted {source}");
    }
    for source in [
        "__nxc_update(@, h(3))",
        "f(__nxc_update(a), h(3))",
        "f(__nxc_update(a, @), h(3))",
        "f(__nxc_update(a, b, c), h(3))",
        "f(__nxc_update(a, (@, g(2))), h(3))",
        "[__nxc_update(@, b), h(3)]",
        "{ a = __nxc_update(@, b); good = h(3); }",
        "let { a = __nxc_update(@, b); yield h(3); }",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(
            root.descendants()
                .any(|n| n.kind() == syntax::SyntaxKind::CallExpr && n.text() == "h(3)"),
            "lost later item: {root:#?}"
        );
    }
    for source in [
        "a //",
        "a // x: x",
        "[a // b]",
        "s.a or a // x: x",
        "a // if c then a else b",
        "a // assert true; b",
        "__nxc_update a b",
        "__nxc_update: 1",
        "let __nxc_update = 1; in 1",
    ] {
        assert!(nix::import(source).is_err(), "accepted native: {source}");
    }
}

#[test]
fn emitters_validate_each_update_operand_before_rendering() {
    let update = nix::import("a // b").unwrap();
    for invalid in [
        Expr::Integer(u64::MAX),
        Expr::Variable("__nxc_update".into()),
        (0..MAX_DEPTH - 1).fold(Expr::Integer(1), |e, _| Expr::Not(Box::new(e))),
        Expr::List(vec![Expr::Integer(1); MAX_TOKENS]),
    ] {
        for in_left in [true, false] {
            let mut expr = update.clone();
            let Expr::Binary { left, right, .. } = &mut expr else {
                panic!("update must remain binary")
            };
            **if in_left { left } else { right } = invalid.clone();
            assert!(emit::nxc(&expr).is_err());
            assert!(nix::emit(&expr).is_err());
        }
    }
}

#[test]
fn both_update_operands_obey_depth_and_token_limits() {
    for in_left in [true, false] {
        let wrap = |inner: &str, native| match (in_left, native) {
            (true, false) => format!("__nxc_update({inner}, a)"),
            (false, false) => format!("__nxc_update(a, {inner})"),
            (true, true) => format!("({inner} // a)"),
            (false, true) => format!("a // {inner}"),
        };
        let source = (0..MAX_DEPTH - 1).fold("a".to_owned(), |inner, _| wrap(&inner, false));
        let native = (0..MAX_DEPTH - 1).fold("a".to_owned(), |inner, _| wrap(&inner, true));
        roundtrip(&source, &native);
        let over = wrap(&source, false);
        let parsed = syntax::parse(&over);
        assert_eq!(parsed.syntax().unwrap().to_string(), over);
        assert!(parsed.lower().is_err());
        assert!(nix::import(&wrap(&native, true)).is_err());
    }
    let longest = format!("{}a", "a // ".repeat((MAX_TOKENS - 1) / 2));
    assert!(nix::import(&longest).is_err());
    for (item, tokens, native) in [("__nxc_update(a, b) ", 6, false), ("(a // b) ", 5, true)] {
        let content = format!(
            "{}{}",
            item.repeat((MAX_TOKENS - 2) / tokens),
            "a ".repeat((MAX_TOKENS - 2) % tokens)
        );
        let source = format!("[{content}]");
        let ir = if native {
            nix::import(&source)
        } else {
            parse_nxc(&source)
        }
        .unwrap();
        assert!(
            emit::nxc(&ir).is_err(),
            "generated intrinsic and commas count toward token limits"
        );
        assert!(nix::emit(&ir).is_ok());
        let over = format!("[{content}a]");
        if native {
            assert!(nix::import(&over).is_err());
        } else {
            let parsed = syntax::parse(&over);
            assert_eq!(parsed.syntax().unwrap().to_string(), over);
            assert!(parsed.lower().is_err());
        }
    }
}

#[test]
fn native_nix_confirms_shallow_overrides_scope_and_strictness() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(output) => assert!(output.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    for (source, expected) in [
        ("{} // {}", Some("{}")),
        ("{ a = 1; } // { a = 2; }", Some(r#"{"a":2}"#)),
        (
            "{ a.x = 1; left = 3; } // { a.y = 2; right = 4; }",
            Some(r#"{"a":{"y":2},"left":3,"right":4}"#),
        ),
        ("({ a = 1 / 0; } // { a = 2; }).a", Some("2")),
        (
            "builtins.attrNames ({ a = 1 / 0; } // { b = 1 / 0; })",
            Some(r#"["a","b"]"#),
        ),
        ("({ bad = 1 / 0; } // { good = 2; }).good", Some("2")),
        ("(rec { a = 1; b = a; } // { a = 2; }).b", Some("1")),
        (
            "let x = 1; in ({ a = x; } // (let x = 2; in { b = x; }))",
            Some(r#"{"a":1,"b":2}"#),
        ),
        ("let builtins = 1; in ({} // { a = 2; }).a", Some("2")),
        ("({} // { __nxc_update = x: x; }).__nxc_update 3", Some("3")),
        ("({} // { __functor = self: x: x + 1; }) 2", Some("3")),
        ("{ a = 1; } // { a = 2; } // { a = 3; }", Some(r#"{"a":3}"#)),
        ("1 // {}", None),
        ("{} // 1", None),
        ("null // {}", None),
        ("{} // []", None),
        ("{} // (x: x)", None),
        ("(assert false; {}) // {}", None),
        ("{} // (assert false; {})", None),
        ("({ a = 1; } // (assert false; {})).a", None),
        ("((assert false; {}) // { a = 1; }).a", None),
        ("(x: 1) ({} // (assert false; {}))", Some("1")),
        ("if false then (1 // {}) else 2", Some("2")),
        (
            "(builtins.tryEval ((builtins.throw \"left\") // (builtins.abort \"right forced\"))).success",
            None,
        ),
        (
            "(builtins.tryEval ((builtins.abort \"left forced\") // (builtins.throw \"right\"))).success",
            Some("false"),
        ),
    ] {
        let ir = nix::import(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let generated = nix::emit(&parse_nxc(&emit::nxc(&ir).unwrap()).unwrap()).unwrap();
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
