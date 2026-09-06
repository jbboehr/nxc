// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

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
