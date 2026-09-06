// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use super::{NxcLanguage, SyntaxKind as K, SyntaxNode};
use crate::{
    Diagnostic,
    ir::{BinaryOp, Expr, Formal, Pattern},
};
use rowan::ast::AstNode;

/// A typed expression view over the lossless nxc CST.
#[derive(Debug, Clone)]
pub struct Expression(SyntaxNode);

impl AstNode for Expression {
    type Language = NxcLanguage;
    fn can_cast(kind: K) -> bool {
        kind.is_expr()
    }
    fn cast(node: SyntaxNode) -> Option<Self> {
        Self::can_cast(node.kind()).then_some(Self(node))
    }
    fn syntax(&self) -> &SyntaxNode {
        &self.0
    }
}

impl Expression {
    pub(super) fn lower(&self) -> Result<Expr, Diagnostic> {
        let span = self.0.text_range();
        let error =
            |message| Diagnostic::new(usize::from(span.start())..usize::from(span.end()), message);
        let mut children = self.0.children().filter_map(Self::cast);
        let mut child = || {
            children
                .next()
                .ok_or_else(|| error("missing expression"))?
                .lower()
        };
        match self.0.kind() {
            K::IntegerExpr => {
                let value = self
                    .0
                    .text()
                    .to_string()
                    .parse::<u64>()
                    .map_err(|_| error("integer literal is out of range"))?;
                Ok(Expr::Integer(value))
            }
            K::VariableExpr => Ok(Expr::Variable(self.0.text().to_string())),
            K::ParenExpr => child(),
            K::NegateExpr => Ok(Expr::Negate(Box::new(child()?))),
            K::LambdaExpr => {
                let parameter = self
                    .0
                    .children()
                    .find(|n| matches!(n.kind(), K::IdentPattern | K::AttrPattern))
                    .ok_or_else(|| error("missing lambda parameter"))?;
                Ok(Expr::Lambda {
                    parameter: lower_pattern(&parameter)?,
                    body: Box::new(child()?),
                })
            }
            K::CallExpr => {
                let mut function = child()?;
                for argument in children {
                    function = Expr::Apply {
                        function: Box::new(function),
                        argument: Box::new(argument.lower()?),
                    };
                }
                Ok(function)
            }
            K::BinaryExpr => {
                let op = self
                    .0
                    .children_with_tokens()
                    .filter_map(|it| it.into_token())
                    .find_map(|token| match token.kind() {
                        K::Plus => Some(BinaryOp::Add),
                        K::Minus => Some(BinaryOp::Subtract),
                        K::Star => Some(BinaryOp::Multiply),
                        K::Slash => Some(BinaryOp::Divide),
                        _ => None,
                    })
                    .ok_or_else(|| error("missing arithmetic operator"))?;
                Ok(Expr::Binary {
                    op,
                    left: Box::new(child()?),
                    right: Box::new(child()?),
                })
            }
            _ => Err(error("cannot lower an erroneous expression")),
        }
    }
}

fn lower_pattern(node: &SyntaxNode) -> Result<Pattern, Diagnostic> {
    let name = |node: &SyntaxNode| {
        node.children_with_tokens()
            .filter_map(|it| it.into_token())
            .find(|token| token.kind() == K::Ident)
            .map(|token| token.text().to_owned())
            .ok_or_else(|| {
                let span = node.text_range();
                Diagnostic::new(
                    usize::from(span.start())..usize::from(span.end()),
                    "missing parameter name",
                )
            })
    };
    if node.kind() == K::IdentPattern {
        return Ok(Pattern::Ident(name(node)?));
    }
    let mut fields = Vec::new();
    let mut bind = None;
    let mut ellipsis = false;
    for child in node.children() {
        match child.kind() {
            K::Formal => fields.push(Formal {
                name: name(&child)?,
                default: child
                    .children()
                    .find_map(Expression::cast)
                    .map(|expr| expr.lower())
                    .transpose()?,
            }),
            K::PatternBind => bind = Some(name(&child)?),
            K::PatternEllipsis => ellipsis = true,
            _ => {}
        }
    }
    Ok(Pattern::AttrSet {
        fields,
        ellipsis,
        bind,
    })
}
