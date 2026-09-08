// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use crate::string::StringContext;
use crate::{
    Diagnostic, MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS,
    ir::{self, AttrName, BinaryOp, Binding, Expr, Formal, Pattern, StringPart},
};
use rnix::{
    SyntaxKind as K,
    ast::{self, AstToken, HasEntry, InterpolPart},
};

/// Parse native Nix through rnix and adapt only supported forms to the semantic IR.
/// Native syntax-node types never cross this module's public boundary.
pub fn import(source: &str) -> Result<Expr, Vec<Diagnostic>> {
    parse(source)?.lower()
}

/// A native syntax tree that passed parsing and compatibility/resource checks.
#[derive(Debug, Clone)]
pub struct Parsed {
    root: Option<ast::Root>,
    source_len: usize,
}

impl Drop for Parsed {
    fn drop(&mut self) {
        let Some(root) = self.root.take() else {
            return;
        };
        // Native parsing can produce long, flat operator chains that lowering
        // rejects for semantic depth. Rowan frees green children recursively;
        // retain each node's children before releasing it to bound stack use.
        let mut pending = vec![syntax(&root).green().into_owned()];
        drop(root);
        while let Some(node) = pending.pop() {
            pending.extend(
                node.children()
                    .filter_map(|child| child.into_node())
                    .map(ToOwned::to_owned),
            );
        }
    }
}

impl Parsed {
    /// Adapt supported native forms to the semantic IR without reparsing.
    pub fn lower(&self) -> Result<Expr, Vec<Diagnostic>> {
        let root = self
            .root
            .as_ref()
            .expect("native tree exists until drop")
            .expr()
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    0..self.source_len,
                    "missing native Nix expression",
                )]
            })?;
        let result = lower(root, 1).map_err(|e| vec![e])?;
        result.validate().map_err(|mut e| {
            e.span = 0..self.source_len;
            vec![e]
        })?;
        Ok(result)
    }
}

/// Parse native syntax separately from lowering, including preflight checks.
/// Parsing success does not imply that the syntax is supported by lowering.
pub fn parse(source: &str) -> Result<Parsed, Vec<Diagnostic>> {
    check_source(source).map_err(|e| vec![e])?;
    let parsed = rnix::Root::parse(source);
    let root = Parsed {
        root: Some(parsed.tree()),
        source_len: source.len(),
    };
    let errors: Vec<_> = parsed
        .errors()
        .iter()
        .map(|e| {
            use rnix::ParseError::*;
            let span = match e {
                Unexpected(r)
                | UnexpectedExtra(r)
                | UnexpectedWanted(_, r, _)
                | UnexpectedDoubleBind(r)
                | DuplicatedArgs(r, _) => usize::from(r.start())..usize::from(r.end()),
                _ => source.len()..source.len(),
            };
            Diagnostic::new(span, format!("native Nix parse error: {e}"))
        })
        .collect();
    // Release rnix's shared owner first so our iterative drop also covers errors.
    drop(parsed);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(root)
}

pub(super) fn check_source(source: &str) -> Result<(), Diagnostic> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(Diagnostic::new(
            0..source.len(),
            "source exceeds the 1 MiB limit",
        ));
    }
    let mut count = 0;
    let mut depth = 0usize;
    let mut offset = 0;
    for (kind, text) in rnix::tokenize(source) {
        let start = offset;
        offset += text.len();
        if kind == K::TOKEN_WHITESPACE {
            // rnix accepts Unicode whitespace beyond Nix's four ASCII characters.
            if let Some((index, character)) = text
                .char_indices()
                .find(|(_, c)| !matches!(c, ' ' | '\t' | '\r' | '\n'))
            {
                return Err(Diagnostic::new(
                    start + index..start + index + character.len_utf8(),
                    "whitespace is not accepted by native Nix",
                ));
            }
            continue;
        }
        if kind == K::TOKEN_COMMENT {
            // rnix ends # comments only at LF; Nix also ends them at CR.
            if text.starts_with('#')
                && let Some(index) = text.find('\r')
                && source.as_bytes().get(start + index + 1) != Some(&b'\n')
            {
                return Err(Diagnostic::new(
                    start + index..start + index + 1,
                    "bare-CR native line comments are not supported; use LF or CRLF line endings",
                ));
            }
            continue;
        }
        count += 1;
        match kind {
            K::TOKEN_L_PAREN
            | K::TOKEN_L_BRACE
            | K::TOKEN_L_BRACK
            | K::TOKEN_STRING_START
            | K::TOKEN_INTERPOL_START => depth += 1,
            K::TOKEN_R_PAREN
            | K::TOKEN_R_BRACE
            | K::TOKEN_R_BRACK
            | K::TOKEN_STRING_END
            | K::TOKEN_INTERPOL_END => depth = depth.saturating_sub(1),
            _ => {}
        }
        if count > MAX_TOKENS || depth > MAX_DEPTH {
            return Err(Diagnostic::new(
                0..source.len(),
                "expression exceeds the token or nesting limit",
            ));
        }
    }
    Ok(())
}

// Use rnix's marker trait to access its own Rowan version inside the adapter.
fn syntax(node: &impl ast::AstNode) -> &rnix::SyntaxNode {
    node.syntax()
}

fn lower(node: ast::Expr, depth: usize) -> Result<Expr, Diagnostic> {
    lower_with_string_context(node, depth, StringContext::Value)
}

fn lower_with_string_context(
    mut node: ast::Expr,
    depth: usize,
    context: StringContext,
) -> Result<Expr, Diagnostic> {
    // Parentheses do not add semantic depth. Unwrap them without adding stack
    // frames so fully parenthesized output still fits the supported depth.
    while let ast::Expr::Paren(paren) = node {
        node = paren.expr().ok_or_else(|| {
            let range = syntax(&paren).text_range();
            Diagnostic::new(
                usize::from(range.start())..usize::from(range.end()),
                "missing parenthesized expression",
            )
        })?;
    }
    let range = syntax(&node).text_range();
    let span = usize::from(range.start())..usize::from(range.end());
    let error = |message: &str| Diagnostic::new(span.clone(), message);
    if depth > MAX_DEPTH {
        return Err(error("expression exceeds the nesting limit"));
    }
    let child = |node: Option<ast::Expr>| {
        lower(
            node.ok_or_else(|| error("missing native Nix operand"))?,
            depth + 1,
        )
    };
    let operator_child = |node: Option<ast::Expr>| {
        // rnix accepts bare lambdas as operator operands, while Nix requires
        // parentheses. Check before lowering removes the AST wrapper.
        if matches!(node, Some(ast::Expr::Lambda(_))) {
            return Err(error("native lambda operator operands require parentheses"));
        }
        child(node)
    };
    match node {
        ast::Expr::Ident(ident) => {
            let name = syntax(&ident).text().to_string();
            ir::validate_name(&name).map_err(error)?;
            Ok(Expr::Variable(name))
        }
        ast::Expr::PathRel(path) => {
            let path = syntax(&path).text().to_string();
            ir::validate_relative_path(&path).map_err(error)?;
            Ok(Expr::RelativePath(path))
        }
        ast::Expr::PathSearch(path) => lower_search_path(path),
        ast::Expr::PathAbs(path) => lower_absolute_path(path),
        ast::Expr::PathHome(path) => lower_home_path(path),
        ast::Expr::Literal(literal) => {
            let token = syntax(&literal)
                .first_token()
                .ok_or_else(|| error("missing literal"))?;
            if token.kind() == K::TOKEN_FLOAT {
                return Ok(Expr::Float(token.text().parse().map_err(error)?));
            }
            if token.kind() != K::TOKEN_INTEGER {
                return Err(error("native literal is not supported yet"));
            }
            let value = token
                .text()
                .parse::<i64>()
                .map_err(|_| error("integer literal exceeds the Nix signed 64-bit range"))?;
            Ok(Expr::Integer(value as u64))
        }
        ast::Expr::Apply(apply) => {
            let argument = apply.argument();
            // rnix accepts bare lambda arguments, but Nix requires parentheses.
            // Check before child lowering removes the parenthesized wrapper.
            if matches!(argument, Some(ast::Expr::Lambda(_))) {
                return Err(error("native lambda arguments require parentheses"));
            }
            Ok(Expr::Apply {
                function: Box::new(child(apply.lambda())?),
                argument: Box::new(child(argument)?),
            })
        }
        ast::Expr::List(list) => lower_list(list, depth),
        ast::Expr::Str(string) => lower_string(string, depth, context),
        ast::Expr::AttrSet(set) => Ok(Expr::AttrSet {
            recursive: set.rec_token().is_some(),
            bindings: lower_bindings(&set, depth)?,
        }),
        ast::Expr::LetIn(local) => Ok(Expr::Let {
            bindings: lower_bindings(&local, depth)?,
            body: Box::new(child(local.body())?),
        }),
        ast::Expr::With(with) => Ok(Expr::With {
            scope: Box::new(child(with.namespace())?),
            body: Box::new(child(with.body())?),
        }),
        ast::Expr::Assert(assertion) => Ok(Expr::Assert {
            condition: Box::new(child(assertion.condition())?),
            body: Box::new(child(assertion.body())?),
        }),
        ast::Expr::IfElse(conditional) => Ok(Expr::If {
            condition: Box::new(child(conditional.condition())?),
            then_branch: Box::new(child(conditional.body())?),
            else_branch: Box::new(child(conditional.else_body())?),
        }),
        ast::Expr::Select(select) => Ok(Expr::Select {
            value: Box::new(child(select.expr())?),
            path: lower_lookup_path(select.attrpath(), depth)?,
            default: select
                .default_expr()
                .map(|value| {
                    // rnix accepts bare lambdas here, but Nix requires parentheses.
                    // Check before lowering erases the parenthesized AST wrapper.
                    if matches!(value, ast::Expr::Lambda(_)) {
                        let range = syntax(&value).text_range();
                        return Err(Diagnostic::new(
                            usize::from(range.start())..usize::from(range.end()),
                            "native lambda selection defaults require parentheses",
                        ));
                    }
                    lower(value, depth + 1).map(Box::new)
                })
                .transpose()?,
        }),
        ast::Expr::HasAttr(has_attr) => Ok(Expr::HasAttr {
            value: Box::new(operator_child(has_attr.expr())?),
            path: lower_lookup_path(has_attr.attrpath(), depth)?,
        }),
        ast::Expr::Lambda(lambda) => {
            let name = |ident: Option<ast::Ident>| {
                ident
                    .map(|ident| syntax(&ident).text().to_string())
                    .ok_or_else(|| error("missing lambda parameter name"))
            };
            let parameter = match lambda
                .param()
                .ok_or_else(|| error("missing lambda parameter"))?
            {
                ast::Param::IdentParam(param) => Pattern::Ident(name(param.ident())?),
                ast::Param::Pattern(pattern) => Pattern::AttrSet {
                    fields: pattern
                        .pat_entries()
                        .map(|field| {
                            Ok(Formal {
                                name: name(field.ident())?,
                                default: field
                                    .default()
                                    .map(|expr| lower(expr, depth + 1))
                                    .transpose()?,
                            })
                        })
                        .collect::<Result<_, Diagnostic>>()?,
                    ellipsis: pattern.ellipsis_token().is_some(),
                    bind: pattern
                        .pat_bind()
                        .map(|bind| name(bind.ident()))
                        .transpose()?,
                },
            };
            Ok(Expr::Lambda {
                parameter,
                body: Box::new(child(lambda.body())?),
            })
        }
        ast::Expr::UnaryOp(unary) if unary.operator() == Some(ast::UnaryOpKind::Negate) => {
            Ok(Expr::Negate(Box::new(operator_child(unary.expr())?)))
        }
        ast::Expr::UnaryOp(unary) if unary.operator() == Some(ast::UnaryOpKind::Invert) => {
            Ok(Expr::Not(Box::new(operator_child(unary.expr())?)))
        }
        ast::Expr::BinOp(binary) => lower_binary(binary, operator_child),
        other => Err(error(&format!(
            "native Nix form {:?} is not supported yet",
            syntax(&other).kind()
        ))),
    }
}

// Keep literal collection out of the recursive importer frame.
fn lower_home_path(path: ast::PathHome) -> Result<Expr, Diagnostic> {
    let range = syntax(&path).text_range();
    let path = syntax(&path).text().to_string();
    ir::validate_home_path(&path).map_err(|message| {
        Diagnostic::new(
            usize::from(range.start())..usize::from(range.end()),
            message,
        )
    })?;
    Ok(Expr::HomePath(path))
}

fn lower_absolute_path(path: ast::PathAbs) -> Result<Expr, Diagnostic> {
    let range = syntax(&path).text_range();
    let path = syntax(&path).text().to_string();
    ir::validate_absolute_path(&path).map_err(|message| {
        Diagnostic::new(
            usize::from(range.start())..usize::from(range.end()),
            message,
        )
    })?;
    Ok(Expr::AbsolutePath(path))
}

fn lower_search_path(path: ast::PathSearch) -> Result<Expr, Diagnostic> {
    let range = syntax(&path).text_range();
    let path = syntax(&path).text().to_string();
    ir::validate_search_path(&path).map_err(|message| {
        Diagnostic::new(
            usize::from(range.start())..usize::from(range.end()),
            message,
        )
    })?;
    Ok(Expr::SearchPath(path))
}

// Keep binary construction out of the recursive lower frame so adding an
// operator does not increase stack usage for every nested expression kind.
fn lower_binary(
    binary: ast::BinOp,
    operand: impl Fn(Option<ast::Expr>) -> Result<Expr, Diagnostic>,
) -> Result<Expr, Diagnostic> {
    let native_op = binary.operator();
    let op = match native_op {
        Some(ast::BinOpKind::Add) => BinaryOp::Add,
        Some(ast::BinOpKind::Sub) => BinaryOp::Subtract,
        Some(ast::BinOpKind::Mul) => BinaryOp::Multiply,
        Some(ast::BinOpKind::Div) => BinaryOp::Divide,
        Some(ast::BinOpKind::Equal) => BinaryOp::Equal,
        Some(ast::BinOpKind::NotEqual) => BinaryOp::NotEqual,
        Some(ast::BinOpKind::Less) => BinaryOp::Less,
        Some(ast::BinOpKind::LessOrEq) => BinaryOp::LessOrEqual,
        Some(ast::BinOpKind::More) => BinaryOp::Greater,
        Some(ast::BinOpKind::MoreOrEq) => BinaryOp::GreaterOrEqual,
        Some(ast::BinOpKind::And) => BinaryOp::And,
        Some(ast::BinOpKind::Or | ast::BinOpKind::Implication) => BinaryOp::Or,
        Some(ast::BinOpKind::Update) => BinaryOp::Update,
        Some(ast::BinOpKind::Concat) => BinaryOp::Concat,
        _ => {
            let range = syntax(&binary).text_range();
            return Err(Diagnostic::new(
                usize::from(range.start())..usize::from(range.end()),
                "native Nix operator is not supported yet",
            ));
        }
    };
    let left = operand(binary.lhs())?;
    // Normalize before semantic equality; final IR validation counts the Not.
    let left = if native_op == Some(ast::BinOpKind::Implication) {
        Expr::Not(Box::new(left))
    } else {
        left
    };
    Ok(Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(operand(binary.rhs())?),
    })
}

fn lower_bindings(node: &impl HasEntry, depth: usize) -> Result<Vec<Binding>, Diagnostic> {
    let range = syntax(node).text_range();
    let error = |message| {
        Diagnostic::new(
            usize::from(range.start())..usize::from(range.end()),
            message,
        )
    };
    node.entries()
        .map(|entry| match entry {
            ast::Entry::AttrpathValue(binding) => {
                let path = lower_binding_path(binding.attrpath(), depth)?;
                let value = lower(
                    binding
                        .value()
                        .ok_or_else(|| error("missing binding value"))?,
                    depth + path.len(),
                )?;
                Ok(Binding::Assign { path, value })
            }
            ast::Entry::Inherit(inherit) => Ok(Binding::Inherit {
                source: inherit
                    .from()
                    .map(|source| {
                        lower(
                            source
                                .expr()
                                .ok_or_else(|| error("missing inheritance source"))?,
                            depth + 1,
                        )
                    })
                    .transpose()?,
                names: inherit
                    .attrs()
                    .map(|attr| {
                        let key = lower_attr(attr, depth)?;
                        key.literal_name()
                            .map(ToOwned::to_owned)
                            .ok_or_else(|| error("dynamic attributes are not allowed in inherit"))
                    })
                    .collect::<Result<_, _>>()?,
            }),
        })
        .collect()
}

fn lower_string(
    string: ast::Str,
    depth: usize,
    context: StringContext,
) -> Result<Expr, Diagnostic> {
    let range = syntax(&string).text_range();
    let error = |message| {
        Diagnostic::new(
            usize::from(range.start())..usize::from(range.end()),
            message,
        )
    };
    let indented = syntax(&string)
        .first_token()
        .is_some_and(|token| token.text() == "''");
    let mut parts = Vec::new();
    for part in string.parts() {
        parts.push(match part {
            InterpolPart::Literal(text) => StringPart::Literal(text.syntax().text().to_owned()),
            InterpolPart::Interpolation(value) => {
                StringPart::Interpolation(lower_with_string_context(
                    value
                        .expr()
                        .ok_or_else(|| error("missing interpolation expression"))?,
                    depth + 1,
                    if indented {
                        context
                    } else {
                        StringContext::Value
                    },
                )?)
            }
        });
    }
    crate::string::lower(parts, indented, context).map_err(error)
}

// Keep collection machinery out of the recursive lower frame.
fn lower_list(list: ast::List, depth: usize) -> Result<Expr, Diagnostic> {
    Ok(Expr::List(
        list.items()
            .map(|item| {
                // rnix accepts bare lambdas as simple list elements;
                // native Nix requires parentheses around them.
                if matches!(item, ast::Expr::Lambda(_)) {
                    let range = syntax(&item).text_range();
                    return Err(Diagnostic::new(
                        usize::from(range.start())..usize::from(range.end()),
                        "native lambda list elements require parentheses",
                    ));
                }
                lower(item, depth + 1)
            })
            .collect::<Result<_, _>>()?,
    ))
}

fn lower_lookup_path(
    path: Option<ast::Attrpath>,
    depth: usize,
) -> Result<Vec<AttrName>, Diagnostic> {
    path.ok_or_else(|| Diagnostic::new(0..0, "missing native attribute path"))?
        .attrs()
        .map(|attr| lower_attr(attr, depth))
        .collect()
}

fn lower_binding_path(
    path: Option<ast::Attrpath>,
    depth: usize,
) -> Result<Vec<AttrName>, Diagnostic> {
    path.ok_or_else(|| Diagnostic::new(0..0, "missing native attribute path"))?
        .attrs()
        .enumerate()
        .map(|(index, attr)| lower_attr(attr, depth + index))
        .collect()
}

fn lower_attr(attr: ast::Attr, depth: usize) -> Result<AttrName, Diagnostic> {
    let range = syntax(&attr).text_range();
    let error = |message| {
        Diagnostic::new(
            usize::from(range.start())..usize::from(range.end()),
            message,
        )
    };
    match attr {
        ast::Attr::Ident(ident) => {
            let name = syntax(&ident).text().to_string();
            ir::validate_bare_attr_name(&name).map_err(error)?;
            Ok(AttrName::Static(name))
        }
        ast::Attr::Str(string) => {
            if !syntax(&string)
                .first_token()
                .is_some_and(|token| token.text() == "\"")
            {
                return Err(error("attribute names require double quotes"));
            }
            if string
                .parts()
                .any(|part| matches!(part, ast::InterpolPart::Interpolation(_)))
            {
                return Ok(AttrName::Dynamic(Box::new(lower_string(
                    string,
                    depth + 1,
                    StringContext::AttributeKey,
                )?)));
            }
            let text = syntax(&string).text().to_string();
            crate::string::decode(&text[1..text.len() - 1])
                .map(AttrName::Static)
                .map_err(error)
        }
        ast::Attr::Dynamic(key) => Ok(AttrName::Dynamic(Box::new(lower_with_string_context(
            key.expr()
                .ok_or_else(|| error("missing dynamic key expression"))?,
            depth + 1,
            StringContext::AttributeKey,
        )?))),
    }
}
