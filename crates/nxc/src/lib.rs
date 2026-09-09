// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

//! A C/Rust-flavored concrete syntax for Nix with unchanged evaluation semantics.
//!
pub mod emit;
pub mod ir;
mod limits;
pub use limits::Limits;
pub mod nix;
mod string;
pub mod syntax;

/// Resource bounds shared by source parsing, IR validation, and emission.
pub const MAX_SOURCE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_TOKENS: usize = 4 * 1024 * 1024;
pub const MAX_DEPTH: usize = 256;

use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Range<usize>,
    pub message: String,
}

impl Diagnostic {
    pub(crate) fn new(span: Range<usize>, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

/// Lower nxc source into a syntax-independent semantic expression.
pub fn parse_nxc(source: &str) -> Result<ir::Expr, Vec<Diagnostic>> {
    syntax::parse(source).lower()
}

/// Lower nxc using explicitly selected byte and token/node budgets.
pub fn parse_nxc_with_limits(source: &str, limits: Limits) -> Result<ir::Expr, Vec<Diagnostic>> {
    syntax::parse_with_limits(source, limits).lower()
}
