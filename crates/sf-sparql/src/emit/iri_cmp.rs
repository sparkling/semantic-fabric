//! Only live decoder-owned, base-resolved values confer column-IRI equality.
use super::*;
use crate::iq::iri_cmp::{IriComparison, IriOperand, IriPart};
#[cfg(test)]
mod tests;

pub(super) fn column(
    column: &ColRef,
    base: Option<&str>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    let decode = (dialect == Dialect::Sqlite)
        .then(|| lexical_key::column_decode(column, actuals))
        .flatten()
        .ok_or_else(|| {
            Error::Unsupported("resolved column-IRI identity requires a live SQLite decoder".into())
        })?;
    let lexical = lexical_key::expression(colref(column, dialect, actuals), decode, catalog);
    finalize(lexical, base, dialect, catalog, params, pidx)
}

fn finalize(
    lexical: String,
    base: Option<&str>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    catalog
        .lexical_keys
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let base = match base {
        Some(base) => {
            params.push(base.to_owned());
            *pidx += 1;
            dialect.placeholder(*pidx)
        }
        None => "NULL".into(),
    };
    let value = format!("__sf_iri_key_v1({lexical}, {base})");
    Ok(if catalog.suppress_path_collation {
        value
    } else {
        path_comparison::exact_text(value, dialect)
    })
}

pub(super) fn template(
    parts: &[IriPart],
    base: Option<&str>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    finalize(
        template_lexical(parts, dialect, catalog, actuals)?,
        base,
        dialect,
        catalog,
        params,
        pidx,
    )
}

pub(super) fn template_lexical(
    parts: &[IriPart],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
) -> Result<String> {
    if dialect != Dialect::Sqlite {
        return Err(Error::Unsupported(
            "resolved template-IRI identity requires a live SQLite decoder".into(),
        ));
    }
    let mut expressions = Vec::with_capacity(parts.len());
    for part in parts {
        expressions.push(match part {
            IriPart::Literal(text) => sql_string_literal(text),
            IriPart::Column(column) => {
                let decode = lexical_key::column_decode(column, actuals).ok_or_else(|| {
                    Error::Unsupported(
                        "resolved template-IRI identity requires every live SQLite decoder".into(),
                    )
                })?;
                let lexical =
                    lexical_key::expression(colref(column, dialect, actuals), decode, catalog);
                percent_encode_col(&lexical, dialect)?
            }
        });
    }
    Ok(if expressions.is_empty() {
        "''".into()
    } else {
        format!("({})", expressions.join(" || "))
    })
}

pub(super) fn render(
    cmp: &IriComparison,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    let mut operand = |operand: &IriOperand| match operand {
        IriOperand::Column { column: col, base } => column(
            col,
            base.as_deref(),
            dialect,
            catalog,
            actuals,
            params,
            pidx,
        ),
        IriOperand::Template { parts, base } => template(
            parts,
            base.as_deref(),
            dialect,
            catalog,
            actuals,
            params,
            pidx,
        ),
        IriOperand::Constant(iri) => {
            params.push(iri.as_str().to_owned());
            *pidx += 1;
            Ok(path_comparison::exact_text(
                dialect.placeholder(*pidx),
                dialect,
            ))
        }
    };
    Ok(format!(
        "({} = {})",
        operand(&cmp.left)?,
        operand(&cmp.right)?
    ))
}
