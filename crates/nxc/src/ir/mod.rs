// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

/// Nix semantics, without source locations, trivia, or redundant parentheses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// A nonnegative integer literal, at most `i64::MAX`. Negation is separate.
    Integer(u64),
    Variable(String),
    /// A literal source-relative path, retaining its spelling without resolution.
    RelativePath(String),
    /// Decoded parts, with no empty or adjacent literals. An empty vector is "".
    String(Vec<StringPart>),
    /// Ordered, unevaluated elements; nesting is preserved.
    List(Vec<Expr>),
    AttrSet {
        recursive: bool,
        bindings: Vec<Binding>,
    },
    /// Lazy, mutually recursive bindings in scope for the body.
    Let {
        bindings: Vec<Binding>,
        body: Box<Expr>,
    },
    /// Expose the scope's attributes to the body using Nix's with semantics.
    With {
        scope: Box<Expr>,
        body: Box<Expr>,
    },
    /// Require a true Boolean condition before evaluating the body, as in Nix.
    Assert {
        condition: Box<Expr>,
        body: Box<Expr>,
    },
    /// Evaluate the condition, then only the selected branch, using Nix semantics.
    If {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
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
    Not(Box<Expr>),
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
}

/// Interpolations retain their expression and Nix's string coercion/context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringPart {
    Literal(String),
    Interpolation(Expr),
}

pub(crate) fn push_string_literal(parts: &mut Vec<StringPart>, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(StringPart::Literal(previous)) = parts.last_mut() {
        previous.push_str(&text);
    } else {
        parts.push(StringPart::Literal(text));
    }
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
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    And,
    Or,
    Update,
    Concat,
}

impl BinaryOp {
    pub(crate) fn spelling(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::Less => "<",
            Self::LessOrEqual => "<=",
            Self::Greater => ">",
            Self::GreaterOrEqual => ">=",
            Self::And => "&&",
            Self::Or => "||",
            Self::Update => "//",
            Self::Concat => "++",
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

pub(crate) fn validate_relative_path(path: &str) -> Result<(), &'static str> {
    // rnix recognizes a leading ellipsis before checking for a longer path.
    // Keep emitted literals readable by both frontends without rewriting them.
    if path.starts_with("...") {
        return Err("relative paths starting with '...' require a './' prefix");
    }
    if !path.contains('/')
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains("//")
        || !path
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b'+' | b'/'))
    {
        return Err("expected a literal relative path with nonempty components");
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
        let mut literal_bytes = 0;
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
                Self::RelativePath(path) => {
                    if path.len() > crate::MAX_SOURCE_BYTES - literal_bytes {
                        return Err(error("path literals exceed the source size limit"));
                    }
                    validate_relative_path(path).map_err(error)?;
                    literal_bytes += path.len();
                }
                Self::List(items) => {
                    if items.len() > crate::MAX_TOKENS - count {
                        return Err(error("list elements exceed the node limit"));
                    }
                    pending.extend(items.iter().map(|item| (item, depth + 1)));
                }
                Self::String(parts) => {
                    if parts.len() > crate::MAX_TOKENS - count {
                        return Err(error("string parts exceed the node limit"));
                    }
                    count += parts.len();
                    let mut previous_literal = false;
                    for part in parts {
                        match part {
                            StringPart::Literal(text) => {
                                if text.is_empty() || previous_literal {
                                    return Err(error(
                                        "string literals must be nonempty and nonadjacent",
                                    ));
                                }
                                if text.contains('\0') {
                                    return Err(error("Nix strings cannot contain null bytes"));
                                }
                                if text.len() > crate::MAX_SOURCE_BYTES - literal_bytes {
                                    return Err(error(
                                        "string literals exceed the source size limit",
                                    ));
                                }
                                literal_bytes += text.len();
                                previous_literal = true;
                            }
                            StringPart::Interpolation(value) => {
                                pending.push((value, depth + 1));
                                previous_literal = false;
                            }
                        }
                    }
                }
                Self::AttrSet { bindings, .. } | Self::Let { bindings, .. } => {
                    let local = matches!(expr, Self::Let { .. });
                    if let Self::Let { body, .. } = expr {
                        pending.push((body, depth + 1));
                    }
                    if bindings.len() > crate::MAX_TOKENS - count {
                        return Err(error("bindings exceed the node limit"));
                    }
                    count += bindings.len();
                    sets.push(bindings);
                    for binding in bindings {
                        match binding {
                            Binding::Assign { path, value } => {
                                validate_path(path, &mut count).map_err(error)?;
                                if local {
                                    validate_name(&path[0]).map_err(error)?;
                                }
                                // Dotted bindings introduce implicit nested attrsets.
                                pending.push((value, depth + path.len()));
                            }
                            Binding::Inherit { source, names } => {
                                if names.len() > crate::MAX_TOKENS - count {
                                    return Err(error("inheritance exceeds the node limit"));
                                }
                                count += names.len();
                                for name in names {
                                    if source.is_some() && !local {
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
                Self::Negate(expr) | Self::Not(expr) => pending.push((expr, depth + 1)),
                Self::With { scope, body } => {
                    pending.extend([(scope.as_ref(), depth + 1), (body.as_ref(), depth + 1)])
                }
                Self::Assert { condition, body } => {
                    pending.extend([(condition.as_ref(), depth + 1), (body.as_ref(), depth + 1)])
                }
                Self::If {
                    condition,
                    then_branch,
                    else_branch,
                } => pending.extend([
                    (condition.as_ref(), depth + 1),
                    (then_branch.as_ref(), depth + 1),
                    (else_branch.as_ref(), depth + 1),
                ]),
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
