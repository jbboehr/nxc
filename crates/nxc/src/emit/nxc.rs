// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use crate::{
    Diagnostic, MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS,
    ir::{BinaryOp, Expr},
    syntax::{SyntaxKind as K, lexer},
};

/// Emit canonical nxc, flattening left-associated unary application chains.
pub fn emit(expr: &Expr) -> Result<String, Diagnostic> {
    expr.validate()?;
    let source = render(expr);
    let limit = || {
        Diagnostic::new(
            0..0,
            "generated nxc exceeds the source, token, or nesting limit",
        )
    };
    if source.len() > MAX_SOURCE_BYTES {
        return Err(limit());
    }
    let mut depth = 0usize;
    for (index, token) in lexer::lex(&source)
        .iter()
        .filter(|t| !t.kind.is_trivia())
        .enumerate()
    {
        match token.kind {
            K::LParen | K::LBrace | K::LBracket | K::StringStart | K::InterpolationStart => {
                depth += 1
            }
            K::RParen | K::RBrace | K::RBracket | K::StringEnd | K::InterpolationEnd => {
                depth = depth.saturating_sub(1)
            }
            _ => {}
        }
        if index >= MAX_TOKENS || depth > MAX_DEPTH {
            return Err(limit());
        }
    }
    Ok(source)
}

fn render(expr: &Expr) -> String {
    match expr {
        Expr::Integer(value) => value.to_string(),
        Expr::Float(value) => value.to_string(),
        Expr::Variable(name) => name.clone(),
        // Protect path characters from surrounding unary operators and selections.
        Expr::RelativePath(path)
        | Expr::AbsolutePath(path)
        | Expr::HomePath(path)
        | Expr::SearchPath(path) => format!("({path})"),
        Expr::String(parts) => super::string(parts, render),
        Expr::List(items) => format!(
            "[{}]",
            items.iter().map(render).collect::<Vec<_>>().join(", ")
        ),
        Expr::AttrSet {
            recursive,
            bindings,
        } => super::attrset(*recursive, bindings, render),
        Expr::Let { bindings, body } => format!(
            "let {{{} yield {}; }}",
            super::bindings(bindings, render),
            render(body)
        ),
        Expr::With { scope, body } => format!("with({}, {})", render(scope), render(body)),
        Expr::Assert { condition, body } => {
            format!("assert({}, {})", render(condition), render(body))
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
        } => super::selection(value, path, default.as_deref(), render),
        Expr::HasAttr { value, path } => super::has_attr(value, path, render),
        Expr::Lambda { parameter, body } => {
            let spelling = super::pattern(parameter, render);
            let parameter = match parameter {
                crate::ir::Pattern::Ident(_) => spelling,
                crate::ir::Pattern::AttrSet { .. } => format!("({spelling})"),
            };
            format!("({parameter} => {})", render(body))
        }
        Expr::Negate(value) => format!("(-{})", render(value)),
        Expr::Not(value) => format!("(!{})", render(value)),
        Expr::Binary {
            op: BinaryOp::Update,
            left,
            right,
        } => format!("__nxc_update({}, {})", render(left), render(right)),
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
