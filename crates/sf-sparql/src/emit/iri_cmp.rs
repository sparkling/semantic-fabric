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
    if base.is_none() && matches!(dialect, Dialect::Postgres | Dialect::MySql) {
        let mut expressions = Vec::with_capacity(parts.len());
        for part in parts {
            expressions.push(match part {
                IriPart::Literal(text) => {
                    params.push(text.to_string());
                    *pidx += 1;
                    dialect.placeholder(*pidx)
                }
                IriPart::Column(column) => {
                    let lexical = if path_comparison::column_text(column, actuals).is_some() {
                        path_comparison::rdf_column(column, dialect, catalog, actuals)
                    } else if integer_column(column, actuals) {
                        let raw = colref(column, dialect, actuals);
                        if dialect == Dialect::Postgres {
                            format!("CAST({raw} AS TEXT)")
                        } else {
                            // YEAR zero and ZEROFILL display strings differ
                            // from the integer wire decoder. Decimal(20,0)
                            // normalizes both without narrowing unsigned u64.
                            format!("CAST(CAST({raw} AS DECIMAL(20, 0)) AS CHAR)")
                        }
                    } else {
                        return Err(Error::Unsupported(
                            "static template identity requires a live native text or integer decoder".into(),
                        ));
                    };
                    percent_encode_col(&lexical, dialect)?
                }
            });
        }
        let lexical = if expressions.is_empty() {
            "''".into()
        } else if dialect == Dialect::MySql {
            format!("CONCAT({})", expressions.join(", "))
        } else {
            format!("({})", expressions.join(" || "))
        };
        return Ok(path_comparison::exact_text(lexical, dialect));
    }
    finalize(
        template_lexical(parts, dialect, catalog, actuals)?,
        base,
        dialect,
        catalog,
        params,
        pidx,
    )
}

pub(super) fn integer_column(column: &ColRef, actuals: &ActualColumns) -> bool {
    actuals.get(&column.alias).is_some_and(|source| {
        source
            .integer_columns
            .contains(resolve_col(&column.column, Some(&source.columns)))
    })
}

pub(super) fn projected_integers(
    columns: &[(Box<str>, TermMap)],
    inner: &AliasActuals,
) -> HashSet<String> {
    columns
        .iter()
        .filter_map(|(name, term)| match term {
            TermMap::Column(raw, _)
                if inner
                    .integer_columns
                    .contains(resolve_col(raw, Some(&inner.columns))) =>
            {
                Some(name.to_string())
            }
            _ => None,
        })
        .collect()
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
