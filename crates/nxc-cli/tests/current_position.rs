use std::{fs, io::ErrorKind, path::Path, process::Command};
use tempfile::TempDir;

fn convert(dir: &Path, source: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let input = dir.join("input.nix");
    let converted = dir.join("converted.nxc");
    let output = dir.join("output.nix");
    fs::write(&input, source).unwrap();
    for (command, from, to) in [
        ("from-nix", &input, &converted),
        ("to-nix", &converted, &output),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_nxc"))
            .arg(command)
            .arg(from)
            .arg("-o")
            .arg(to)
            .output()
            .unwrap();
        assert!(result.status.success(), "{command}: {result:?}");
    }
    let checked = Command::new(env!("CARGO_BIN_EXE_nxc"))
        .arg("check")
        .arg(&converted)
        .output()
        .unwrap();
    assert!(checked.status.success(), "{checked:?}");
    assert_eq!(fs::read_to_string(&input).unwrap(), source);
    assert_eq!(
        nxc::nix::import(&fs::read_to_string(&output).unwrap()).unwrap(),
        nxc::nix::import(source).unwrap()
    );
    (input, output)
}

fn has_nix() -> bool {
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

fn evaluate(file: &Path) -> String {
    let output = Command::new("nix-instantiate")
        .args(["--store", "dummy://", "--eval", "--strict", "--json"])
        .arg(file)
        .output()
        .unwrap();
    assert!(output.status.success(), "{file:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn current_position_reports_the_generated_file_and_coordinates() {
    let dir = TempDir::new().unwrap();
    let (input, output) = convert(dir.path(), "\n\n  __curPos\n");
    assert_eq!(fs::read_to_string(&output).unwrap(), "__curPos\n");
    if !has_nix() {
        return;
    }
    assert_eq!(
        evaluate(&input),
        format!(r#"{{"column":3,"file":"{}","line":3}}"#, input.display())
    );
    assert_eq!(
        evaluate(&output),
        format!(r#"{{"column":1,"file":"{}","line":1}}"#, output.display())
    );
    let relocated = dir.path().join("relocated.nix");
    fs::write(
        &relocated,
        format!("\n\n    {}", fs::read_to_string(&output).unwrap()),
    )
    .unwrap();
    assert_eq!(
        evaluate(&relocated),
        format!(
            r#"{{"column":5,"file":"{}","line":3}}"#,
            relocated.display()
        )
    );
}

#[test]
fn each_current_position_reports_its_own_generated_location() {
    let dir = TempDir::new().unwrap();
    let (_, output) = convert(dir.path(), "{ first = __curPos; nested = [ __curPos ]; }");
    if !has_nix() {
        return;
    }
    let generated = fs::read_to_string(&output).unwrap();
    let offsets: Vec<_> = generated
        .match_indices("__curPos")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(offsets.len(), 2, "generated Nix: {generated}");
    let position = |offset| {
        let prefix = &generated[..offset];
        let line = prefix.bytes().filter(|&byte| byte == b'\n').count() + 1;
        let column = prefix.rsplit('\n').next().unwrap().chars().count() + 1;
        format!(
            r#"{{"column":{column},"file":"{}","line":{line}}}"#,
            output.display()
        )
    };
    assert_eq!(
        evaluate(&output),
        format!(
            r#"{{"first":{},"nested":[{}]}}"#,
            position(offsets[0]),
            position(offsets[1])
        )
    );
}

#[test]
fn current_position_remains_consistent_with_native_attribute_locations() {
    let dir = TempDir::new().unwrap();
    let source = r#"let
        __curPos = 7;
        x = { a = 1; };
    in [
        ((builtins.unsafeGetAttrPos "a" x).file == __curPos.file)
        (builtins.getContext __curPos.file)
        ({ inherit __curPos; }.__curPos)
        ((__curPos: __curPos.file) 99 == __curPos.file)
        (with { __curPos = 99; }; __curPos.file == (builtins.unsafeGetAttrPos "a" x).file)
    ]"#;
    let (input, output) = convert(dir.path(), source);
    if !has_nix() {
        return;
    }
    for file in [&input, &output] {
        assert_eq!(evaluate(file), "[true,{},7,true,true]");
    }
}
