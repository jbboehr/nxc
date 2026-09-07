// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use super::{NxcLanguage, SyntaxKind as K, SyntaxNode};
use crate::{
    Diagnostic,
    ir::{self, BinaryOp, Binding, Expr, Formal, Pattern, StringPart},
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
        // Generated output parenthesizes semantic operations. These wrappers
        // must not add a full recursive lowering frame at each nesting level.
        let mut expr = self.clone();
        while expr.0.kind() == K::ParenExpr {
            expr = expr.0.children().find_map(Self::cast).ok_or_else(|| {
                let span = expr.0.text_range();
                Diagnostic::new(
                    usize::from(span.start())..usize::from(span.end()),
                    "missing parenthesized expression",
                )
            })?;
        }
        expr.lower_unparenthesized()
    }

    fn lower_unparenthesized(&self) -> Result<Expr, Diagnostic> {
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
            K::VariableExpr => {
                let name = self.0.text().to_string();
                ir::validate_name(&name).map_err(error)?;
                Ok(Expr::Variable(name))
            }
            K::ListExpr => Ok(Expr::List(
                children
                    .map(|item| item.lower())
                    .collect::<Result<_, _>>()?,
            )),
            K::StringExpr => lower_string(&self.0),
            K::AttrSetExpr => Ok(Expr::AttrSet {
                recursive: self
                    .0
                    .children_with_tokens()
                    .filter_map(|it| it.into_token())
                    .any(|token| token.kind() == K::Rec),
                bindings: lower_bindings(&self.0)?,
            }),
            K::LetExpr => Ok(Expr::Let {
                bindings: lower_bindings(&self.0)?,
                body: Box::new(child()?),
            }),
            K::WithExpr => Ok(Expr::With {
                scope: Box::new(child()?),
                body: Box::new(child()?),
            }),
            K::IfExpr => Ok(Expr::If {
                condition: Box::new(child()?),
                then_branch: Box::new(child()?),
                else_branch: Box::new(child()?),
            }),
            K::SelectExpr => Ok(Expr::Select {
                value: Box::new(child()?),
                path: lower_path(&self.0)?,
                default: children
                    .next()
                    .map(|expr| expr.lower().map(Box::new))
                    .transpose()?,
            }),
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

fn lower_bindings(node: &SyntaxNode) -> Result<Vec<Binding>, Diagnostic> {
    let range = node.text_range();
    let error = |message| {
        Diagnostic::new(
            usize::from(range.start())..usize::from(range.end()),
            message,
        )
    };
    node.children()
        .filter(|node| !node.kind().is_expr())
        .map(|binding| match binding.kind() {
            K::AssignBinding => Ok(Binding::Assign {
                path: lower_path(&binding)?,
                value: binding
                    .children()
                    .find_map(Expression::cast)
                    .ok_or_else(|| error("missing binding value"))?
                    .lower()?,
            }),
            K::InheritBinding => Ok(Binding::Inherit {
                source: binding
                    .children()
                    .find(|n| n.kind() == K::InheritSource)
                    .map(|source| {
                        source
                            .children()
                            .find_map(Expression::cast)
                            .ok_or_else(|| error("missing inheritance source"))?
                            .lower()
                    })
                    .transpose()?,
                names: binding
                    .children()
                    .filter(|n| n.kind() == K::AttrName)
                    .map(|name| name.text().to_string())
                    .collect(),
            }),
            _ => Err(error("cannot lower an erroneous binding")),
        })
        .collect()
}

fn lower_string(node: &SyntaxNode) -> Result<Expr, Diagnostic> {
    let range = node.text_range();
    let error = |message| {
        Diagnostic::new(
            usize::from(range.start())..usize::from(range.end()),
            message,
        )
    };
    let indented = node.first_token().is_some_and(|token| token.text() == "''");
    let mut parts = Vec::new();
    for part in node.children() {
        parts.push(match part.kind() {
            K::StringText => StringPart::Literal(part.text().to_string()),
            K::StringInterpolation => StringPart::Interpolation(
                part.children()
                    .find_map(Expression::cast)
                    .ok_or_else(|| error("missing interpolation expression"))?
                    .lower()?,
            ),
            _ => return Err(error("cannot lower an erroneous string part")),
        });
    }
    crate::string::lower(parts, indented).map_err(error)
}

fn lower_path(node: &SyntaxNode) -> Result<Vec<String>, Diagnostic> {
    let path = node
        .children()
        .find(|node| node.kind() == K::AttrPath)
        .ok_or_else(|| {
            let range = node.text_range();
            Diagnostic::new(
                usize::from(range.start())..usize::from(range.end()),
                "missing attribute path",
            )
        })?;
    Ok(path
        .children()
        .filter(|node| node.kind() == K::AttrName)
        .map(|node| node.text().to_string())
        .collect())
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
