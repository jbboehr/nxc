use nxc::{
    emit,
    ir::{Binding, Expr},
    nix, parse_nxc,
};
use std::{io::ErrorKind, process::Command};

#[test]
fn non_nul_control_characters_remain_static_keys() {
    let name = (1..=31)
        .chain(127..=159)
        .map(|codepoint| char::from_u32(codepoint).unwrap())
        .collect::<String>();
    let expr = Expr::Select {
        value: Box::new(Expr::AttrSet {
            recursive: false,
            bindings: vec![Binding::Assign {
                path: vec![name.clone()],
                value: Expr::Integer(1),
            }],
        }),
        path: vec![name],
        default: None,
    };

    let nxc_source = emit::nxc(&expr).unwrap();
    assert_eq!(parse_nxc(&nxc_source).unwrap(), expr);

    let nix_source = nix::emit(&expr).unwrap();
    assert_eq!(nix::import(&nix_source).unwrap(), expr);

    match Command::new("nix-instantiate").arg("--version").output() {
        Ok(result) => assert!(result.status.success()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            eprintln!("skipping native Nix oracle: nix-instantiate is unavailable");
            return;
        }
        Err(error) => panic!("cannot start Nix: {error}"),
    }
    let output = Command::new("nix-instantiate")
        .args([
            "--store",
            "dummy://",
            "--eval",
            "--strict",
            "--json",
            "--expr",
            &nix_source,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "1");
}
