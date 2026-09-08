// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use super::{Expr, StringPart, validate_absolute_path, validate_home_path, validate_relative_path};

pub(crate) fn lower_path(mut parts: Vec<StringPart>) -> Result<Expr, &'static str> {
    if let [StringPart::Literal(text)] = parts.as_slice() {
        validate_literal(text)?;
        let StringPart::Literal(text) = parts.pop().unwrap() else {
            unreachable!()
        };
        return Ok(if text.starts_with("~/") {
            Expr::HomePath(text)
        } else if text.starts_with('/') {
            Expr::AbsolutePath(text)
        } else {
            Expr::RelativePath(text)
        });
    }
    validate_interpolated_path(&parts)?;
    Ok(Expr::InterpolatedPath(parts))
}

fn validate_literal(path: &str) -> Result<(), &'static str> {
    if path.starts_with("~/") {
        validate_home_path(path)
    } else if path.starts_with('/') {
        validate_absolute_path(path)
    } else {
        validate_relative_path(path)
    }
}

pub(super) fn validate_interpolated_path(parts: &[StringPart]) -> Result<(), &'static str> {
    let Some(StringPart::Literal(prefix)) = parts.first() else {
        return Err("interpolated paths must begin with a literal path prefix");
    };
    if !prefix.contains('/') {
        return Err("a path requires a slash before its first interpolation");
    }
    // An interpolation supplies a component syntactically, even when its value
    // will be empty or invalid. Validate spelling without evaluating that value.
    // Callers bound fragment counts and bytes before constructing this spelling.
    let mut spelling = String::new();
    let mut interpolated = false;
    let mut previous_literal = false;
    for part in parts {
        match part {
            StringPart::Literal(text) => {
                if text.is_empty() || previous_literal {
                    return Err("path literals must be nonempty and nonadjacent");
                }
                spelling.push_str(text);
                previous_literal = true;
            }
            StringPart::Interpolation(_) => {
                spelling.push('x');
                interpolated = true;
                previous_literal = false;
            }
        }
    }
    if !interpolated {
        return Err("interpolated paths require an interpolation");
    }
    validate_literal(&spelling)
}
