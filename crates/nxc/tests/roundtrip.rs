use nxc::{
    emit,
    ir::{BinaryOp, Expr},
    nix, parse_nxc,
};

fn var(name: &str) -> Expr {
    Expr::Variable(name.into())
}

fn apply(function: Expr, argument: Expr) -> Expr {
    Expr::Apply {
        function: Box::new(function),
        argument: Box::new(argument),
    }
}

#[test]
fn calls_lower_to_ordered_unary_applications() {
    let expected = apply(
        apply(
            var("f"),
            Expr::Binary {
                op: BinaryOp::Add,
                left: Box::new(Expr::Integer(1)),
                right: Box::new(Expr::Integer(2)),
            },
        ),
        var("x"),
    );
    assert_eq!(parse_nxc("f(1 + 2, x)").unwrap(), expected);
    assert_eq!(nix::import("f (1 + 2) x").unwrap(), expected);
    assert_eq!(emit::nxc(&expected).unwrap(), "f((1 + 2), x)");
    assert_eq!(
        nix::import(&nix::emit(&expected).unwrap()).unwrap(),
        expected
    );
}

#[test]
fn supported_expressions_roundtrip_in_both_directions() {
    let cases = [
        ("0", "0"),
        ("00042", "42"),
        ("9223372036854775807", "9223372036854775807"),
        ("true", "true"), // Nix's true, false and null remain variable references.
        ("foo-bar'", "foo-bar'"),
        ("f(g(x), y)", "f (g x) y"),
        ("f(x)(y)", "f x y"),
        ("(f + g)(x,)", "(f + g) x"),
        ("-f(x) * 2 + 3", "-(f x) * 2 + 3"),
        ("f(-1, --2)", "f (-1) (-(-2))"),
        ("1 + 2 * 3", "1 + (2 * 3)"),
        ("10 - 3 - 2", "(10 - 3) - 2"),
        ("12 / 3 * 2", "(12 / 3) * 2"),
        ("a / (b / c)", "a / (b / c)"),
        ("# before\nf /* callee */ (1, // arg\n x,) # after", "f 1 x"),
    ];
    for (source, native) in cases {
        let expected = nix::import(native).unwrap();
        let actual = parse_nxc(source).unwrap_or_else(|e| panic!("{source:?}: {e:?}"));
        assert_eq!(actual, expected, "{source}");
        let generated_nxc = emit::nxc(&expected).unwrap();
        assert_eq!(
            parse_nxc(&generated_nxc).unwrap(),
            expected,
            "{generated_nxc}"
        );
        let generated_nix = nix::emit(&actual).unwrap();
        assert_eq!(
            nix::import(&generated_nix).unwrap(),
            expected,
            "{generated_nix}"
        );
        assert_eq!(
            emit::nxc(&parse_nxc(&generated_nxc).unwrap()).unwrap(),
            generated_nxc
        );
    }
}

#[test]
fn malformed_and_unsupported_nxc_is_rejected() {
    for source in [
        "",
        " ",
        "f x",
        "f()",
        "f(,)",
        "f(1,,2)",
        "f(1",
        "1 +",
        "x: x",
        "let",
        "yield",
        "__nxc_unknown(a,b)",
        "__curPos",
        "1.5e+",
        "./foo/",
        "~/foo",
        "a/b/",
        "1/2/",
        "x -> y",
        "x +++ y",
        "9223372036854775808",
        "1 /* unterminated",
    ] {
        assert!(parse_nxc(source).is_err(), "accepted {source:?}");
    }
}

#[test]
fn unsupported_native_forms_are_never_guessed() {
    for source in [
        "",
        "f (",
        "1.5e+",
        "~/unsupported/path",
        "./foo/",
        "a +++ b",
        "a ->> b",
        "fn",
        "yield",
        "__nxc_update",
        "__curPos",
        "9223372036854775808",
    ] {
        assert!(nix::import(source).is_err(), "accepted {source:?}");
    }
}

#[test]
fn native_line_comments_with_bare_cr_are_rejected_before_lowering() {
    for source in [
        "1 # comment\r+ 2",
        "/* α */ 1 # β\r+ 2\n",
        "# first\r\n1 # second\r+ 2",
        "1 # comment\r\r\n+ 2",
        "1 # comment\r",
    ] {
        let errors = nix::import(source).expect_err("bare-CR line comment must be rejected");
        assert_eq!(errors.len(), 1);
        let error = &errors[0];
        assert_eq!(&source[error.span.clone()], "\r");
        assert_ne!(source.as_bytes().get(error.span.end), Some(&b'\n'));
        assert!(error.message.contains("line comment"), "{error:?}");
    }
}

#[test]
fn native_crlf_comments_and_cr_outside_line_comments_remain_supported() {
    let expected = Expr::Binary {
        op: BinaryOp::Add,
        left: Box::new(Expr::Integer(1)),
        right: Box::new(Expr::Integer(2)),
    };
    for source in [
        "1 # comment\n+ 2",
        "1 # comment\r\n+ 2",
        "1 # \u{000b}\u{000c}\u{00a0}\u{2003}\n+ 2",
        "1 /* comment\rstill inside */ + 2",
        "1 /* \u{000b}\u{000c}\u{00a0}\u{2003} */ + 2",
        "1\r+ 2",
    ] {
        assert_eq!(nix::import(source).unwrap(), expected, "{source:?}");
    }
}

#[test]
fn native_whitespace_rejected_by_nix_is_not_accepted_by_the_adapter() {
    for whitespace in ["\u{000b}", "\u{000c}", "\u{00a0}", "\u{2003}"] {
        let source = format!("/* α */ 1{whitespace}+ 2");
        let errors = nix::import(&source).expect_err("native Nix rejects this whitespace");
        assert_eq!(errors.len(), 1);
        assert_eq!(&source[errors[0].span.clone()], whitespace);
        assert!(errors[0].message.contains("whitespace"));
    }
}

#[test]
fn lexical_boundaries_preserve_names_and_distinguish_paths_from_arithmetic() {
    for name in [
        "_", "a-", "a--b", "foo-bar'", "true", "false", "null", "__nxc",
    ] {
        let expected = var(name);
        assert_eq!(parse_nxc(name).unwrap(), expected, "nxc identifier {name}");
        assert_eq!(
            nix::import(name).unwrap(),
            expected,
            "Nix identifier {name}"
        );
    }

    assert_eq!(parse_nxc("a-b").unwrap(), var("a-b"));
    assert_eq!(
        parse_nxc("a/* left */ - /* right */b").unwrap(),
        Expr::Binary {
            op: BinaryOp::Subtract,
            left: Box::new(var("a")),
            right: Box::new(var("b")),
        }
    );

    let plain = parse_nxc("f(1 + 2, x)").unwrap();
    assert_eq!(
        parse_nxc("f/* call */(1/* lhs */+/* rhs */2, // next\n x,)").unwrap(),
        plain
    );
    assert_eq!(
        nix::import("f /* call */ (1 /* lhs */ + /* rhs */ 2) # next\n x").unwrap(),
        plain
    );

    for keyword in [
        "assert", "else", "fn", "if", "in", "inherit", "let", "or", "rec", "then", "with", "yield",
    ] {
        assert!(
            parse_nxc(keyword).is_err(),
            "accepted nxc keyword {keyword}"
        );
        assert!(
            nix::import(keyword).is_err(),
            "accepted native keyword {keyword}"
        );
    }
    assert_eq!(parse_nxc("/x").unwrap(), Expr::AbsolutePath("/x".into()));
    assert_eq!(nix::import("/x").unwrap(), Expr::AbsolutePath("/x".into()));
    for path in ["~/x", "/x/", "./x/", "../x//y"] {
        assert!(
            parse_nxc(path).is_err(),
            "guessed path {path} as arithmetic"
        );
        assert!(
            nix::import(path).is_err(),
            "accepted unsupported path {path}"
        );
    }
}
