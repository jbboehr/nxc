// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use super::{SyntaxKind as K, lexer::Token};
use crate::Diagnostic;
use chumsky::{
    input::Stream,
    pratt::{infix, left, postfix, prefix},
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

        // Skip nested parentheses as a unit, stopping at the outer separator.
        let argument = expr.clone().recover_with(via_parser(
            nested_delimiters(K::LParen, K::RParen, [], |_| ())
                .or(none_of([K::LParen, K::Comma, K::RParen]).ignored())
                .repeated()
                .at_least(1)
                .ignored()
                .map_with(|_, e| Node::new(K::ErrorExpr, e.span(), vec![])),
        ));
        let arguments = argument
            .separated_by(just(K::Comma))
            .at_least(1)
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(K::LParen), just(K::RParen));

        let arithmetic = choice((integer, variable, paren)).pratt((
            postfix(4, arguments, |function, arguments: Vec<Node>, e| {
                let mut children = vec![function];
                children.extend(arguments);
                Node::new(K::CallExpr, e.span(), children)
            }),
            prefix(3, just(K::Minus), |_, operand, e| {
                Node::new(K::NegateExpr, e.span(), vec![operand])
            }),
            infix(left(2), one_of([K::Star, K::Slash]), |lhs, _, rhs, e| {
                Node::new(K::BinaryExpr, e.span(), vec![lhs, rhs])
            }),
            infix(left(1), one_of([K::Plus, K::Minus]), |lhs, _, rhs, e| {
                Node::new(K::BinaryExpr, e.span(), vec![lhs, rhs])
            }),
        ));
        // A lambda extends to the right over the entire body. It is only an
        // arithmetic operand or callee when explicitly parenthesized.
        lambda.or(arithmetic)
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
