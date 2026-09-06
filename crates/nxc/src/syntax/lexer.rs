// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use super::SyntaxKind as K;
use logos::Logos;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: K,
    pub span: Range<usize>,
}

/// Lex every byte, including trivia and invalid tokens. Spans are UTF-8 byte offsets.
pub fn lex(source: &str) -> Vec<Token> {
    enum Mode {
        String,
        Interpolation { braces: usize },
    }

    let mut lexer = K::lexer(source);
    let mut modes = Vec::new();
    let mut tokens = Vec::new();
    loop {
        if matches!(modes.last(), Some(Mode::String)) {
            let text = lexer.remainder();
            let start = source.len() - text.len();
            if text.is_empty() {
                break;
            }
            let (kind, len) = if text.starts_with('"') {
                modes.pop();
                (K::StringEnd, 1)
            } else if text.starts_with("${") {
                modes.push(Mode::Interpolation { braces: 0 });
                (K::InterpolationStart, 2)
            } else {
                (K::StringContent, string_content_len(text))
            };
            lexer.bump(len);
            tokens.push(Token {
                kind,
                span: start..start + len,
            });
            continue;
        }

        let Some(kind) = lexer.next() else { break };
        let mut kind = kind.unwrap_or(K::ErrorToken);
        match kind {
            K::StringStart => modes.push(Mode::String),
            K::LBrace => {
                if let Some(Mode::Interpolation { braces }) = modes.last_mut() {
                    *braces += 1;
                }
            }
            K::RBrace => {
                if let Some(Mode::Interpolation { braces }) = modes.last_mut() {
                    if *braces == 0 {
                        kind = K::InterpolationEnd;
                        modes.pop();
                    } else {
                        *braces -= 1;
                    }
                }
            }
            _ => {}
        }
        tokens.push(Token {
            kind,
            span: lexer.span(),
        });
    }
    tokens
}

fn string_content_len(text: &str) -> usize {
    let mut chars = text.char_indices().peekable();
    while let Some((index, character)) = chars.next() {
        match character {
            '"' => return index,
            '\\' => {
                chars.next();
            }
            // Nix consumes paired dollars as literal text, even before '{'.
            '$' => match chars.peek() {
                Some((_, '$')) => {
                    chars.next();
                }
                Some((_, '{')) => return index,
                _ => {}
            },
            _ => {}
        }
    }
    text.len()
}
