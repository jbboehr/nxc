use nxc::{
    ir::{BinaryOp, Binding, Expr},
    nix, parse_nxc,
};

fn literal_set(recursive: bool, name: &str, value: u64) -> Expr {
    Expr::AttrSet {
        recursive,
        bindings: vec![Binding::Assign {
            path: vec![name.into()],
            value: Expr::Integer(value),
        }],
    }
}

#[test]
fn literal_attrset_updates_remain_binary_in_both_frontends() {
    let expected = Expr::Binary {
        op: BinaryOp::Update,
        left: Box::new(literal_set(false, "shared", 1)),
        right: Box::new(literal_set(true, "shared", 2)),
    };

    assert_eq!(
        nix::import("{ shared = 1; } // rec { shared = 2; }").unwrap(),
        expected
    );
    assert_eq!(
        parse_nxc("__nxc_update({ shared = 1; }, rec { shared = 2; })").unwrap(),
        expected
    );
}
