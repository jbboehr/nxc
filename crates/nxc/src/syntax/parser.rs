// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use super::{SyntaxKind as K, lexer::Token};
use crate::Diagnostic;
use chumsky::{
    input::Stream,
    pratt::{infix, left, none, postfix, prefix, right},
    prelude::*,
    recovery::{nested_delimiters, via_parser},
};
use std::ops::Range;

/// A temporary grammar tree. The CST builder fills the gaps with original tokens.
pub(super) struct Node {
    pub kind: K,
    pub span: Range<usize>,
    pub children: Vec<Node>,
}

impl Node {
    fn new(kind: K, span: SimpleSpan, children: Vec<Node>) -> Self {
        Self {
            kind,
            span: span.into_range(),
            children,
        }
    }

    pub fn depth(&self) -> usize {
        let mut max = 0;
        let mut pending = vec![(self, 1)];
        while let Some((node, depth)) = pending.pop() {
            max = max.max(depth);
            pending.extend(node.children.iter().map(|child| (child, depth + 1)));
        }
        max
    }
}

pub(super) fn parse(tokens: &[Token], source_len: usize) -> (Option<Node>, Vec<Diagnostic>) {
    let input = Stream::from_iter(
        tokens
            .iter()
            .filter(|t| !t.kind.is_trivia())
            .map(|t| (t.kind, SimpleSpan::from(t.span.clone()))),
    )
    .map((source_len..source_len).into(), |(kind, span)| (kind, span));

    let expr = recursive(|expr| {
        let integer = just::<_, _, extra::Err<Rich<'_, K>>>(K::Integer)
            .map_with(|_, e| Node::new(K::IntegerExpr, e.span(), vec![]));
        let variable = just(K::Ident).map_with(|_, e| Node::new(K::VariableExpr, e.span(), vec![]));
        let paren = expr
            .clone()
            .delimited_by(just(K::LParen), just(K::RParen))
            .map_with(|inner, e| Node::new(K::ParenExpr, e.span(), vec![inner]));
        let string_text =
            just(K::StringContent).map_with(|_, e| Node::new(K::StringText, e.span(), vec![]));
        let interpolation = expr
            .clone()
            .delimited_by(just(K::InterpolationStart), just(K::InterpolationEnd))
            .map_with(|value, e| Node::new(K::StringInterpolation, e.span(), vec![value]));
        let string = choice((string_text, interpolation))
            .repeated()
            .collect::<Vec<_>>()
            .delimited_by(just(K::StringStart), just(K::StringEnd))
            .map_with(|parts, e| Node::new(K::StringExpr, e.span(), parts));

        let name = just(K::Ident).map_with(|_, e| Node::new(K::IdentPattern, e.span(), vec![]));
        let formal = just(K::Ident)
            .then(just(K::Question).ignore_then(expr.clone()).or_not())
            .map_with(|(_, default), e| {
                Node::new(K::Formal, e.span(), default.into_iter().collect())
            });
        let ellipsis =
            just(K::Ellipsis).map_with(|_, e| Node::new(K::PatternEllipsis, e.span(), vec![]));
        // Consume each default once, including the last field. Retrying a
        // complete formal when its trailing comma is absent would reparse
        // nested defaults exponentially.
        let attrs = formal
            .then(just(K::Comma).or_not())
            .repeated()
            .collect::<Vec<_>>()
            .then(ellipsis.or_not())
            .delimited_by(just(K::LBrace), just(K::RBrace))
            .try_map(|(fields, ellipsis), span| {
                for (index, (_, comma)) in fields.iter().enumerate() {
                    if comma.is_none() && (index + 1 < fields.len() || ellipsis.is_some()) {
                        return Err(Rich::custom(span, "expected ',' after pattern field"));
                    }
                }
                let mut children: Vec<_> = fields.into_iter().map(|(field, _)| field).collect();
                children.extend(ellipsis);
                Ok(Node::new(K::AttrPattern, span, children))
            });
        let prefix_bind = just(K::Ident)
            .then_ignore(just(K::At))
            .map_with(|_, e| Node::new(K::PatternBind, e.span(), vec![]));
        let suffix_bind = just(K::At)
            .ignore_then(just(K::Ident))
            .map_with(|_, e| Node::new(K::PatternBind, e.span(), vec![]));
        let pattern = choice((
            prefix_bind
                .then(attrs.clone())
                .map_with(|(bind, attrs), e| {
                    let mut children = vec![bind];
                    children.extend(attrs.children);
                    Node::new(K::AttrPattern, e.span(), children)
                }),
            attrs
                .then(suffix_bind.or_not())
                .map_with(|(attrs, bind), e| {
                    let mut children = attrs.children;
                    children.extend(bind);
                    Node::new(K::AttrPattern, e.span(), children)
                }),
            name,
        ));
        let parameter = just(K::Fn)
            .or_not()
            .ignore_then(pattern.delimited_by(just(K::LParen), just(K::RParen)))
            .or(name);
        let lambda = parameter
            .then_ignore(just(K::Arrow))
            .then(expr.clone())
            .map_with(|(parameter, body), e| {
                Node::new(K::LambdaExpr, e.span(), vec![parameter, body])
            });

        let attr_name = one_of([K::Ident, K::Or, K::Fn, K::Yield, K::UpdateIntrinsic])
            .map_with(|_, e| Node::new(K::AttrName, e.span(), vec![]));
        let path = attr_name
            .separated_by(just(K::Dot))
            .at_least(1)
            .collect::<Vec<_>>()
            .map_with(|names, e| Node::new(K::AttrPath, e.span(), names));
        let assignment = path
            .then_ignore(just(K::Assign))
            .then(expr.clone())
            .map_with(|(path, value), e| Node::new(K::AssignBinding, e.span(), vec![path, value]));
        let inherit_source = expr
            .clone()
            .delimited_by(just(K::LParen), just(K::RParen))
            .map_with(|source, e| Node::new(K::InheritSource, e.span(), vec![source]));
        let inherit = just(K::Inherit)
            .ignore_then(inherit_source.or_not())
            .then(attr_name.repeated().collect::<Vec<_>>())
            .map_with(|(source, names), e| {
                let mut children: Vec<_> = source.into_iter().collect();
                children.extend(names);
                Node::new(K::InheritBinding, e.span(), children)
            });
        let nested = choice((
            nested_delimiters(
                K::LBracket,
                K::RBracket,
                [
                    (K::LParen, K::RParen),
                    (K::LBrace, K::RBrace),
                    (K::StringStart, K::StringEnd),
                    (K::InterpolationStart, K::InterpolationEnd),
                ],
                |_| (),
            ),
            nested_delimiters(
                K::LParen,
                K::RParen,
                [
                    (K::LBracket, K::RBracket),
                    (K::LBrace, K::RBrace),
                    (K::StringStart, K::StringEnd),
                    (K::InterpolationStart, K::InterpolationEnd),
                ],
                |_| (),
            ),
            nested_delimiters(
                K::LBrace,
                K::RBrace,
                [
                    (K::LBracket, K::RBracket),
                    (K::LParen, K::RParen),
                    (K::StringStart, K::StringEnd),
                    (K::InterpolationStart, K::InterpolationEnd),
                ],
                |_| (),
            ),
            nested_delimiters(
                K::StringStart,
                K::StringEnd,
                [
                    (K::LBracket, K::RBracket),
                    (K::LParen, K::RParen),
                    (K::LBrace, K::RBrace),
                    (K::InterpolationStart, K::InterpolationEnd),
                ],
                |_| (),
            ),
            nested_delimiters(
                K::InterpolationStart,
                K::InterpolationEnd,
                [
                    (K::LBracket, K::RBracket),
                    (K::LParen, K::RParen),
                    (K::LBrace, K::RBrace),
                    (K::StringStart, K::StringEnd),
                ],
                |_| (),
            ),
        ));
        let binding = |in_let: bool| {
            choice((assignment.clone(), inherit.clone()))
                .then_ignore(just(K::Semicolon))
                .recover_with(via_parser(
                    nested
                        .clone()
                        // A qualified attribute name is not the let result marker.
                        .or(just(K::Dot).then(just(K::Yield)).ignored())
                        .or(none_of([
                            K::LParen,
                            K::LBracket,
                            K::RBracket,
                            K::LBrace,
                            K::StringStart,
                            K::InterpolationStart,
                            K::Semicolon,
                            K::RBrace,
                            K::RParen,
                            K::StringEnd,
                            K::InterpolationEnd,
                        ])
                        .filter(move |kind| !in_let || *kind != K::Yield)
                        .ignored())
                        .repeated()
                        .at_least(1)
                        .ignored()
                        .then_ignore(just(K::Semicolon).or_not())
                        .map_with(|_, e| Node::new(K::ErrorBinding, e.span(), vec![])),
                ))
        };
        let attrset = just(K::Rec)
            .or_not()
            .ignore_then(
                binding(false)
                    .repeated()
                    .collect::<Vec<_>>()
                    .delimited_by(just(K::LBrace), just(K::RBrace)),
            )
            .map_with(|bindings, e| Node::new(K::AttrSetExpr, e.span(), bindings));

        let let_expr = just(K::Let)
            .ignore_then(
                // `yield` belongs to the result even when an earlier binding
                // is malformed or missing its semicolon.
                just(K::Yield)
                    .not()
                    .ignore_then(binding(true))
                    .repeated()
                    .collect::<Vec<_>>()
                    .then(
                        just(K::Yield)
                            .ignore_then(expr.clone())
                            .then_ignore(just(K::Semicolon)),
                    )
                    .delimited_by(just(K::LBrace), just(K::RBrace)),
            )
            .map_with(|(mut bindings, body), e| {
                bindings.push(body);
                Node::new(K::LetExpr, e.span(), bindings)
            });

        // Skip nested groups as a unit, stopping at the outer separator.
        // Pratt parsing can stop before an operator whose right operand fails.
        // Treat that partial result as an error here so recovery consumes the
        // malformed item and preserves expressions after its separator.
        let item = expr
            .clone()
            .then_ignore(
                one_of([
                    K::Plus,
                    K::PlusPlus,
                    K::Minus,
                    K::Star,
                    K::Slash,
                    K::EqualEqual,
                    K::NotEqual,
                    K::Less,
                    K::LessEqual,
                    K::Greater,
                    K::GreaterEqual,
                    K::AndAnd,
                    K::OrOr,
                ])
                .not(),
            )
            .recover_with(via_parser(
                nested
                    .or(none_of([
                        K::LParen,
                        K::LBracket,
                        K::RBracket,
                        K::LBrace,
                        K::StringStart,
                        K::InterpolationStart,
                        K::Comma,
                        K::RParen,
                        K::RBrace,
                        K::StringEnd,
                        K::InterpolationEnd,
                    ])
                    .ignored())
                    .repeated()
                    .at_least(1)
                    .ignored()
                    .map_with(|_, e| Node::new(K::ErrorExpr, e.span(), vec![])),
            ));
        // Parse each complete expression once. Missing commas do not cause
        // backtracking into it to invent a shorter element boundary.
        let list = item
            .clone()
            .then_ignore(just(K::Comma).or_not())
            .repeated()
            .collect::<Vec<_>>()
            .delimited_by(just(K::LBracket), just(K::RBracket))
            .map_with(|items, e| Node::new(K::ListExpr, e.span(), items));
        let arguments = item
            .separated_by(just(K::Comma))
            .at_least(1)
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(K::LParen), just(K::RParen));

        let with_expr = just(K::With)
            .ignore_then(arguments.clone())
            .try_map(|children, span| {
                if children.len() != 2 {
                    return Err(Rich::custom(span, "with requires a context and a body"));
                }
                Ok(Node::new(K::WithExpr, span, children))
            });
        let assert_expr =
            just(K::Assert)
                .ignore_then(arguments.clone())
                .try_map(|children, span| {
                    if children.len() != 2 {
                        return Err(Rich::custom(span, "assert requires a condition and a body"));
                    }
                    Ok(Node::new(K::AssertExpr, span, children))
                });

        let update = just(K::UpdateIntrinsic)
            .ignore_then(arguments.clone())
            .try_map(|children, span| {
                if children.len() != 2 {
                    return Err(Rich::custom(span, "__nxc_update requires two operands"));
                }
                Ok(Node::new(K::UpdateExpr, span, children))
            });

        let atom = choice((
            integer,
            variable,
            paren,
            attrset,
            let_expr,
            with_expr,
            assert_expr,
            update,
            string,
            list,
        ))
        .boxed();
        // Native `or` takes a simple expression: a call/arithmetic/lambda in
        // the fallback needs parentheses. Nested selections extend right.
        let simple = recursive(|simple| {
            atom.clone()
                .then(
                    just(K::Dot)
                        .ignore_then(path)
                        .then(just(K::Or).ignore_then(simple).or_not())
                        .or_not(),
                )
                .map_with(|(value, selection), e| {
                    if let Some((path, default)) = selection {
                        let mut children = vec![value, path];
                        children.extend(default);
                        Node::new(K::SelectExpr, e.span(), children)
                    } else {
                        value
                    }
                })
        });
        let selection = just(K::Dot)
            .ignore_then(path)
            .then(just(K::Or).ignore_then(simple).or_not());
        let operators = atom.pratt((
            postfix(10, arguments, |function, arguments: Vec<Node>, e| {
                let mut children = vec![function];
                children.extend(arguments);
                Node::new(K::CallExpr, e.span(), children)
            }),
            postfix(
                10,
                selection,
                |value, (path, default): (Node, Option<Node>), e| {
                    let mut children = vec![value, path];
                    children.extend(default);
                    Node::new(K::SelectExpr, e.span(), children)
                },
            ),
            prefix(9, just(K::Minus), |_, operand, e| {
                Node::new(K::NegateExpr, e.span(), vec![operand])
            }),
            infix(right(8), just(K::PlusPlus), |lhs, _, rhs, e| {
                Node::new(K::BinaryExpr, e.span(), vec![lhs, rhs])
            }),
            infix(left(7), one_of([K::Star, K::Slash]), |lhs, _, rhs, e| {
                Node::new(K::BinaryExpr, e.span(), vec![lhs, rhs])
            }),
            infix(left(6), one_of([K::Plus, K::Minus]), |lhs, _, rhs, e| {
                Node::new(K::BinaryExpr, e.span(), vec![lhs, rhs])
            }),
            // Nix's Boolean negation binds below arithmetic, above comparisons.
            prefix(5, just(K::Bang), |_, operand, e| {
                Node::new(K::NotExpr, e.span(), vec![operand])
            }),
            infix(
                none(4),
                one_of([K::Less, K::LessEqual, K::Greater, K::GreaterEqual]),
                |lhs, _, rhs, e| Node::new(K::BinaryExpr, e.span(), vec![lhs, rhs]),
            ),
            infix(
                none(3),
                one_of([K::EqualEqual, K::NotEqual]),
                |lhs, _, rhs, e| Node::new(K::BinaryExpr, e.span(), vec![lhs, rhs]),
            ),
            infix(left(2), just(K::AndAnd), |lhs, _, rhs, e| {
                Node::new(K::BinaryExpr, e.span(), vec![lhs, rhs])
            }),
            infix(left(1), just(K::OrOr), |lhs, _, rhs, e| {
                Node::new(K::BinaryExpr, e.span(), vec![lhs, rhs])
            }),
        ));
        let conditional = just(K::If)
            .ignore_then(expr.clone())
            .then_ignore(just(K::Then))
            .then(expr.clone())
            .then_ignore(just(K::Else))
            .then(expr)
            .map_with(|((condition, then_branch), else_branch), e| {
                Node::new(
                    K::IfExpr,
                    e.span(),
                    vec![condition, then_branch, else_branch],
                )
            });
        // Lambdas and conditionals extend to the right over the whole body or
        // final branch. As operator operands or callees they need parentheses.
        choice((conditional, lambda, operators))
    });

    let (node, errors) = expr.parse(input).into_output_errors();
    let diagnostics = errors
        .into_iter()
        .map(|error| {
            Diagnostic::new(
                error.span().into_range(),
                format!("expected an expression or delimiter: {error}"),
            )
        })
        .collect();
    (node, diagnostics)
}
