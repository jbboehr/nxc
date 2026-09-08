use nxc::{
    MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, emit,
    ir::{AttrName, BinaryOp, Binding, Expr, Float, Formal, Pattern, StringPart},
    nix, parse_nxc, syntax,
};
use proptest::prelude::*;

fn attribute_names() -> impl Strategy<Value = String> {
    prop::collection::vec(
        any::<char>().prop_filter("Nix names exclude NUL", |c| *c != '\0'),
        0..16,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

fn expressions() -> impl Strategy<Value = Expr> {
    prop_oneof![
        prop::collection::vec(
            any::<char>().prop_filter("Nix strings exclude NUL", |c| *c != '\0'),
            0..24
        )
        .prop_map(|chars| {
            let text: String = chars.into_iter().collect();
            Expr::String(if text.is_empty() {
                vec![]
            } else {
                vec![StringPart::Literal(text)]
            })
        }),
        (0u64..100_000).prop_map(Expr::Integer),
        (0u64..0x7ff0_0000_0000_0000)
            .prop_map(|bits| Expr::Float(Float::new(f64::from_bits(bits)).unwrap())),
        (
            prop::sample::select(vec!["./", "../", "dir/"]),
            "[a-zA-Z0-9_+.-]{1,16}"
        )
            .prop_map(|(prefix, name)| Expr::RelativePath(format!("{prefix}{name}"))),
        "[a-zA-Z0-9_+.-]{1,16}(/[a-zA-Z0-9_+.-]{1,16}){0,3}"
            .prop_map(|name| Expr::SearchPath(format!("<{name}>"))),
        "[a-zA-Z0-9_+.-]{1,16}(/[a-zA-Z0-9_+.-]{1,16}){0,3}"
            .prop_map(|name| Expr::AbsolutePath(format!("/{name}"))),
        "[a-zA-Z0-9_+.-]{1,16}(/[a-zA-Z0-9_+.-]{1,16}){0,3}"
            .prop_map(|name| Expr::HomePath(format!("~/{name}"))),
        prop::sample::select(vec!["f", "x", "g", "foo-bar'", "true", "false", "null"])
            .prop_map(|name| Expr::Variable(name.into())),
    ]
    .prop_recursive(5, 64, 3, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(condition, body)| Expr::Assert {
                condition: Box::new(condition),
                body: Box::new(body),
            }),
            (inner.clone(), inner.clone(), inner.clone()).prop_map(
                |(condition, then_branch, else_branch)| Expr::If {
                    condition: Box::new(condition),
                    then_branch: Box::new(then_branch),
                    else_branch: Box::new(else_branch),
                }
            ),
            (inner.clone(), inner.clone()).prop_map(|(scope, body)| Expr::With {
                scope: Box::new(scope),
                body: Box::new(body),
            }),
            (inner.clone(), inner.clone()).prop_map(|(value, body)| Expr::Let {
                bindings: vec![Binding::Assign {
                    path: vec!["local".into(), "yield".into()],
                    value,
                }],
                body: Box::new(body),
            }),
            prop::collection::vec(inner.clone(), 0..4).prop_map(Expr::List),
            (
                prop::sample::select(vec!["./", "../a/", "/", "/a/../", "~/a/../"]),
                inner.clone()
            )
                .prop_map(|(prefix, value)| Expr::InterpolatedPath(vec![
                    StringPart::Literal(prefix.into()),
                    StringPart::Interpolation(value),
                    StringPart::Literal("/suffix".into()),
                ])),
            inner.clone().prop_map(|value| Expr::String(vec![
                StringPart::Literal("prefix$".into()),
                StringPart::Interpolation(value),
                StringPart::Literal("${suffix}\\\"".into()),
            ])),
            (
                inner.clone(),
                any::<bool>(),
                prop::collection::vec(
                    prop_oneof![
                        attribute_names().prop_map(AttrName::Static),
                        inner
                            .clone()
                            .prop_map(|key| AttrName::Dynamic(Box::new(key))),
                    ],
                    1..4
                )
            )
                .prop_map(|(value, recursive, path)| Expr::AttrSet {
                    recursive,
                    bindings: vec![Binding::Assign { path, value }],
                }),
            (inner.clone(), attribute_names()).prop_map(|(source, name)| Expr::AttrSet {
                recursive: false,
                bindings: vec![Binding::Inherit {
                    source: Some(source),
                    names: vec![name]
                }],
            }),
            (
                inner.clone(),
                prop::option::of(inner.clone()),
                prop::collection::vec(
                    prop_oneof![
                        attribute_names().prop_map(AttrName::Static),
                        inner
                            .clone()
                            .prop_map(|key| AttrName::Dynamic(Box::new(key))),
                    ],
                    1..4
                )
            )
                .prop_map(|(value, default, path)| {
                    Expr::Select {
                        value: Box::new(value),
                        path,
                        default: default.map(Box::new),
                    }
                }),
            (
                inner.clone(),
                prop::collection::vec(
                    prop_oneof![
                        attribute_names().prop_map(AttrName::Static),
                        inner
                            .clone()
                            .prop_map(|key| AttrName::Dynamic(Box::new(key))),
                    ],
                    1..4,
                ),
            )
                .prop_map(|(value, path)| Expr::HasAttr {
                    value: Box::new(value),
                    path,
                }),
            inner.clone().prop_map(|body| Expr::Lambda {
                parameter: Pattern::Ident("x".into()),
                body: Box::new(body),
            }),
            (inner.clone(), inner.clone(), any::<bool>(), any::<bool>()).prop_map(
                |(default, body, ellipsis, capture)| Expr::Lambda {
                    parameter: Pattern::AttrSet {
                        fields: vec![
                            Formal {
                                name: "x".into(),
                                default: Some(default)
                            },
                            Formal {
                                name: "y".into(),
                                default: None
                            },
                        ],
                        ellipsis,
                        bind: capture.then(|| "args".into()),
                    },
                    body: Box::new(body),
                }
            ),
            inner.clone().prop_map(|e| Expr::Negate(Box::new(e))),
            inner.clone().prop_map(|e| Expr::Not(Box::new(e))),
            (inner.clone(), inner.clone()).prop_map(|(f, a)| Expr::Apply {
                function: Box::new(f),
                argument: Box::new(a)
            }),
            (
                prop::sample::select(vec![
                    BinaryOp::Add,
                    BinaryOp::Subtract,
                    BinaryOp::Multiply,
                    BinaryOp::Divide,
                    BinaryOp::Equal,
                    BinaryOp::NotEqual,
                    BinaryOp::Less,
                    BinaryOp::LessOrEqual,
                    BinaryOp::Greater,
                    BinaryOp::GreaterOrEqual,
                    BinaryOp::And,
                    BinaryOp::Or,
                    BinaryOp::Update,
                    BinaryOp::Concat,
                ]),
                inner.clone(),
                inner
            )
                .prop_map(|(op, l, r)| Expr::Binary {
                    op,
                    left: Box::new(l),
                    right: Box::new(r)
                }),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn arbitrary_text_is_lossless_and_diagnostics_have_valid_spans(source in prop::collection::vec(any::<char>(), 0..256).prop_map(|c| c.into_iter().collect::<String>())) {
        let parsed = syntax::parse(&source);
        prop_assert_eq!(parsed.syntax().unwrap().to_string(), source.as_str());
        let mut cursor = 0;
        for token in syntax::lexer::lex(&source) {
            prop_assert_eq!(token.span.start, cursor);
            prop_assert!(token.span.end > cursor);
            prop_assert!(source.is_char_boundary(token.span.end));
            cursor = token.span.end;
        }
        prop_assert_eq!(cursor, source.len());
        for errors in [parsed.lower().err(), nix::import(&source).err()].into_iter().flatten() {
            prop_assert!(!errors.is_empty());
            for error in errors {
                prop_assert!(error.span.start <= error.span.end && error.span.end <= source.len());
                prop_assert!(source.is_char_boundary(error.span.start));
                prop_assert!(source.is_char_boundary(error.span.end));
            }
        }
    }

    #[test]
    fn generated_semantic_expressions_survive_both_dialects(expr in expressions()) {
        let native = nix::emit(&expr).unwrap();
        let source = emit::nxc(&expr).unwrap();
        let from_nxc = parse_nxc(&source).unwrap();
        let from_native = nix::import(&native).unwrap();
        prop_assert_eq!(from_nxc.canonical(), expr.canonical());
        prop_assert_eq!(from_native.canonical(), expr.canonical());
        prop_assert_eq!(emit::nxc(&from_nxc).unwrap(), source);
        prop_assert_eq!(nix::emit(&from_native).unwrap(), native);
    }

    #[test]
    fn generated_native_implications_normalize_consistently(left in expressions(), right in expressions()) {
        let native = format!("({}) -> ({})", nix::emit(&left).unwrap(), nix::emit(&right).unwrap());
        let expected = Expr::Binary {
            op: BinaryOp::Or,
            left: Box::new(Expr::Not(Box::new(left))),
            right: Box::new(right),
        };
        let actual = nix::import(&native).unwrap();
        prop_assert_eq!(actual.canonical(), expected.canonical());
        prop_assert_eq!(parse_nxc(&emit::nxc(&actual).unwrap()).unwrap(), expected.clone());
        prop_assert_eq!(nix::import(&nix::emit(&actual).unwrap()).unwrap(), expected);
    }
}

#[test]
fn resource_limits_return_errors_and_preserve_cst_when_possible() {
    for source in [
        format!(
            "{}1{}",
            "(".repeat(MAX_DEPTH + 1),
            ")".repeat(MAX_DEPTH + 1)
        ),
        format!("{}1", "- ".repeat(MAX_TOKENS + 1)),
        format!("{}1", "- ".repeat(MAX_DEPTH + 1)),
    ] {
        let parsed = syntax::parse(&source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(nix::import(&source).is_err());
    }
    let too_large = " ".repeat(MAX_SOURCE_BYTES + 1);
    let parsed = syntax::parse(&too_large);
    assert!(parsed.syntax().is_none());
    assert!(parsed.lower().is_err());
    assert!(nix::import(&too_large).is_err());
}

#[test]
fn emitters_reject_invalid_ir_instead_of_emitting_different_semantics() {
    for expr in [
        Expr::Integer(u64::MAX),
        Expr::Variable("x: x".into()),
        Expr::Variable("fn".into()),
        Expr::Variable("__curPos".into()),
    ] {
        assert!(emit::nxc(&expr).is_err());
        assert!(nix::emit(&expr).is_err());
    }
}
