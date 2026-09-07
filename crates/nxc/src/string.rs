// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use crate::ir::{Expr, StringPart, push_string_literal};
use std::borrow::Cow;

/// Normalize raw literal fragments and already-lowered interpolations.
pub(crate) fn lower(mut parts: Vec<StringPart>, indented: bool) -> Result<Expr, &'static str> {
    if !indented {
        let mut result = Vec::new();
        for part in parts {
            match part {
                StringPart::Literal(text) => push_string_literal(&mut result, decode(&text)?),
                interpolation => result.push(interpolation),
            }
        }
        return Ok(Expr::String(result));
    }

    // Only spaces followed by LF belong to Nix's indented opening delimiter.
    if let Some(StringPart::Literal(text)) = parts.first_mut() {
        let spaces = text.bytes().take_while(|b| *b == b' ').count();
        if text.as_bytes().get(spaces) == Some(&b'\n') {
            text.drain(..spaces + 1);
        }
    }

    // Match Nix's finite initial minimum, including all-blank large strings.
    let mut minimum = 1_000_000;
    let mut indentation = 0;
    let mut at_start = true;
    for part in &parts {
        match part {
            StringPart::Interpolation(_) => {
                if at_start {
                    minimum = minimum.min(indentation);
                }
                at_start = false;
            }
            StringPart::Literal(text) => {
                for chunk in indented_chunks(text) {
                    let (text, raw) = chunk?;
                    // Even an escaped newline or space ends indentation here.
                    if !raw {
                        if at_start {
                            minimum = minimum.min(indentation);
                        }
                        at_start = false;
                        continue;
                    }
                    for c in text.chars() {
                        if c == '\n' {
                            at_start = true;
                            indentation = 0;
                        } else if at_start {
                            if c == ' ' {
                                indentation += 1;
                            } else {
                                minimum = minimum.min(indentation);
                                at_start = false;
                            }
                        }
                    }
                }
            }
        }
    }

    let mut result = Vec::new();
    at_start = true;
    let mut dropped = 0;
    let last_part = parts.len().saturating_sub(1);
    for (index, part) in parts.into_iter().enumerate() {
        match part {
            StringPart::Interpolation(_) => {
                at_start = false;
                dropped = 0;
                result.push(part);
            }
            StringPart::Literal(text) => {
                let mut chunks = indented_chunks(&text).peekable();
                while let Some(chunk) = chunks.next() {
                    let (text, raw) = chunk?;
                    let decoded = if raw {
                        Cow::Borrowed(text)
                    } else if text == "'''" {
                        Cow::Borrowed("''")
                    } else if text == "''$" {
                        Cow::Borrowed("$")
                    } else {
                        Cow::Owned(decode(&text[2..])?)
                    };
                    let mut stripped = String::new();
                    for c in decoded.chars() {
                        if c == '\0' {
                            return Err("Nix strings cannot contain null bytes");
                        }
                        if at_start && c == ' ' {
                            if dropped >= minimum {
                                stripped.push(c);
                            }
                            dropped += 1;
                        } else {
                            stripped.push(c);
                            if c == '\n' {
                                at_start = true;
                                dropped = 0;
                            } else {
                                at_start = false;
                                dropped = 0;
                            }
                        }
                    }
                    // Trim only within the final lexical fragment. An escape
                    // splits fragments even though the resulting IR merges them.
                    if index == last_part
                        && chunks.peek().is_none()
                        && let Some(newline) = stripped.rfind('\n')
                        && stripped[newline + 1..].bytes().all(|b| b == b' ')
                    {
                        stripped.truncate(newline + 1);
                    }
                    push_string_literal(&mut result, stripped);
                }
            }
        }
    }
    Ok(Expr::String(result))
}

/// Borrow raw runs and escapes separately, without allocating per character.
fn indented_chunks(mut text: &str) -> impl Iterator<Item = Result<(&str, bool), &'static str>> {
    std::iter::from_fn(move || {
        if text.is_empty() {
            return None;
        }
        let raw = !text.starts_with("''");
        let len = if raw {
            text.find("''").unwrap_or(text.len())
        } else if text.starts_with("'''") || text.starts_with("''$") {
            3
        } else if text.starts_with("''\\") {
            match text[3..].chars().next() {
                Some(c) => 3 + c.len_utf8(),
                None => {
                    text = "";
                    return Some(Err("unterminated indented string escape"));
                }
            }
        } else {
            text = "";
            return Some(Err("unexpected indented string delimiter"));
        };
        let (chunk, rest) = text.split_at(len);
        text = rest;
        Some(Ok((chunk, raw)))
    })
}

/// Decode a double-quoted Nix string fragment, without its delimiters.
pub(crate) fn decode(text: &str) -> Result<String, &'static str> {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        let character = match character {
            '\\' => match chars.next().ok_or("unterminated string escape")? {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            },
            // Nix normalizes unescaped CR/CRLF, but preserves escaped CR.
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                '\n'
            }
            other => other,
        };
        if character == '\0' {
            return Err("Nix strings cannot contain null bytes");
        }
        result.push(character);
    }
    Ok(result)
}
