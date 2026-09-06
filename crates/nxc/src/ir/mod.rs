// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

/// Nix semantics, without source locations, trivia, or redundant parentheses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// A nonnegative integer literal, at most `i64::MAX`. Negation is separate.
    Integer(u64),
    Variable(String),
    Lambda {
        parameter: Pattern,
        body: Box<Expr>,
    },
    Apply {
        function: Box<Expr>,
        argument: Box<Expr>,
    },
    Negate(Box<Expr>),
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
}

/// A single Nix argument, optionally destructured into named attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Ident(String),
    AttrSet {
        fields: Vec<Formal>,
        ellipsis: bool,
        bind: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formal {
    pub name: String,
    /// Evaluated lazily in the parameter scope when the attribute is absent.
    pub default: Option<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
}

impl BinaryOp {
    pub(crate) fn spelling(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
        }
    }
}

pub(crate) fn validate_name(name: &str) -> Result<(), &'static str> {
    let mut chars = name.chars();
    if !chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '\'' | '-'))
    {
        return Err("invalid identifier");
    }
    if name.starts_with("__nxc_")
        || matches!(
            name,
            "__curPos"
                | "assert"
                | "else"
                | "fn"
                | "if"
                | "in"
                | "inherit"
                | "let"
                | "or"
                | "rec"
                | "then"
                | "with"
                | "yield"
        )
    {
        return Err("reserved form is not supported yet");
    }
    Ok(())
}

impl Expr {
    /// Equality is already canonical for this subset; no evaluation or folding occurs.
    pub fn canonical(&self) -> &Self {
        self
    }

    pub(crate) fn validate(&self) -> Result<(), crate::Diagnostic> {
        let mut pending = vec![(self, 1)];
        let mut count = 0;
        while let Some((expr, depth)) = pending.pop() {
            count += 1;
            let error = |msg| crate::Diagnostic::new(0..0, msg);
            if depth > crate::MAX_DEPTH || count > crate::MAX_TOKENS {
                return Err(error("expression exceeds the node or nesting limit"));
            }
            match expr {
                Self::Integer(value) if *value > i64::MAX as u64 => {
                    return Err(error("integer literal exceeds the Nix signed 64-bit range"));
                }
                Self::Integer(_) => {}
                Self::Variable(name) => validate_name(name).map_err(error)?,
                Self::Lambda { parameter, body } => {
                    match parameter {
                        Pattern::Ident(name) => validate_name(name).map_err(error)?,
                        Pattern::AttrSet { fields, bind, .. } => {
                            if fields.len() > crate::MAX_TOKENS - count {
                                return Err(error("pattern exceeds the node limit"));
                            }
                            count += fields.len();
                            let mut names = std::collections::BTreeSet::new();
                            if let Some(name) = bind {
                                validate_name(name).map_err(error)?;
                                names.insert(name.as_str());
                            }
                            for field in fields {
                                validate_name(&field.name).map_err(error)?;
                                if !names.insert(field.name.as_str()) {
                                    return Err(error("duplicate lambda parameter"));
                                }
                                if let Some(default) = &field.default {
                                    pending.push((default, depth + 1));
                                }
                            }
                        }
                    }
                    pending.push((body, depth + 1));
                }
                Self::Negate(expr) => pending.push((expr, depth + 1)),
                Self::Apply { function, argument } => pending.extend([
                    (function.as_ref(), depth + 1),
                    (argument.as_ref(), depth + 1),
                ]),
                Self::Binary { left, right, .. } => {
                    pending.extend([(left.as_ref(), depth + 1), (right.as_ref(), depth + 1)])
                }
            }
        }
        Ok(())
    }
}
