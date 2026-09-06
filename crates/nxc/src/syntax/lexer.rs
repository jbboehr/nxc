// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use super::SyntaxKind;
use logos::Logos;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: SyntaxKind,
    pub span: Range<usize>,
}

/// Lex every byte, including trivia and invalid tokens. Spans are UTF-8 byte offsets.
pub fn lex(source: &str) -> Vec<Token> {
    SyntaxKind::lexer(source)
        .spanned()
        .map(|(kind, span)| Token {
            kind: kind.unwrap_or(SyntaxKind::ErrorToken),
            span,
        })
        .collect()
}
