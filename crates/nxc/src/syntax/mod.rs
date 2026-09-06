// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

pub mod ast;
mod cst;
mod kind;
pub mod lexer;
mod parser;
pub use kind::{NxcLanguage, SyntaxKind};

use crate::{
    Diagnostic, MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS,
    ir::{self, Expr},
};
use rowan::ast::AstNode;
pub type SyntaxNode = rowan::SyntaxNode<NxcLanguage>;

#[derive(Debug, Clone)]
pub struct Parse {
    green: Option<rowan::GreenNode>,
    diagnostics: Vec<Diagnostic>,
}

impl Parse {
    /// The recovered lossless CST. Absent only when the source exceeds the size limit.
    pub fn syntax(&self) -> Option<SyntaxNode> {
        self.green.clone().map(SyntaxNode::new_root)
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn lower(&self) -> Result<Expr, Vec<Diagnostic>> {
        if !self.diagnostics.is_empty() {
            return Err(self.diagnostics.clone());
        }
        let expr = self
            .syntax()
            .and_then(|root| root.children().find_map(ast::Expression::cast))
            .ok_or_else(|| vec![Diagnostic::new(0..0, "missing expression")])?;
        let lowered = expr.lower().map_err(|e| vec![e])?;
        lowered.validate().map_err(|mut e| {
            let range = expr.syntax().text_range();
            e.span = usize::from(range.start())..usize::from(range.end());
            vec![e]
        })?;
        Ok(lowered)
    }
}

pub fn parse(source: &str) -> Parse {
    if source.len() > MAX_SOURCE_BYTES {
        return Parse {
            green: None,
            diagnostics: vec![Diagnostic::new(
                0..source.len(),
                "source exceeds the 1 MiB limit",
            )],
        };
    }
    let tokens = lexer::lex(source);
    let mut depth = 0usize;
    let mut peak_depth = 0;
    let mut count = 0;
    for token in tokens.iter().filter(|t| !t.kind.is_trivia()) {
        count += 1;
        match token.kind {
            SyntaxKind::LParen => {
                depth += 1;
                peak_depth = peak_depth.max(depth);
            }
            SyntaxKind::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let mut diagnostics = Vec::new();
    let node = if count > MAX_TOKENS || peak_depth > MAX_DEPTH {
        diagnostics.push(Diagnostic::new(
            0..source.len(),
            "expression exceeds the token or nesting limit",
        ));
        None
    } else {
        // Enforce resource bounds before allocating per-token diagnostics.
        for token in &tokens {
            let text = &source[token.span.clone()];
            let message = match token.kind {
                SyntaxKind::ErrorToken => Some("invalid or unsupported token"),
                SyntaxKind::UnsupportedPath => Some("path expressions are not supported yet"),
                SyntaxKind::Ident => ir::validate_name(text).err(),
                SyntaxKind::Integer if text.parse::<i64>().is_err() => {
                    Some("integer literal exceeds the Nix signed 64-bit range")
                }
                _ => None,
            };
            if let Some(message) = message {
                diagnostics.push(Diagnostic::new(token.span.clone(), message));
            }
        }
        let (node, errors) = parser::parse(&tokens, source.len());
        diagnostics.extend(errors);
        // Canonical output may wrap each semantic operation in parentheses.
        if node
            .as_ref()
            .is_some_and(|node| node.depth() > 2 * MAX_DEPTH)
        {
            diagnostics.push(Diagnostic::new(
                0..source.len(),
                "expression exceeds the nesting limit",
            ));
            None
        } else {
            node
        }
    };
    let green = Some(cst::build(source, &tokens, node.as_ref()));
    Parse { green, diagnostics }
}
