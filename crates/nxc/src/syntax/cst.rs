// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use super::{SyntaxKind as K, lexer::Token, parser::Node};
use rowan::{GreenNode, GreenNodeBuilder};

pub(super) fn build(source: &str, tokens: &[Token], expression: Option<&Node>) -> GreenNode {
    let mut builder = Builder {
        source,
        tokens,
        cursor: 0,
        green: GreenNodeBuilder::new(),
    };
    builder.green.start_node(rowan::SyntaxKind(K::Root as u16));
    if let Some(expression) = expression {
        builder.tokens_before(expression.span.start);
        builder.node(expression);
    } else {
        builder
            .green
            .start_node(rowan::SyntaxKind(K::ErrorExpr as u16));
        builder.tokens_before(source.len());
        builder.green.finish_node();
    }
    builder.tokens_before(source.len());
    builder.green.finish_node();
    builder.green.finish()
}

struct Builder<'src> {
    source: &'src str,
    tokens: &'src [Token],
    cursor: usize,
    green: GreenNodeBuilder<'static>,
}

impl Builder<'_> {
    fn tokens_before(&mut self, end: usize) {
        while let Some(token) = self.tokens.get(self.cursor).filter(|t| t.span.start < end) {
            self.green.token(
                rowan::SyntaxKind(token.kind as u16),
                &self.source[token.span.clone()],
            );
            self.cursor += 1;
        }
    }

    fn node(&mut self, node: &Node) {
        self.green.start_node(rowan::SyntaxKind(node.kind as u16));
        for child in &node.children {
            self.tokens_before(child.span.start);
            self.node(child);
        }
        self.tokens_before(node.span.end);
        self.green.finish_node();
    }
}
