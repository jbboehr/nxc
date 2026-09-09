// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use crate::{Diagnostic, MAX_SOURCE_BYTES, MAX_TOKENS};

/// Per-operation byte and token/node budgets. Nesting remains bounded by
/// `MAX_DEPTH`; callers may reduce these budgets below the library ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub(crate) source_bytes: usize,
    pub(crate) tokens: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            source_bytes: MAX_SOURCE_BYTES,
            tokens: MAX_TOKENS,
        }
    }
}

impl Limits {
    /// Return a budget within the supported ceilings. Zero is permitted.
    pub const fn new(source_bytes: usize, tokens: usize) -> Option<Self> {
        if source_bytes > MAX_SOURCE_BYTES || tokens > MAX_TOKENS {
            None
        } else {
            Some(Self {
                source_bytes,
                tokens,
            })
        }
    }

    pub const fn source_bytes(self) -> usize {
        self.source_bytes
    }
    pub const fn tokens(self) -> usize {
        self.tokens
    }

    /// Check bytes already observed by a reader. For a capped read, `observed`
    /// may be only a lower bound on the complete file size.
    pub fn check_source_bytes(self, observed: usize) -> Result<(), Diagnostic> {
        check("source byte", observed, self.source_bytes).map_err(|mut error| {
            error.span = 0..observed;
            error
        })
    }
}

pub(crate) const MAX_DIAGNOSTICS: usize = 100;

// Lowering has larger frames than tree traversal, especially in debug builds.
// Keep room for the next frame on caller-owned stacks while retaining MAX_DEPTH.
pub(crate) fn with_stack<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, f)
}

pub(crate) fn exceeded(kind: &str, observed: usize, limit: usize) -> Diagnostic {
    Diagnostic::new(
        0..0,
        format!("{kind} limit exceeded: observed {observed}, limit {limit}"),
    )
}

pub(crate) fn check(kind: &str, observed: usize, limit: usize) -> Result<(), Diagnostic> {
    if observed > limit {
        Err(exceeded(kind, observed, limit))
    } else {
        Ok(())
    }
}

pub(crate) fn consume(
    kind: &str,
    count: &mut usize,
    additional: usize,
    limit: usize,
) -> Result<(), Diagnostic> {
    *count = count.saturating_add(additional);
    check(kind, *count, limit)
}
