// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use crate::syntax::SyntaxKind;
use logos::Logos;
use std::{fmt, str::FromStr};

/// A finite, nonnegative binary64 literal. Negation remains an IR operation.
/// Equality compares the represented value, independently of decimal spelling.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Float(u64);

impl Float {
    pub fn new(value: f64) -> Result<Self, &'static str> {
        if !value.is_finite() || value.is_sign_negative() {
            return Err("float literals must be finite and nonnegative; use negation separately");
        }
        Ok(Self(value.to_bits()))
    }

    pub fn value(self) -> f64 {
        f64::from_bits(self.0)
    }
}

impl FromStr for Float {
    type Err = &'static str;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        if source.len() > crate::MAX_SOURCE_BYTES {
            return Err("float literal exceeds the source size limit");
        }
        let mut lexer = SyntaxKind::lexer(source);
        if lexer.next() != Some(Ok(SyntaxKind::Float)) || lexer.span().end != source.len() {
            return Err("expected a Nix float literal with a decimal point");
        }
        let value = source.parse::<f64>().map_err(|_| "invalid float literal")?;
        let result = Self::new(value)?;
        let mantissa = source.split(['e', 'E']).next().unwrap();
        if value == 0.0 && mantissa.bytes().any(|c| matches!(c, b'1'..=b'9')) {
            return Err("float literal underflows the binary64 range");
        }
        if value > 0.0 && value <= f64::MIN_POSITIVE {
            let exact = format!("{value:.1074}");
            let original = decimal_key(source);
            let represented = decimal_key(&exact);
            // Nix uses libc strtod and rejects ERANGE. Inexact subnormals
            // underflow; tininess just below the smallest normal also depends
            // on libc's boundary handling. Reject that boundary conservatively.
            if original != represented && (value.is_subnormal() || original < represented) {
                return Err("inexact underflow-edge float spellings are not supported");
            }
        }
        Ok(result)
    }
}

// Compare positive decimal values exactly without rounding through binary64.
// Equal decimal order makes lexicographic significand comparison sufficient;
// removing trailing zeros gives all equivalent spellings the same key.
fn decimal_key(source: &str) -> (i64, String) {
    let (mantissa, exponent) = source.split_once(['e', 'E']).unwrap_or((source, "0"));
    let exponent = exponent.parse::<i64>().unwrap_or_else(|_| {
        if exponent.starts_with('-') {
            i64::MIN
        } else {
            i64::MAX
        }
    });
    let fractional = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let digits: String = mantissa
        .bytes()
        .filter(|&c| c != b'.')
        .map(char::from)
        .collect();
    let digits = digits.trim_start_matches('0');
    let order = exponent
        .saturating_add(digits.len() as i64)
        .saturating_sub(fractional as i64);
    (order, digits.trim_end_matches('0').to_owned())
}

impl fmt::Display for Float {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.value();
        if value.is_subnormal() || value == f64::MIN_POSITIVE {
            // Every binary64 value has an exact decimal expansion within 1074
            // fractional places. Exact spellings avoid libc underflow errors.
            let exact = format!("{value:.1074}");
            return f.write_str(exact.trim_end_matches('0'));
        }
        let spelling = format!("{value:?}");
        // Rust's shortest representation can omit the decimal point in an
        // exponent form; Nix requires it to distinguish floats from application.
        if !spelling.contains('.') {
            if let Some((mantissa, exponent)) = spelling.split_once('e') {
                return write!(f, "{mantissa}.0e{exponent}");
            }
            return write!(f, "{spelling}.0");
        }
        f.write_str(&spelling)
    }
}

impl fmt::Debug for Float {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value().fmt(f)
    }
}
