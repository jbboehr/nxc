// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use crate::{Diagnostic, ir::Expr};

/// Emit conservative, parenthesized native Nix without evaluating expressions.
pub fn emit(expr: &Expr) -> Result<String, Diagnostic> {
    expr.validate()?;
    let source = render(expr);
    super::import::check_source(&source)?;
    Ok(source)
}

fn render(expr: &Expr) -> String {
    match expr {
        Expr::Integer(value) => value.to_string(),
        Expr::Variable(name) => name.clone(),
        Expr::AttrSet {
            recursive,
            bindings,
        } => crate::emit::attrset(*recursive, bindings, render),
        Expr::Select {
            value,
            path,
            default,
        } => crate::emit::selection(value, path, default.as_deref(), render),
        Expr::Lambda { parameter, body } => {
            format!(
                "({}: {})",
                crate::emit::pattern(parameter, render),
                render(body)
            )
        }
        Expr::Negate(value) => format!("(-{})", render(value)),
        Expr::Binary { op, left, right } => {
            format!("({} {} {})", render(left), op.spelling(), render(right))
        }
        Expr::Apply { function, argument } => {
            format!("({} {})", render(function), render(argument))
        }
    }
}
