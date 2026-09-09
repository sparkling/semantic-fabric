//! Only live decoder-owned, base-resolved values confer column-IRI equality.
use super::*;
use crate::iq::iri_cmp::{IriComparison, IriOperand};
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
