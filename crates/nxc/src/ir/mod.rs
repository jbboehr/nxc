// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use crate::{Limits, limits};

mod float;
mod path;
pub use float::Float;
pub(crate) use path::lower_path;

/// Nix semantics, without source locations, trivia, or redundant parentheses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// A nonnegative integer literal, at most `i64::MAX`. Negation is separate.
    Integer(u64),
    Float(Float),
    Variable(String),
    /// A literal source-relative path, retaining its spelling without resolution.
    RelativePath(String),
    /// A literal absolute path, retaining its spelling without filesystem access.
    AbsolutePath(String),
    /// A literal home-relative path, retaining `~/` without consulting the environment.
    HomePath(String),
    /// Raw path fragments and unevaluated interpolations. The first literal
    /// contains the path prefix; literals are nonempty and nonadjacent.
    /// At least one interpolation is required. No resolution or folding occurs.
    InterpolatedPath(Vec<StringPart>),
    /// An unresolved search-path expression, including its `<...>` delimiters.
    SearchPath(String),
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
        /// One ordered path, including unevaluated dynamic key expressions.
        path: Vec<AttrName>,
        default: Option<Box<Expr>>,
    },
    /// Test one complete path without forcing the final attribute's value.
    HasAttr {
        value: Box<Expr>,
        path: Vec<AttrName>,
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

/// An attribute key. Quoted interpolation retains its `Expr::String` wrapper
/// to preserve Nix's string coercion; direct keys keep their original expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrName {
    /// Decoded text; dots within a name are not path separators.
    Static(String),
    Dynamic(Box<Expr>),
}

impl AttrName {
    /// Nix treats a direct string literal inside `${...}` as a static binding
    /// name, but keeps interpolated strings dynamic. No evaluation is involved.
    pub(crate) fn literal_name(&self) -> Option<&str> {
        match self {
            Self::Static(name) => Some(name),
            Self::Dynamic(key) => match key.as_ref() {
                Expr::String(parts) => match parts.as_slice() {
                    [] => Some(""),
                    [StringPart::Literal(name)] => Some(name),
                    _ => None,
                },
                _ => None,
            },
        }
    }
}

impl From<String> for AttrName {
    fn from(name: String) -> Self {
        Self::Static(name)
    }
}

impl From<&str> for AttrName {
    fn from(name: &str) -> Self {
        Self::Static(name.into())
    }
}

/// Literal fragments and interpolation expressions. The containing string or
/// path determines Nix's coercion and context handling.
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
        /// Ordered static/dynamic keys; dotted paths retain their implicit sets.
        path: Vec<AttrName>,
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

pub(crate) fn validate_bare_attr_name(name: &str) -> Result<(), &'static str> {
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
    validate_bare_attr_name(name)?;
    validate_scoped_name(name)
}

// Quoting a name may broaden its spelling, but does not unreserve intrinsics.
fn validate_scoped_name(name: &str) -> Result<(), &'static str> {
    if name.starts_with("__nxc_") || matches!(name, "__curPos" | "or") {
        return Err("reserved form is not supported yet");
    }
    Ok(())
}

pub(crate) fn validate_search_path(path: &str) -> Result<(), &'static str> {
    let valid = path
        .strip_prefix('<')
        .and_then(|text| text.strip_suffix('>'))
        .is_some_and(valid_path_components);
    if !valid {
        return Err("expected a search path with nonempty components inside '<...>'");
    }
    Ok(())
}

fn valid_path_components(path: &str) -> bool {
    path.split('/').all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-' | b'+'))
    })
}

pub(crate) fn validate_absolute_path(path: &str) -> Result<(), &'static str> {
    if !path.strip_prefix('/').is_some_and(valid_path_components) {
        return Err("expected a literal absolute path with nonempty components");
    }
    Ok(())
}

pub(crate) fn validate_home_path(path: &str) -> Result<(), &'static str> {
    if !path.strip_prefix("~/").is_some_and(valid_path_components) {
        return Err("expected a literal home-relative path with nonempty components");
    }
    Ok(())
}

pub(crate) fn validate_relative_path(path: &str) -> Result<(), &'static str> {
    // rnix recognizes a leading ellipsis before checking for a longer path.
    // Keep emitted literals readable by both frontends without rewriting them.
    if path.starts_with("...") {
        return Err("relative paths starting with '...' require a './' prefix");
    }
    if !path.contains('/') || !valid_path_components(path) {
        return Err("expected a literal relative path with nonempty components");
    }
    Ok(())
}

impl Expr {
    /// Equality is already canonical for this subset; no evaluation or folding occurs.
    pub fn canonical(&self) -> &Self {
        self
    }

    pub(crate) fn validate(&self, limits: Limits) -> Result<(), crate::Diagnostic> {
        let mut pending = vec![(self, 1)];
        let mut count = 0;
        let mut literal_bytes = 0;
        let mut sets = Vec::new();
        while let Some((expr, depth)) = pending.pop() {
            count += 1;
            let error = |msg| crate::Diagnostic::new(0..0, msg);
            limits::check("semantic depth", depth, crate::MAX_DEPTH)?;
            limits::check("semantic node", count, limits.tokens)?;
            match expr {
                Self::Integer(value) if *value > i64::MAX as u64 => {
                    return Err(error("integer literal exceeds the Nix signed 64-bit range"));
                }
                Self::Integer(_) => {}
                Self::Float(value) => {
                    let bytes = value.to_string().len();
                    limits::consume(
                        "semantic literal byte",
                        &mut literal_bytes,
                        bytes,
                        limits.source_bytes,
                    )?;
                }
                Self::Variable(name) => validate_name(name).map_err(error)?,
                Self::RelativePath(path) => {
                    limits::consume(
                        "semantic literal byte",
                        &mut literal_bytes,
                        path.len(),
                        limits.source_bytes,
                    )?;
                    validate_relative_path(path).map_err(error)?;
                }
                Self::SearchPath(path) => {
                    limits::consume(
                        "semantic literal byte",
                        &mut literal_bytes,
                        path.len(),
                        limits.source_bytes,
                    )?;
                    validate_search_path(path).map_err(error)?;
                }
                Self::AbsolutePath(path) => {
                    limits::consume(
                        "semantic literal byte",
                        &mut literal_bytes,
                        path.len(),
                        limits.source_bytes,
                    )?;
                    validate_absolute_path(path).map_err(error)?;
                }
                Self::HomePath(path) => {
                    limits::consume(
                        "semantic literal byte",
                        &mut literal_bytes,
                        path.len(),
                        limits.source_bytes,
                    )?;
                    validate_home_path(path).map_err(error)?;
                }
                Self::List(items) => {
                    limits::check(
                        "semantic node",
                        count.saturating_add(items.len()),
                        limits.tokens,
                    )?;
                    pending.extend(items.iter().map(|item| (item, depth + 1)));
                }
                Self::String(parts) | Self::InterpolatedPath(parts) => {
                    limits::consume("semantic node", &mut count, parts.len(), limits.tokens)?;
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
                                limits::consume(
                                    "semantic literal byte",
                                    &mut literal_bytes,
                                    text.len(),
                                    limits.source_bytes,
                                )?;
                                previous_literal = true;
                            }
                            StringPart::Interpolation(value) => {
                                pending.push((value, depth + 1));
                                previous_literal = false;
                            }
                        }
                    }
                    if matches!(expr, Self::InterpolatedPath(_)) {
                        path::validate_interpolated_path(parts).map_err(error)?;
                    }
                }
                Self::AttrSet { bindings, .. } | Self::Let { bindings, .. } => {
                    let local = matches!(expr, Self::Let { .. });
                    if let Self::Let { body, .. } = expr {
                        pending.push((body, depth + 1));
                    }
                    limits::consume("semantic node", &mut count, bindings.len(), limits.tokens)?;
                    sets.push(bindings);
                    for binding in bindings {
                        match binding {
                            Binding::Assign { path, value } => {
                                validate_path_length(path.len(), &mut count, limits)?;
                                if local {
                                    let name = path[0].literal_name().ok_or_else(||
                                        error("dynamic attributes are not allowed at the root of let bindings"))?;
                                    validate_scoped_name(name).map_err(error)?;
                                }
                                for (index, name) in path.iter().enumerate() {
                                    match name {
                                        AttrName::Static(name) => {
                                            validate_attr_name(name, &mut literal_bytes, limits)?;
                                        }
                                        AttrName::Dynamic(key) => {
                                            pending.push((key, depth + index + 1))
                                        }
                                    }
                                }
                                // Dotted bindings introduce implicit nested attrsets.
                                pending.push((value, depth + path.len()));
                            }
                            Binding::Inherit { source, names } => {
                                limits::consume(
                                    "semantic node",
                                    &mut count,
                                    names.len(),
                                    limits.tokens,
                                )?;
                                for name in names {
                                    validate_attr_name(name, &mut literal_bytes, limits)?;
                                    if source.is_none() || local {
                                        validate_scoped_name(name).map_err(error)?;
                                    }
                                }
                                if let Some(source) = source {
                                    pending.push((source, depth + 1));
                                }
                            }
                        }
                    }
                }
                Self::Select { value, path, .. } | Self::HasAttr { value, path } => {
                    validate_path_length(path.len(), &mut count, limits)?;
                    for name in path {
                        match name {
                            AttrName::Static(name) => {
                                validate_attr_name(name, &mut literal_bytes, limits)?;
                            }
                            AttrName::Dynamic(key) => pending.push((key, depth + 1)),
                        }
                    }
                    pending.push((value, depth + 1));
                    if let Self::Select {
                        default: Some(default),
                        ..
                    } = expr
                    {
                        pending.push((default, depth + 1));
                    }
                }
                Self::Lambda { parameter, body } => {
                    match parameter {
                        Pattern::Ident(name) => validate_name(name).map_err(error)?,
                        Pattern::AttrSet { fields, bind, .. } => {
                            limits::consume(
                                "semantic node",
                                &mut count,
                                fields.len(),
                                limits.tokens,
                            )?;
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

fn validate_attr_name(
    name: &str,
    literal_bytes: &mut usize,
    limits: Limits,
) -> Result<(), crate::Diagnostic> {
    limits::consume(
        "semantic literal byte",
        literal_bytes,
        name.len(),
        limits.source_bytes,
    )?;
    if name.contains('\0') {
        return Err(crate::Diagnostic::new(
            0..0,
            "attribute names cannot contain null bytes",
        ));
    }
    Ok(())
}

fn validate_path_length(
    len: usize,
    count: &mut usize,
    limits: Limits,
) -> Result<(), crate::Diagnostic> {
    if len == 0 {
        return Err(crate::Diagnostic::new(
            0..0,
            "attribute path must not be empty",
        ));
    }
    limits::check("attribute path depth", len, crate::MAX_DEPTH)?;
    limits::consume("semantic node", count, len, limits.tokens)
}

fn validate_bindings(bindings: &[Binding]) -> Result<(), &'static str> {
    use std::collections::{BTreeMap, btree_map::Entry};
    // A set can merge with another literal set; a value/inherit is a leaf.
    // Computed keys belong to separate runtime bindings. Only their static
    // prefix participates in cross-declaration merges and conflict checks.
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
                        let Some(name) = name.literal_name() else {
                            break;
                        };
                        full.push(name);
                        let set = index + 1 < path.len() || matches!(value, Expr::AttrSet { .. });
                        insert(shape, full.clone(), set)?;
                    }
                    if path.iter().all(|name| name.literal_name().is_some())
                        && let Expr::AttrSet { bindings, .. } = value
                    {
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
