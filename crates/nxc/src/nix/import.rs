// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use crate::{
    Diagnostic, MAX_DEPTH, MAX_SOURCE_BYTES, MAX_TOKENS,
    ir::{self, BinaryOp, Binding, Expr, Formal, Pattern, StringPart},
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
    root: ast::Root,
    source_len: usize,
}

impl Parsed {
    /// Adapt supported native forms to the semantic IR without reparsing.
    pub fn lower(&self) -> Result<Expr, Vec<Diagnostic>> {
        let root = self.root.expr().ok_or_else(|| {
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
    if !parsed.errors().is_empty() {
        return Err(parsed
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
            .collect());
    }
    Ok(Parsed {
        root: parsed.tree(),
        source_len: source.len(),
    })
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
    match node {
        ast::Expr::Ident(ident) => {
            let name = syntax(&ident).text().to_string();
            ir::validate_name(&name).map_err(error)?;
            Ok(Expr::Variable(name))
        }
        ast::Expr::Literal(literal) => {
            let token = syntax(&literal)
                .first_token()
                .ok_or_else(|| error("missing literal"))?;
            if token.kind() != K::TOKEN_INTEGER {
                return Err(error("only integer literals are supported yet"));
            }
            let value = token
                .text()
                .parse::<i64>()
                .map_err(|_| error("integer literal exceeds the Nix signed 64-bit range"))?;
            Ok(Expr::Integer(value as u64))
        }
        ast::Expr::Paren(paren) => lower(
            paren
                .expr()
                .ok_or_else(|| error("missing parenthesized expression"))?,
            depth,
        ),
        ast::Expr::Apply(apply) => Ok(Expr::Apply {
            function: Box::new(child(apply.lambda())?),
            argument: Box::new(child(apply.argument())?),
        }),
        ast::Expr::Str(string) => {
            if !syntax(&string)
                .first_token()
                .is_some_and(|token| token.text() == "\"")
            {
                return Err(error("only double-quoted strings are supported yet"));
            }
            let mut parts = Vec::new();
            for part in string.parts() {
                match part {
                    InterpolPart::Literal(text) => ir::push_string_literal(
                        &mut parts,
                        crate::string::decode(text.syntax().text()).map_err(error)?,
                    ),
                    InterpolPart::Interpolation(value) => {
                        parts.push(StringPart::Interpolation(child(value.expr())?));
                    }
                }
            }
            Ok(Expr::String(parts))
        }
        ast::Expr::AttrSet(set) => Ok(Expr::AttrSet {
            recursive: set.rec_token().is_some(),
            bindings: set
                .entries()
                .map(|entry| match entry {
                    ast::Entry::AttrpathValue(binding) => {
                        let path = lower_path(binding.attrpath())?;
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
                            .map(|source| child(source.expr()))
                            .transpose()?,
                        names: inherit.attrs().map(lower_attr).collect::<Result<_, _>>()?,
                    }),
                })
                .collect::<Result<_, Diagnostic>>()?,
        }),
        ast::Expr::Select(select) => Ok(Expr::Select {
            value: Box::new(child(select.expr())?),
            path: lower_path(select.attrpath())?,
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
            Ok(Expr::Negate(Box::new(child(unary.expr())?)))
        }
        ast::Expr::BinOp(binary) => {
            let op = match binary.operator() {
                Some(ast::BinOpKind::Add) => BinaryOp::Add,
                Some(ast::BinOpKind::Sub) => BinaryOp::Subtract,
                Some(ast::BinOpKind::Mul) => BinaryOp::Multiply,
                Some(ast::BinOpKind::Div) => BinaryOp::Divide,
                _ => return Err(error("native Nix operator is not supported yet")),
            };
            Ok(Expr::Binary {
                op,
                left: Box::new(child(binary.lhs())?),
                right: Box::new(child(binary.rhs())?),
            })
        }
        other => Err(error(&format!(
            "native Nix form {:?} is not supported yet",
            syntax(&other).kind()
        ))),
    }
}

fn lower_attr(attr: ast::Attr) -> Result<String, Diagnostic> {
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
            ir::validate_attr_name(&name).map_err(error)?;
            Ok(name)
        }
        _ => Err(error("quoted and dynamic attributes are not supported yet")),
    }
}

fn lower_path(path: Option<ast::Attrpath>) -> Result<Vec<String>, Diagnostic> {
    path.ok_or_else(|| Diagnostic::new(0..0, "missing native attribute path"))?
        .attrs()
        .map(lower_attr)
        .collect()
}
