// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use super::{NxcLanguage, SyntaxKind as K, SyntaxNode};
use crate::{
    Diagnostic, MAX_DEPTH,
    ir::{self, AttrName, BinaryOp, Binding, Expr, Formal, Pattern, StringPart},
    string::StringContext,
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
    pub(super) fn lower(&self, depth: usize) -> Result<Expr, Diagnostic> {
        self.lower_with_string_context(depth, StringContext::Value)
    }

    fn lower_with_string_context(
        &self,
        depth: usize,
        context: StringContext,
    ) -> Result<Expr, Diagnostic> {
        // Reject excessive recursion before building the IR. Delimiter-free
        // chains can pass parser preflight but exceed the semantic depth limit.
        if depth > MAX_DEPTH {
            let span = self.0.text_range();
            let mut error = crate::limits::exceeded("semantic depth", depth, MAX_DEPTH);
            error.span = usize::from(span.start())..usize::from(span.end());
            return Err(error);
        }
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
        expr.lower_unparenthesized(depth, context)
    }

    fn lower_unparenthesized(
        &self,
        depth: usize,
        context: StringContext,
    ) -> Result<Expr, Diagnostic> {
        let span = self.0.text_range();
        let error =
            |message| Diagnostic::new(usize::from(span.start())..usize::from(span.end()), message);
        let mut children = self.0.children().filter_map(Self::cast);
        let mut child = || {
            children
                .next()
                .ok_or_else(|| error("missing expression"))?
                .lower(depth + 1)
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
            K::FloatExpr => lower_float(&self.0),
            K::VariableExpr => {
                let spelling = self.0.text().to_string();
                let name = super::ident::decode(&spelling).to_owned();
                ir::validate_name(&name).map_err(error)?;
                Ok(Expr::Variable(name))
            }
            K::RelativePathExpr => {
                let path = self.0.text().to_string();
                ir::validate_relative_path(&path).map_err(error)?;
                Ok(Expr::RelativePath(path))
            }
            K::SearchPathExpr => lower_search_path(&self.0),
            K::AbsolutePathExpr => lower_absolute_path(&self.0),
            K::HomePathExpr => lower_home_path(&self.0),
            K::InterpolatedPathExpr => lower_interpolated_path(&self.0, depth),
            K::ListExpr => Ok(Expr::List(
                children
                    .map(|item| item.lower(depth + 1))
                    .collect::<Result<_, _>>()?,
            )),
            K::StringExpr => lower_string(&self.0, depth, context),
            K::AttrSetExpr => Ok(Expr::AttrSet {
                recursive: self
                    .0
                    .children_with_tokens()
                    .filter_map(|it| it.into_token())
                    .any(|token| token.kind() == K::Rec),
                bindings: lower_bindings(&self.0, depth)?,
            }),
            K::LetExpr => Ok(Expr::Let {
                bindings: lower_bindings(&self.0, depth)?,
                body: Box::new(child()?),
            }),
            K::WithExpr => Ok(Expr::With {
                scope: Box::new(child()?),
                body: Box::new(child()?),
            }),
            K::AssertExpr => Ok(Expr::Assert {
                condition: Box::new(child()?),
                body: Box::new(child()?),
            }),
            K::UpdateExpr => Ok(Expr::Binary {
                op: BinaryOp::Update,
                left: Box::new(child()?),
                right: Box::new(child()?),
            }),
            K::IfExpr => Ok(Expr::If {
                condition: Box::new(child()?),
                then_branch: Box::new(child()?),
                else_branch: Box::new(child()?),
            }),
            K::SelectExpr => Ok(Expr::Select {
                value: Box::new(child()?),
                path: lower_lookup_path(&self.0, depth)?,
                default: children
                    .next()
                    .map(|expr| expr.lower(depth + 1).map(Box::new))
                    .transpose()?,
            }),
            K::HasAttrExpr => lower_has_attr(&self.0, depth),
            K::NegateExpr => Ok(Expr::Negate(Box::new(child()?))),
            K::NotExpr => Ok(Expr::Not(Box::new(child()?))),
            K::LambdaExpr => {
                let parameter = self
                    .0
                    .children()
                    .find(|n| matches!(n.kind(), K::IdentPattern | K::AttrPattern))
                    .ok_or_else(|| error("missing lambda parameter"))?;
                Ok(Expr::Lambda {
                    parameter: lower_pattern(&parameter, depth)?,
                    body: Box::new(child()?),
                })
            }
            K::CallExpr => {
                let mut function = child()?;
                for argument in children {
                    function = Expr::Apply {
                        function: Box::new(function),
                        argument: Box::new(argument.lower(depth + 1)?),
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
                        K::PlusPlus => Some(BinaryOp::Concat),
                        K::Minus => Some(BinaryOp::Subtract),
                        K::Star => Some(BinaryOp::Multiply),
                        K::Slash => Some(BinaryOp::Divide),
                        K::EqualEqual => Some(BinaryOp::Equal),
                        K::NotEqual => Some(BinaryOp::NotEqual),
                        K::Less => Some(BinaryOp::Less),
                        K::LessEqual => Some(BinaryOp::LessOrEqual),
                        K::Greater => Some(BinaryOp::Greater),
                        K::GreaterEqual => Some(BinaryOp::GreaterOrEqual),
                        K::AndAnd => Some(BinaryOp::And),
                        K::OrOr => Some(BinaryOp::Or),
                        _ => None,
                    })
                    .ok_or_else(|| error("missing binary operator"))?;
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

// Keep path text and validation out of every recursive expression's frame.
fn lower_interpolated_path(node: &SyntaxNode, depth: usize) -> Result<Expr, Diagnostic> {
    let range = node.text_range();
    let error = |message| {
        Diagnostic::new(
            usize::from(range.start())..usize::from(range.end()),
            message,
        )
    };
    let mut parts = Vec::new();
    let mut end = range.start();
    for part in node.children() {
        // Trivia ends a path. The expression grammar otherwise ignores it,
        // so do not combine a later standalone interpolation with this path.
        if part.text_range().start() != end {
            return Err(error("path fragments must be adjacent"));
        }
        end = part.text_range().end();
        parts.push(match part.kind() {
            K::PathText => StringPart::Literal(part.text().to_string()),
            K::StringInterpolation => StringPart::Interpolation(
                part.children()
                    .find_map(Expression::cast)
                    .ok_or_else(|| error("missing path interpolation expression"))?
                    .lower(depth + 1)?,
            ),
            _ => return Err(error("cannot lower an erroneous path part")),
        });
    }
    ir::lower_path(parts).map_err(error)
}

fn lower_home_path(node: &SyntaxNode) -> Result<Expr, Diagnostic> {
    let path = node.text().to_string();
    ir::validate_home_path(&path).map_err(|message| {
        let range = node.text_range();
        Diagnostic::new(
            usize::from(range.start())..usize::from(range.end()),
            message,
        )
    })?;
    Ok(Expr::HomePath(path))
}

fn lower_absolute_path(node: &SyntaxNode) -> Result<Expr, Diagnostic> {
    let path = node.text().to_string();
    ir::validate_absolute_path(&path).map_err(|message| {
        let range = node.text_range();
        Diagnostic::new(
            usize::from(range.start())..usize::from(range.end()),
            message,
        )
    })?;
    Ok(Expr::AbsolutePath(path))
}

fn lower_search_path(node: &SyntaxNode) -> Result<Expr, Diagnostic> {
    let path = node.text().to_string();
    ir::validate_search_path(&path).map_err(|message| {
        let range = node.text_range();
        Diagnostic::new(
            usize::from(range.start())..usize::from(range.end()),
            message,
        )
    })?;
    Ok(Expr::SearchPath(path))
}

// Keep literal parsing out of the frame used by every recursive expression.
fn lower_float(node: &SyntaxNode) -> Result<Expr, Diagnostic> {
    let range = node.text_range();
    node.text()
        .to_string()
        .parse()
        .map(Expr::Float)
        .map_err(|message| {
            Diagnostic::new(
                usize::from(range.start())..usize::from(range.end()),
                message,
            )
        })
}

// Keep path collection out of the frame used by every recursive expression.
fn lower_has_attr(node: &SyntaxNode, depth: usize) -> Result<Expr, Diagnostic> {
    let value = node.children().find_map(Expression::cast).ok_or_else(|| {
        let span = node.text_range();
        Diagnostic::new(
            usize::from(span.start())..usize::from(span.end()),
            "missing existence operand",
        )
    })?;
    Ok(Expr::HasAttr {
        value: Box::new(value.lower(depth + 1)?),
        path: lower_lookup_path(node, depth)?,
    })
}

fn lower_bindings(node: &SyntaxNode, depth: usize) -> Result<Vec<Binding>, Diagnostic> {
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
                path: lower_binding_path(&binding, depth)?,
                value: binding
                    .children()
                    .find_map(Expression::cast)
                    .ok_or_else(|| error("missing binding value"))?
                    .lower(depth + 1)?,
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
                            .lower(depth + 1)
                    })
                    .transpose()?,
                names: binding
                    .children()
                    .filter(|n| n.kind() == K::AttrName)
                    .map(|name| {
                        let key = lower_attr(&name, depth)?;
                        key.literal_name()
                            .map(ToOwned::to_owned)
                            .ok_or_else(|| error("dynamic attributes are not allowed in inherit"))
                    })
                    .collect::<Result<_, _>>()?,
            }),
            _ => Err(error("cannot lower an erroneous binding")),
        })
        .collect()
}

fn lower_string(
    node: &SyntaxNode,
    depth: usize,
    context: StringContext,
) -> Result<Expr, Diagnostic> {
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
                    .lower_with_string_context(
                        depth + 1,
                        if indented {
                            context
                        } else {
                            StringContext::Value
                        },
                    )?,
            ),
            _ => return Err(error("cannot lower an erroneous string part")),
        });
    }
    crate::string::lower(parts, indented, context).map_err(error)
}

fn lower_lookup_path(node: &SyntaxNode, depth: usize) -> Result<Vec<AttrName>, Diagnostic> {
    node.children()
        .filter(|node| node.kind() == K::AttrName)
        .map(|name| lower_attr(&name, depth))
        .collect()
}

fn lower_binding_path(node: &SyntaxNode, depth: usize) -> Result<Vec<AttrName>, Diagnostic> {
    node.children()
        .filter(|node| node.kind() == K::AttrName)
        .enumerate()
        .map(|(index, name)| lower_attr(&name, depth + index))
        .collect()
}

fn lower_attr(node: &SyntaxNode, depth: usize) -> Result<AttrName, Diagnostic> {
    let range = node.text_range();
    let error = |message| {
        Diagnostic::new(
            usize::from(range.start())..usize::from(range.end()),
            message,
        )
    };
    if node.first_token().is_some_and(|token| token.text() == "''") {
        return Err(error("attribute names require double quotes"));
    }
    if let Some(key) = node.children().find_map(Expression::cast) {
        return Ok(AttrName::Dynamic(Box::new(key.lower_with_string_context(
            depth + 1,
            StringContext::AttributeKey,
        )?)));
    }
    let name = node.text().to_string();
    if node
        .first_token()
        .is_some_and(|token| token.kind() == K::StringStart)
    {
        crate::string::decode(&name[1..name.len() - 1])
            .map(AttrName::Static)
            .map_err(error)
    } else {
        ir::validate_bare_attr_name(&name).map_err(error)?;
        Ok(AttrName::Static(name))
    }
}

fn lower_pattern(node: &SyntaxNode, depth: usize) -> Result<Pattern, Diagnostic> {
    let name = |node: &SyntaxNode| {
        node.children_with_tokens()
            .filter_map(|it| it.into_token())
            .find(|token| token.kind() == K::Ident)
            .map(|token| super::ident::decode(token.text()).to_owned())
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
                    .map(|expr| expr.lower(depth + 1))
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
