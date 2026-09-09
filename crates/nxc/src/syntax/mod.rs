// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

pub mod ast;
mod cst;
pub(crate) mod ident;
mod kind;
pub mod lexer;
mod parser;
pub use kind::{NxcLanguage, SyntaxKind};

use crate::{Diagnostic, Limits, MAX_DEPTH, ir::Expr, limits};
use rowan::ast::AstNode;
pub type SyntaxNode = rowan::SyntaxNode<NxcLanguage>;

#[derive(Debug, Clone)]
pub struct Parse {
    green: Option<rowan::GreenNode>,
    diagnostics: Vec<Diagnostic>,
    limits: Limits,
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
        let lowered = expr.lower(1).map_err(|e| vec![e])?;
        lowered.validate(self.limits).map_err(|mut e| {
            let range = expr.syntax().text_range();
            e.span = usize::from(range.start())..usize::from(range.end());
            vec![e]
        })?;
        Ok(lowered)
    }
}

pub fn parse(source: &str) -> Parse {
    parse_with_limits(source, Limits::default())
}

/// Parse with an explicit byte and token/node budget, retaining it for lowering.
pub fn parse_with_limits(source: &str, limits: Limits) -> Parse {
    if let Err(error) = limits.check_source_bytes(source.len()) {
        return Parse {
            green: None,
            diagnostics: vec![error],
            limits,
        };
    }
    let tokens = lexer::lex(source);
    let mut depth = 0usize;
    let mut peak_depth = 0;
    let mut count = 0;
    for token in tokens.iter().filter(|t| !t.kind.is_trivia()) {
        count += 1;
        match token.kind {
            SyntaxKind::LParen
            | SyntaxKind::LBrace
            | SyntaxKind::LBracket
            | SyntaxKind::StringStart
            | SyntaxKind::InterpolationStart => {
                depth += 1;
                peak_depth = peak_depth.max(depth);
            }
            SyntaxKind::RParen
            | SyntaxKind::RBrace
            | SyntaxKind::RBracket
            | SyntaxKind::StringEnd
            | SyntaxKind::InterpolationEnd => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let mut diagnostics = Vec::new();
    let preflight = limits::check("source token", count, limits.tokens)
        .and_then(|()| limits::check("delimiter depth", peak_depth, MAX_DEPTH));
    let node = if let Err(mut error) = preflight {
        error.span = 0..source.len();
        diagnostics.push(error);
        None
    } else {
        // Enforce resource bounds before allocating per-token diagnostics.
        for token in &tokens {
            let text = &source[token.span.clone()];
            let message = match token.kind {
                SyntaxKind::ErrorToken => Some("invalid or unsupported token"),
                SyntaxKind::Integer if text.parse::<i64>().is_err() => {
                    Some("integer literal exceeds the Nix signed 64-bit range")
                }
                _ => None,
            };
            if let Some(message) = message {
                diagnostics.push(Diagnostic::new(token.span.clone(), message));
                if diagnostics.len() == limits::MAX_DIAGNOSTICS {
                    break;
                }
            }
        }
        let (node, errors) = if diagnostics.len() < limits::MAX_DIAGNOSTICS {
            parser::parse(&tokens, source.len())
        } else {
            (None, Vec::new())
        };
        diagnostics.extend(
            errors
                .into_iter()
                .take(limits::MAX_DIAGNOSTICS - diagnostics.len()),
        );
        // Canonical output may wrap each semantic operation in parentheses.
        if let Some(depth) = node
            .as_ref()
            .map(|node| node.depth())
            .filter(|depth| *depth > 2 * MAX_DEPTH)
        {
            let mut error = limits::exceeded("syntax tree depth", depth, 2 * MAX_DEPTH);
            error.span = 0..source.len();
            diagnostics.truncate(limits::MAX_DIAGNOSTICS - 1);
            diagnostics.push(error);
            None
        } else {
            node
        }
    };
    let green = Some(cst::build(source, &tokens, node.as_ref()));
    Parse {
        green,
        diagnostics,
        limits,
    }
}
