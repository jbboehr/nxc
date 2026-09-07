// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

mod nxc;
pub use nxc::emit as nxc;

pub(crate) fn string(
    parts: &[crate::ir::StringPart],
    render: fn(&crate::ir::Expr) -> String,
) -> String {
    use crate::ir::StringPart;
    let mut source = String::from("\"");
    for part in parts {
        match part {
            StringPart::Literal(text) => escape_literal(text, &mut source),
            StringPart::Interpolation(value) => {
                source.push_str("${");
                source.push_str(&render(value));
                source.push('}');
            }
        }
    }
    source.push('"');
    source
}

fn escape_literal(text: &str, source: &mut String) {
    for character in text.chars() {
        match character {
            '"' => source.push_str("\\\""),
            '\\' => source.push_str("\\\\"),
            // Escape every dollar, including one next to an interpolation.
            '$' => source.push_str("\\$"),
            '\n' => source.push_str("\\n"),
            '\r' => source.push_str("\\r"),
            '\t' => source.push_str("\\t"),
            other => source.push(other),
        }
    }
}

fn attribute(name: &str) -> String {
    if crate::ir::validate_bare_attr_name(name).is_ok() {
        return name.to_owned();
    }
    let mut source = String::from("\"");
    escape_literal(name, &mut source);
    source.push('"');
    source
}

fn attribute_path(path: &[String]) -> String {
    path.iter()
        .map(|name| attribute(name))
        .collect::<Vec<_>>()
        .join(".")
}

pub(crate) fn attrset(
    recursive: bool,
    bindings: &[crate::ir::Binding],
    render: fn(&crate::ir::Expr) -> String,
) -> String {
    format!(
        "{}{{{} }}",
        if recursive { "rec " } else { "" },
        self::bindings(bindings, render)
    )
}

pub(crate) fn bindings(
    bindings: &[crate::ir::Binding],
    render: fn(&crate::ir::Expr) -> String,
) -> String {
    use crate::ir::Binding;
    let mut source = String::new();
    for binding in bindings {
        source.push(' ');
        match binding {
            Binding::Assign { path, value } => {
                source.push_str(&format!("{} = {};", attribute_path(path), render(value)));
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
                    source.push_str(&attribute(name));
                }
                source.push(';');
            }
        }
    }
    source
}

pub(crate) fn selection(
    value: &crate::ir::Expr,
    path: &[crate::ir::AttrName],
    default: Option<&crate::ir::Expr>,
    render: fn(&crate::ir::Expr) -> String,
) -> String {
    use crate::ir::AttrName;
    let path = path
        .iter()
        .map(|name| match name {
            AttrName::Static(name) => attribute(name),
            AttrName::Dynamic(key) => format!("${{{}}}", render(key)),
        })
        .collect::<Vec<_>>()
        .join(".");
    let mut source = format!("(({}).{path}", render(value));
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
