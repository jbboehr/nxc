use nxc::{
    MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS, emit,
    ir::{BinaryOp, Expr},
    nix, parse_nxc, syntax,
};
use std::{io::ErrorKind, process::Command};

fn roundtrip(source: &str, native: &str) -> Expr {
    let parsed = syntax::parse(source);
    assert_eq!(parsed.syntax().unwrap().to_string(), source);
    let ir = parsed.lower().unwrap_or_else(|e| panic!("{source}: {e:?}"));
    assert_eq!(nix::import(native).unwrap(), ir, "{native}");
    let converted = emit::nxc(&ir).unwrap();
    let reparsed = parse_nxc(&converted).unwrap();
    assert_eq!(reparsed, ir, "{converted}");
    let output = nix::emit(&reparsed).unwrap();
    assert_eq!(nix::import(&output).unwrap(), ir, "{output}");
    assert_eq!(emit::nxc(&reparsed).unwrap(), converted);
    ir
}

#[test]
fn literal_relative_paths_preserve_their_spelling_in_both_directions() {
    for path in [
        "./foo",
        "../foo",
        "foo/bar",
        "1/2",
        "./.",
        "../..",
        "./a/../b",
        "a-b/c+d.nix",
        "a++b/c",
        "./a++b",
        "./a.b",
        "+/b",
        "-./b",
        "./.../_A-1+2",
    ] {
        let parsed = syntax::parse(path);
        assert_eq!(parsed.syntax().unwrap().to_string(), path);
        let ir = parsed.lower().unwrap();
        assert_eq!(ir, Expr::RelativePath(path.into()));
        assert_eq!(nix::import(path).unwrap(), ir);
        let converted = emit::nxc(&ir).unwrap();
        let native = nix::emit(&parse_nxc(&converted).unwrap()).unwrap();
        assert_eq!(converted, format!("({path})"));
        assert_eq!(native, format!("({path})"));
        assert_eq!(nix::import(&native).unwrap(), ir);
    }
}

#[test]
fn paths_keep_their_boundaries_and_compose_with_other_expressions() {
    for (source, native) in [
        ("# before\n ./foo # after", "/* before */ ./foo # after"),
        ("[./foo, ../bar, a/b, 1/2]", "[./foo ../bar a/b 1/2]"),
        ("f(./foo, ../bar)", "f ./foo ../bar"),
        ("./f(1)", "./f 1"),
        ("(./foo).a", "(./foo).a"),
        ("s.a or ./fallback", "s.a or ./fallback"),
        ("- (./foo)", "- (./foo)"),
        ("!./foo", "!./foo"),
        ("./foo + \"/bar\"", "./foo + \"/bar\""),
        ("x / ./foo", "x / ./foo"),
        ("./foo / x", "./foo / x"),
        ("1 / 2", "1 / 2"),
        ("./foo == ./bar", "./foo == ./bar"),
        ("false && ./missing", "false && ./missing"),
        ("[./foo] ++ [../bar]", "[./foo] ++ [../bar]"),
        ("{ value = ./foo; }.value", "{ value = ./foo; }.value"),
        ("rec { a = ./foo; b = a; }", "rec { a = ./foo; b = a; }"),
        ("{ inherit (./foo) a; }", "{ inherit (./foo) a; }"),
        ("let { x = ./foo; yield x; }", "let x = ./foo; in x"),
        ("with(./foo, 1)", "with ./foo; 1"),
        ("assert(true, ./foo)", "assert true; ./foo"),
        ("if true then ./a else ./b", "if true then ./a else ./b"),
        ("fn({ x ? ./foo }) => x", "{ x ? ./foo }: x"),
        ("fn(x) => ./foo", "x: ./foo"),
        ("\"${./foo}\"", "\"${./foo}\""),
        ("''${./foo}''", "''${./foo}''"),
    ] {
        roundtrip(source, native);
    }
    assert_eq!(
        roundtrip("-./foo", "-./foo"),
        Expr::RelativePath("-./foo".into())
    );
    assert_eq!(
        roundtrip("-(./foo)", "-(./foo)"),
        Expr::Negate(Box::new(Expr::RelativePath("./foo".into())))
    );
    assert!(matches!(
        roundtrip("1 / 2", "1 / 2"),
        Expr::Binary {
            op: BinaryOp::Divide,
            ..
        }
    ));
    // nxc's // comment syntax is unaffected where a path has not started.
    assert_eq!(parse_nxc("a//comment").unwrap(), Expr::Variable("a".into()));
}

#[test]
fn invalid_and_deferred_paths_are_rejected_and_recovery_retains_later_items() {
    for source in [
        "./",
        "../",
        "./foo/",
        "./foo//bar",
        "./a/*comment*/",
        "/foo",
        "~/foo",
        "<nixpkgs>",
        "./foo${x}",
        "./${x}/foo",
        "a/${x}",
        "./foo\\bar",
        "./fóo",
        ".../foo",
    ] {
        let parsed = syntax::parse(source);
        assert_eq!(parsed.syntax().unwrap().to_string(), source);
        assert!(parsed.lower().is_err(), "accepted nxc {source}");
        assert!(nix::import(source).is_err(), "accepted native {source}");
    }
    for source in [
        "f(./bad/, h(1))",
        "[./bad/, h(1)]",
        "{ a = ./bad/; b = h(1); }",
    ] {
        let parsed = syntax::parse(source);
        let root = parsed.syntax().unwrap();
        assert_eq!(root.to_string(), source);
        assert!(parsed.lower().is_err());
        assert!(
            root.descendants()
                .any(|node| node.kind() == syntax::SyntaxKind::CallExpr && node.text() == "h(1)")
        );
    }
}

#[test]
fn emitters_validate_path_spelling_and_resource_limits() {
    for path in [
        "",
        "foo",
        "/foo",
        "~/foo",
        "<foo>",
        "./",
        "./a/",
        "./a//b",
        "./a${x}",
        "./foo bar",
        "./foo\0",
        "./fóo",
        ".../foo",
        "./a); abort \"x\"",
    ] {
        let ir = Expr::RelativePath(path.into());
        assert!(emit::nxc(&ir).is_err(), "accepted {path:?}");
        assert!(nix::emit(&ir).is_err(), "accepted {path:?}");
    }
    let path = format!("./{}", "a".repeat(MAX_SOURCE_BYTES - 4));
    let ir = Expr::RelativePath(path.clone());
    for output in [emit::nxc(&ir).unwrap(), nix::emit(&ir).unwrap()] {
        assert_eq!(output.len(), MAX_SOURCE_BYTES);
        assert_eq!(parse_nxc(&output).unwrap(), ir);
        assert_eq!(nix::import(&output).unwrap(), ir);
    }
    let too_large = Expr::RelativePath(format!("{path}a"));
    assert!(emit::nxc(&too_large).is_err());
    assert!(nix::emit(&too_large).is_err());
    let too_large = format!("{path}aaa");
    assert!(syntax::parse(&too_large).syntax().is_none());
    assert!(nix::import(&too_large).is_err());

    let path = Expr::RelativePath("./a".into());
    let mut items = vec![path.clone(); (MAX_TOKENS - 1) / 4];
    // Each parenthesized path plus comma takes four tokens; an empty final list
    // fills the remaining three tokens to reach the exact nxc output ceiling.
    items.push(Expr::List(vec![]));
    let exact = Expr::List(items);
    let output = emit::nxc(&exact).unwrap();
    assert_eq!(
        syntax::lexer::lex(&output)
            .iter()
            .filter(|token| !token.kind.is_trivia())
            .count(),
        MAX_TOKENS
    );
    assert_eq!(parse_nxc(&output).unwrap(), exact);
    assert!(emit::nxc(&Expr::List(vec![path.clone(); (MAX_TOKENS - 1) / 4 + 1])).is_err());
    let mut items = vec![path.clone(); (MAX_TOKENS - 2) / 3];
    items.extend(vec![Expr::Integer(1); (MAX_TOKENS - 2) % 3]);
    let exact = Expr::List(items.clone());
    let output = nix::emit(&exact).unwrap();
    assert_eq!(
        syntax::lexer::lex(&output)
            .iter()
            .filter(|token| !token.kind.is_trivia())
            .count(),
        MAX_TOKENS
    );
    assert_eq!(nix::import(&output).unwrap(), exact);
    items.push(Expr::Integer(1));
    assert!(nix::emit(&Expr::List(items)).is_err());
    let nested = (0..MAX_DEPTH - 1).fold(path, |inner, _| Expr::Not(Box::new(inner)));
    roundtrip(&emit::nxc(&nested).unwrap(), &nix::emit(&nested).unwrap());
    assert!(emit::nxc(&Expr::Not(Box::new(nested))).is_err());
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

#[test]
fn native_nix_confirms_path_values_lexical_boundaries_and_laziness() {
    if !nix_available() {
        return;
    }
    for (source, expected) in [
        ("builtins.typeOf ./does-not-exist", Some("\"path\"")),
        ("builtins.typeOf 1/2", Some("\"path\"")),
        ("builtins.typeOf (1 / 2)", Some("\"int\"")),
        ("builtins.isPath -./foo", Some("true")),
        ("builtins.isPath +/foo", Some("true")),
        ("builtins.isPath ++/foo", Some("true")),
        ("./a/../b == ./b", Some("true")),
        ("././foo == ./foo", Some("true")),
        ("./foo == ./bar", Some("false")),
        ("builtins.baseNameOf ./foo", Some("\"foo\"")),
        ("builtins.dirOf ./foo == ./.", Some("true")),
        ("toString ./foo == (toString ./.) + \"/foo\"", Some("true")),
        ("builtins.hasContext (toString ./foo)", Some("false")),
        (
            "toString (./foo + \"/bar\") == toString ./foo/bar",
            Some("true"),
        ),
        ("if true then 3 else import ./does-not-exist", Some("3")),
        ("with ./does-not-exist; 1", Some("1")),
        (
            "builtins.head [./foo (abort \"unused\")] == ./foo",
            Some("true"),
        ),
        ("({ x ? ./foo }: x) {} == ./foo", Some("true")),
        ("false -> ./does-not-exist", Some("true")),
        ("assert ./foo; 1", None),
        ("./foo + 1", None),
        ("-(./foo)", None),
    ] {
        let ir = nix::import(source).unwrap();
        let converted = emit::nxc(&ir).unwrap();
        let reparsed = parse_nxc(&converted).unwrap();
        assert_eq!(reparsed, ir);
        let native = nix::emit(&reparsed).unwrap();
        assert_eq!(nix::import(&native).unwrap(), ir);
        for value in [source, &native] {
            let output = Command::new("nix-instantiate")
                .args([
                    "--store",
                    "dummy://",
                    "--eval",
                    "--strict",
                    "--json",
                    "--expr",
                    &format!("({value})"),
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
