// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

mod nxc;
pub use nxc::emit as nxc;

// Pattern punctuation is shared; default expressions use the target dialect.
pub(crate) fn pattern(
    parameter: &crate::ir::Pattern,
    render: fn(&crate::ir::Expr) -> String,
) -> String {
    use crate::ir::Pattern;
    match parameter {
        Pattern::Ident(name) => name.clone(),
        Pattern::AttrSet {
            fields,
            ellipsis,
            bind,
        } => {
            let mut entries: Vec<_> = fields
                .iter()
                .map(|field| match &field.default {
                    Some(default) => format!("{} ? {}", field.name, render(default)),
                    None => field.name.clone(),
                })
                .collect();
            if *ellipsis {
                entries.push("...".into());
            }
            let mut result = format!("{{ {} }}", entries.join(", "));
            if let Some(bind) = bind {
                result.push('@');
                result.push_str(bind);
            }
            result
        }
    }
}
