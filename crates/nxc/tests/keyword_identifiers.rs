use nxc::{
    MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, emit,
    ir::{Expr, Formal, Pattern, StringPart},
    nix, parse_nxc, syntax,
};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let expected = nix::import(native).unwrap_or_else(|e| panic!("{native}: {e:?}"));
    let actual = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    assert_eq!(actual, expected);
    let converted = emit::nxc(&expected).unwrap();
    let reparsed = parse_nxc(&converted).unwrap();
    assert_eq!(reparsed, expected);
    let generated = nix::emit(&reparsed).unwrap();
    assert_eq!(nix::import(&generated).unwrap(), expected);
    assert_eq!(emit::nxc(&reparsed).unwrap(), converted);
}

#[test]
fn compatibility_identifiers_preserve_native_names_in_every_binding_scope() {
    for (source, native) in [
        ("__nxc_ident_fn", "fn"),
        ("__nxc_ident_yield", "yield"),
        ("__nxc_ident_fn(x)", "fn x"),
        ("__nxc_ident_fn => __nxc_ident_fn", "fn: fn"),
        ("fn(__nxc_ident_yield) => __nxc_ident_yield", "yield: yield"),
        (
            "fn({ __nxc_ident_fn, __nxc_ident_yield ? __nxc_ident_fn }) => __nxc_ident_yield",
            "{ fn, yield ? fn }: yield",
        ),
        (
            "(__nxc_ident_fn@{ x }) => __nxc_ident_fn.x",
            "fn@{ x }: fn.x",
        ),
        (
            "({ x }@__nxc_ident_yield) => __nxc_ident_yield.x",
            "{ x }@yield: yield.x",
        ),
        ("let { fn = 1; yield __nxc_ident_fn; }", "let fn = 1; in fn"),
        (
            "let { \"yield\" = 2; yield __nxc_ident_yield; }",
            "let yield = 2; in yield",
        ),
        (
            "let { \"yield\".fn = 2; yield __nxc_ident_yield.fn; }",
            "let yield.fn = 2; in yield.fn",
        ),
        (
            "let { ${\"yield\"} = 2; yield __nxc_ident_yield; }",
            "let ${\"yield\"} = 2; in yield",
        ),
        (
            "rec { fn = 1; yield = __nxc_ident_fn; }",
            "rec { fn = 1; yield = fn; }",
        ),
        ("__nxc_ident_fn => { inherit fn; }", "fn: { inherit fn; }"),
        (
            "__nxc_ident_yield => rec { inherit yield; x = __nxc_ident_yield; }",
            "yield: rec { inherit yield; x = yield; }",
        ),
        (
            "let { inherit (s) fn yield; yield [__nxc_ident_fn, __nxc_ident_yield]; }",
            "let inherit (s) fn yield; in [ fn yield ]",
        ),
        (
            "with(s, __nxc_ident_fn(__nxc_ident_yield))",
            "with s; fn yield",
        ),
        ("s.a or __nxc_ident_fn", "s.a or fn"),
        (
            "{ ${__nxc_ident_fn} = __nxc_ident_yield; }",
            "{ ${fn} = yield; }",
        ),
        ("\"${__nxc_ident_fn}\"", "\"${fn}\""),
        ("./${__nxc_ident_yield}", "./${yield}"),
    ] {
        roundtrip(source, native);
    }
    for source in [
        "{ fn = 1; yield = 2; }",
        "s.fn",
        "s.yield",
        "s.__nxc_ident_fn",
        "{ __nxc_ident_yield = 2; }",
        "{ inherit (s) __nxc_ident_fn; }",
        r#""__nxc_ident_fn __nxc_ident_yield""#,
    ] {
        roundtrip(source, source);
    }
}

#[test]
fn invalid_aliases_and_duplicate_decoded_parameters_remain_errors() {
    for source in [
        "fn",
        "yield",
        "fn => 1",
        "yield => 1",
        "fn(fn) => 1",
        "({ yield }) => 1",
        "__nxc_ident",
        "__nxc_ident_or",
        "__nxc_ident_fn_extra",
        "__nxc_ident_fn-suffix",
        "__nxc_ident_fn'",
        "__nxc_ident___curPos",
        "__nxc_update",
        "({ __nxc_ident_fn, __nxc_ident_fn }) => 1",
        "(__nxc_ident_yield@{ __nxc_ident_yield }) => 1",
        "let { __nxc_ident_fn = 1; yield 1; }",
        "{ inherit __nxc_ident_yield; }",
        "let { yield = 1; yield __nxc_ident_yield; }",
        "let { fn = 1; \"fn\" = 2; yield __nxc_ident_fn; }",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted nxc {source}");
    }
    for source in [
        "__nxc_ident_fn",
        "__nxc_ident_yield",
        "__nxc_ident_fn: 1",
        "{ __nxc_ident_yield }: 1",
        "let __nxc_ident_fn = 1; in 1",
        "or",
        "__curPos",
        "{ fn, fn }: 1",
        "yield@{ yield }: 1",
    ] {
        assert!(nix::import(source).is_err(), "accepted native {source}");
    }
    for source in [
        "f(__nxc_ident_bad, good(1))",
        "[__nxc_ident_fn +, good(1)]",
        "let { x = __nxc_ident_bad; yield good(1); }",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(
            root.descendants()
                .any(|n| n.kind() == syntax::SyntaxKind::CallExpr && n.text() == "good(1)"),
            "{source}: {root:#?}"
        );
    }
}

#[test]
fn public_ir_keeps_original_parameter_names_and_attribute_keys() {
    let ir = Expr::Lambda {
        parameter: Pattern::AttrSet {
            fields: vec![Formal {
                name: "fn".into(),
                default: None,
            }],
            ellipsis: false,
            bind: Some("yield".into()),
        },
        body: Box::new(Expr::Variable("fn".into())),
    };
    assert_eq!(parse_nxc(&emit::nxc(&ir).unwrap()).unwrap(), ir);
    assert_eq!(nix::import(&nix::emit(&ir).unwrap()).unwrap(), ir);
    assert_eq!(
        parse_nxc("__nxc_ident_fn").unwrap(),
        Expr::Variable("fn".into())
    );
    for name in [
        "__nxc_ident_fn",
        "__nxc_ident_yield",
        "__nxc_unknown",
        "__curPos",
        "or",
    ] {
        let invalid = Expr::Variable(name.into());
        assert!(emit::nxc(&invalid).is_err());
        assert!(nix::emit(&invalid).is_err());
    }
}

#[test]
fn identifier_expansion_obeys_output_bytes_tokens_and_depth() {
    for keyword in ["fn", "yield"] {
        for value in [
            Expr::Variable(keyword.into()),
            Expr::Lambda {
                parameter: Pattern::AttrSet {
                    fields: vec![Formal {
                        name: keyword.into(),
                        default: None,
                    }],
                    ellipsis: false,
                    bind: Some("args".into()),
                },
                body: Box::new(Expr::Variable(keyword.into())),
            },
        ] {
            let empty = Expr::List(vec![value.clone(), Expr::String(vec![])]);
            let overhead = emit::nxc(&empty).unwrap().len();
            let sized = |extra| {
                Expr::List(vec![
                    value.clone(),
                    Expr::String(vec![StringPart::Literal(
                        "a".repeat(MAX_SOURCE_BYTES - overhead + extra),
                    )]),
                ])
            };
            let exact = sized(0);
            let output = emit::nxc(&exact).unwrap();
            assert_eq!(output.len(), MAX_SOURCE_BYTES);
            assert_eq!(parse_nxc(&output).unwrap(), exact);
            let over = sized(1);
            // Native names fit; expanding them must still respect the nxc bound.
            assert_eq!(nix::import(&nix::emit(&over).unwrap()).unwrap(), over);
            assert!(emit::nxc(&over).is_err());
        }
    }
    let mut items = vec![Expr::Variable("yield".into()); (MAX_TOKENS - 1) / 2];
    let ir = Expr::List(items.clone());
    assert_eq!(parse_nxc(&emit::nxc(&ir).unwrap()).unwrap(), ir);
    items.push(Expr::Variable("yield".into()));
    let over = Expr::List(items);
    assert!(nix::emit(&over).is_ok());
    assert!(emit::nxc(&over).is_err());

    let source = format!(
        "{}__nxc_ident_fn",
        "__nxc_ident_fn => ".repeat(MAX_DEPTH - 1)
    );
    let ir = parse_nxc(&source).unwrap();
    assert_eq!(parse_nxc(&emit::nxc(&ir).unwrap()).unwrap(), ir);
    assert_eq!(nix::import(&nix::emit(&ir).unwrap()).unwrap(), ir);
    let parsed = syntax::parse(&format!("__nxc_ident_fn => {source}"));
    assert!(parsed.syntax().is_some());
    assert!(parsed.lower().is_err());
}

#[test]
fn native_nix_confirms_keyword_lookup_shadowing_and_function_arguments() {
    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(e) => panic!("cannot start Nix: {e}"),
    }
    for (source, expected) in [
        ("(fn: fn + 1) 6", "7"),
        ("(yield: (fn: fn + yield) 3) 4", "7"),
        ("let fn = 1; in (fn: fn) 2", "2"),
        (
            "let fn = n: if n == 0 then 1 else n * fn (n - 1); in fn 5",
            "120",
        ),
        ("let fn = 7; yield = fn; in yield", "7"),
        (
            "let fn = 7; in with { fn = 99; __nxc_ident_fn = 123; }; fn",
            "7",
        ),
        ("with { fn = 7; __nxc_ident_fn = 123; }; fn", "7"),
        ("with { fn = 3; }; with { fn = 7; }; fn", "7"),
        ("let fn = 7; in rec { inherit fn; yield = fn; }.yield", "7"),
        ("let fn = 1; in rec { fn = 7; yield = fn; }.yield", "7"),
        ("(yield: let inherit yield; in yield) 7", "7"),
        ("let inherit ({ fn = 7; }) fn; in fn", "7"),
        ("({ fn, yield ? fn }: yield) { fn = 7; }", "7"),
        ("({ fn, yield ? fn }: yield) { fn = 7; yield = 9; }", "9"),
        ("({ fn ? yield, yield ? 7 }: fn) {}", "7"),
        ("({ fn ? 7 }@yield: [ fn (yield ? fn) ]) {}", "[7,false]"),
        (
            "let fn = 5; in ({ yield ? fn }@fn: [ yield fn ]) {}",
            "[{},{}]",
        ),
        (
            "let yield = 5; in ({ fn ? yield }: (yield: [ fn yield ]) 9) {}",
            "[5,9]",
        ),
        (
            r#"builtins.functionArgs ({ fn, yield ? 1 }: fn)"#,
            r#"{"fn":false,"yield":true}"#,
        ),
        (
            r#"builtins.attrNames (rec { fn = 1; yield = fn; __nxc_ident_fn = 2; })"#,
            r#"["__nxc_ident_fn","fn","yield"]"#,
        ),
        (r#"let fn = 7; in { inherit ${"fn"}; }.fn"#, "7"),
        (r#"({ fn ? abort "unused" }: 7) {}"#, "7"),
        (r#"let yield = abort "unused"; in 7"#, "7"),
    ] {
        let ir = nix::import(source).unwrap_or_else(|e| panic!("{source}: {e:?}"));
        let nxc = emit::nxc(&ir).unwrap();
        let generated = nix::emit(&parse_nxc(&nxc).unwrap()).unwrap();
        for text in [source, &generated] {
            let output = Command::new("nix-instantiate")
                .args([
                    "--store", "dummy://", "--eval", "--strict", "--json", "--expr",
                ])
                .arg(text)
                .output()
                .unwrap();
            assert!(output.status.success(), "{text}: {output:?}");
            assert_eq!(
                String::from_utf8(output.stdout).unwrap().trim(),
                expected,
                "{text}"
            );
        }
    }
}
