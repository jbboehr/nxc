// Boundary fixtures use small explicit budgets through the production API.
// Default-capacity tests and the corpus exercise the full supported ceilings.
#![allow(dead_code, unused_imports)]

pub mod nxc {
    pub use ::nxc::{Diagnostic, MAX_DEPTH, ir};
    pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
    pub const MAX_TOKENS: usize = 16 * 1024;
    const LIMITS: ::nxc::Limits = ::nxc::Limits::new(MAX_SOURCE_BYTES, MAX_TOKENS).unwrap();

    pub fn parse_nxc(source: &str) -> Result<ir::Expr, Vec<Diagnostic>> {
        ::nxc::parse_nxc_with_limits(source, LIMITS)
    }

    pub mod syntax {
        pub use ::nxc::syntax::*;
        pub fn parse(source: &str) -> Parse {
            ::nxc::syntax::parse_with_limits(source, super::LIMITS)
        }
    }

    pub mod nix {
        use super::{Diagnostic, ir};
        pub use ::nxc::nix::Parsed;
        pub fn parse(source: &str) -> Result<Parsed, Vec<Diagnostic>> {
            ::nxc::nix::parse_with_limits(source, super::LIMITS)
        }
        pub fn import(source: &str) -> Result<ir::Expr, Vec<Diagnostic>> {
            ::nxc::nix::import_with_limits(source, super::LIMITS)
        }
        pub fn emit(expr: &ir::Expr) -> Result<String, Diagnostic> {
            ::nxc::nix::emit_with_limits(expr, super::LIMITS)
        }
    }

    pub mod emit {
        use super::{Diagnostic, ir};
        pub fn nxc(expr: &ir::Expr) -> Result<String, Diagnostic> {
            ::nxc::emit::nxc_with_limits(expr, super::LIMITS)
        }
    }
}
