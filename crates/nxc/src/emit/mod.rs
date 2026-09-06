// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

mod nxc;
pub use nxc::emit as nxc;

pub(crate) fn attrset(
    recursive: bool,
    bindings: &[crate::ir::Binding],
    render: fn(&crate::ir::Expr) -> String,
) -> String {
    use crate::ir::Binding;
    let mut source = if recursive { "rec {" } else { "{" }.to_owned();
    for binding in bindings {
        source.push(' ');
        match binding {
            Binding::Assign { path, value } => {
                source.push_str(&format!("{} = {};", path.join("."), render(value)));
            }
            Binding::Inherit {
                source: from,
                names,
            } => {
                source.push_str("inherit");
                if let Some(from) = from {
                    source.push_str(&format!(" ({})", render(from)));
                }
                for name in names {
                    source.push(' ');
                    source.push_str(name);
                }
                source.push(';');
            }
        }
    }
    source.push_str(" }");
    source
}

pub(crate) fn selection(
    value: &crate::ir::Expr,
    path: &[String],
    default: Option<&crate::ir::Expr>,
    render: fn(&crate::ir::Expr) -> String,
) -> String {
    let mut source = format!("(({}).{}", render(value), path.join("."));
    if let Some(default) = default {
        source.push_str(&format!(" or ({})", render(default)));
    }
    source.push(')');
    source
}

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
