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
        String { indented: bool },
        Interpolation { braces: usize },
        Path,
    }

    let mut lexer = K::lexer(source);
    let mut modes = Vec::new();
    let mut tokens = Vec::new();
    loop {
        if let Some(Mode::Path) = modes.last() {
            let text = lexer.remainder();
            let start = source.len() - text.len();
            let len = text
                .bytes()
                .take_while(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b'+' | b'/')
                })
                .count();
            let (kind, len) = if text.starts_with("${") {
                modes.push(Mode::Interpolation { braces: 0 });
                (K::InterpolationStart, 2)
            } else if len != 0 {
                (K::PathContent, len)
            } else {
                modes.pop();
                continue;
            };
            lexer.bump(len);
            tokens.push(Token {
                kind,
                span: start..start + len,
            });
            continue;
        }
        if let Some(&Mode::String { indented }) = modes.last() {
            let text = lexer.remainder();
            let start = source.len() - text.len();
            if text.is_empty() {
                break;
            }
            let closing = if indented {
                text.starts_with("''")
                    && !text.starts_with("'''")
                    && !text.starts_with("''$")
                    && !text.starts_with("''\\")
            } else {
                text.starts_with('"')
            };
            let (kind, len) = if closing {
                modes.pop();
                (K::StringEnd, if indented { 2 } else { 1 })
            } else if text.starts_with("${") {
                modes.push(Mode::Interpolation { braces: 0 });
                (K::InterpolationStart, 2)
            } else {
                (K::StringContent, string_content_len(text, indented))
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
            K::PathStart => {
                let span = lexer.span();
                tokens.push(Token {
                    kind,
                    span: span.start..span.end - 2,
                });
                tokens.push(Token {
                    kind: K::InterpolationStart,
                    span: span.end - 2..span.end,
                });
                modes.push(Mode::Path);
                modes.push(Mode::Interpolation { braces: 0 });
                continue;
            }
            K::RelativePath | K::AbsolutePath | K::HomePath
                if lexer.remainder().starts_with("${") =>
            {
                kind = K::PathStart;
                modes.push(Mode::Path);
            }
            K::InterpolationStart => modes.push(Mode::Interpolation { braces: 0 }),
            K::StringStart => modes.push(Mode::String {
                indented: lexer.slice() == "''",
            }),
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

fn string_content_len(text: &str, indented: bool) -> usize {
    let mut chars = text.char_indices().peekable();
    while let Some((index, character)) = chars.next() {
        match character {
            '"' if !indented => return index,
            '\\' if !indented => {
                chars.next();
            }
            '\'' if indented && matches!(chars.peek(), Some((_, '\''))) => {
                chars.next();
                match chars.peek() {
                    Some((_, '\'' | '$')) => {
                        chars.next();
                    }
                    Some((_, '\\')) => {
                        chars.next();
                        chars.next();
                    }
                    _ => return index,
                }
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
