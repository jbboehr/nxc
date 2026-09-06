// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use crate::{Diagnostic, MAX_SOURCE_BYTES, MAX_TOKENS, ir::Expr, syntax::lexer};

/// Emit canonical nxc, flattening left-associated unary application chains.
pub fn emit(expr: &Expr) -> Result<String, Diagnostic> {
    expr.validate()?;
    let source = render(expr);
    if source.len() > MAX_SOURCE_BYTES
        || lexer::lex(&source)
            .iter()
            .filter(|t| !t.kind.is_trivia())
            .count()
            > MAX_TOKENS
    {
        return Err(Diagnostic::new(
            0..0,
            "generated nxc exceeds the source or token limit",
        ));
    }
    Ok(source)
}

fn render(expr: &Expr) -> String {
    match expr {
        Expr::Integer(value) => value.to_string(),
        Expr::Variable(name) => name.clone(),
        Expr::Negate(value) => format!("(-{})", render(value)),
        Expr::Binary { op, left, right } => {
            format!("({} {} {})", render(left), op.spelling(), render(right))
        }
        Expr::Apply { .. } => {
            let mut arguments = Vec::new();
            let mut function = expr;
            while let Expr::Apply {
                function: inner,
                argument,
            } = function
            {
                arguments.push(argument.as_ref());
                function = inner;
            }
            arguments.reverse();
            let arguments = arguments
                .into_iter()
                .map(render)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({arguments})", render(function))
        }
    }
}
