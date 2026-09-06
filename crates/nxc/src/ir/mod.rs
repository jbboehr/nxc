// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

/// Nix semantics, without source locations, trivia, or redundant parentheses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// A nonnegative integer literal, at most `i64::MAX`. Negation is separate.
    Integer(u64),
    Variable(String),
    AttrSet {
        recursive: bool,
        bindings: Vec<Binding>,
    },
    Select {
        value: Box<Expr>,
        path: Vec<String>,
        default: Option<Box<Expr>>,
    },
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

/// Keep source order and grouping: literal-set merges can depend on which
/// declaration introduced a recursive set. Inheritance has its own scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    Assign {
        path: Vec<String>,
        value: Expr,
    },
    Inherit {
        source: Option<Expr>,
        names: Vec<String>,
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

pub(crate) fn validate_attr_name(name: &str) -> Result<(), &'static str> {
    let mut chars = name.chars();
    if !chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '\'' | '-'))
    {
        return Err("invalid identifier");
    }
    if matches!(
        name,
        "assert" | "else" | "if" | "in" | "inherit" | "let" | "rec" | "then" | "with"
    ) {
        return Err("reserved form is not supported yet");
    }
    Ok(())
}

pub(crate) fn validate_name(name: &str) -> Result<(), &'static str> {
    validate_attr_name(name)?;
    if name.starts_with("__nxc_") || matches!(name, "__curPos" | "fn" | "yield" | "or") {
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
        let mut sets = Vec::new();
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
                Self::AttrSet { bindings, .. } => {
                    if bindings.len() > crate::MAX_TOKENS - count {
                        return Err(error("bindings exceed the node limit"));
                    }
                    count += bindings.len();
                    sets.push(bindings);
                    for binding in bindings {
                        match binding {
                            Binding::Assign { path, value } => {
                                validate_path(path, &mut count).map_err(error)?;
                                // Dotted bindings introduce implicit nested attrsets.
                                pending.push((value, depth + path.len()));
                            }
                            Binding::Inherit { source, names } => {
                                if names.len() > crate::MAX_TOKENS - count {
                                    return Err(error("inheritance exceeds the node limit"));
                                }
                                count += names.len();
                                for name in names {
                                    if source.is_some() {
                                        validate_attr_name(name)
                                    } else {
                                        validate_name(name)
                                    }
                                    .map_err(error)?;
                                }
                                if let Some(source) = source {
                                    pending.push((source, depth + 1));
                                }
                            }
                        }
                    }
                }
                Self::Select {
                    value,
                    path,
                    default,
                } => {
                    validate_path(path, &mut count).map_err(error)?;
                    pending.push((value, depth + 1));
                    if let Some(default) = default {
                        pending.push((default, depth + 1));
                    }
                }
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
        // Only inspect merge shapes after the entire IR passed depth/node bounds.
        for bindings in sets {
            validate_bindings(bindings).map_err(|message| crate::Diagnostic::new(0..0, message))?;
        }
        Ok(())
    }
}

fn validate_path(path: &[String], count: &mut usize) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("attribute path must not be empty");
    }
    if path.len() > crate::MAX_DEPTH || path.len() > crate::MAX_TOKENS - *count {
        return Err("attribute path exceeds the node or nesting limit");
    }
    *count += path.len();
    for name in path {
        validate_attr_name(name)?;
    }
    Ok(())
}

fn validate_bindings(bindings: &[Binding]) -> Result<(), &'static str> {
    use std::collections::{BTreeMap, btree_map::Entry};
    // A set can merge with another literal set; a value/inherit is a leaf.
    fn insert<'a>(
        shape: &mut BTreeMap<Vec<&'a str>, bool>,
        path: Vec<&'a str>,
        set: bool,
    ) -> Result<(), &'static str> {
        match shape.entry(path) {
            Entry::Vacant(entry) => {
                entry.insert(set);
                Ok(())
            }
            Entry::Occupied(entry) if set && *entry.get() => Ok(()),
            Entry::Occupied(_) => Err("conflicting attribute bindings"),
        }
    }
    fn visit<'a>(
        bindings: &'a [Binding],
        prefix: &[&'a str],
        shape: &mut BTreeMap<Vec<&'a str>, bool>,
    ) -> Result<(), &'static str> {
        for binding in bindings {
            match binding {
                Binding::Assign { path, value } => {
                    let mut full = prefix.to_vec();
                    for (index, name) in path.iter().enumerate() {
                        full.push(name);
                        let set = index + 1 < path.len() || matches!(value, Expr::AttrSet { .. });
                        insert(shape, full.clone(), set)?;
                    }
                    if let Expr::AttrSet { bindings, .. } = value {
                        visit(bindings, &full, shape)?;
                    }
                }
                Binding::Inherit { names, .. } => {
                    for name in names {
                        let mut full = prefix.to_vec();
                        full.push(name);
                        insert(shape, full, false)?;
                    }
                }
            }
        }
        Ok(())
    }
    visit(bindings, &[], &mut BTreeMap::new())
}
