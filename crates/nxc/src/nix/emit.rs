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
        Expr::String(parts) => crate::emit::string(parts, render),
        // Every non-simple expression is already parenthesized by render.
        Expr::List(items) => format!(
            "[{}]",
            items.iter().map(render).collect::<Vec<_>>().join(" ")
        ),
        Expr::AttrSet {
            recursive,
            bindings,
        } => crate::emit::attrset(*recursive, bindings, render),
        Expr::Let { bindings, body } => format!(
            "(let{} in {})",
            crate::emit::bindings(bindings, render),
            render(body)
        ),
        Expr::With { scope, body } => format!("(with {}; {})", render(scope), render(body)),
        Expr::Assert { condition, body } => {
            format!("(assert {}; {})", render(condition), render(body))
        }
        Expr::If {
            condition,
            then_branch,
            else_branch,
        } => format!(
            "(if {} then {} else {})",
            render(condition),
            render(then_branch),
            render(else_branch)
        ),
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
