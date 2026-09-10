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
                    if path_comparison::column_text(column, actuals).is_some() {
                        percent_encode_col(
                            &path_comparison::rdf_column(column, dialect, catalog, actuals),
                            dialect,
                        )?
                    } else if let Some(key) = scalar_column(column, actuals) {
                        scalar_template_key(key, &colref(column, dialect, actuals), dialect)?
                    } else {
                        return Err(Error::Unsupported(
                            "static template identity requires a proven live native decoder".into(),
                        ));
                    }
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

pub(super) fn scalar_lexical(key: NativeScalarKey, raw: &str, dialect: Dialect) -> Result<String> {
    Ok(match (dialect, key) {
        (Dialect::MySql, NativeScalarKey::MysqlFloat4) => mysql_float_value::float_lexical(raw),
        (_, NativeScalarKey::MysqlFloat8) => {
            return Err(Error::Unsupported(
                "MySQL DOUBLE value authority is not a lexical recipe".into(),
            ))
        }
        (Dialect::Postgres, NativeScalarKey::Integer) => format!("CAST({raw} AS TEXT)"),
        (Dialect::Postgres, NativeScalarKey::PostgresFloat4 | NativeScalarKey::PostgresFloat8) => {
            pg_float::lexical(raw, key, false)
        }
        (Dialect::Postgres, NativeScalarKey::PostgresNumeric) => {
            // JSON validates finite number syntax while retaining the input
            // text exactly. JSONB/to_json would normalize or quote it instead.
            pg_numeric::lexical(raw)
        }
        // YEAR zero and ZEROFILL displays differ from integer wire decoding.
        // Decimal(20,0) normalizes both without narrowing unsigned u64.
        (Dialect::MySql, NativeScalarKey::Integer) => {
            format!("CAST(CAST({raw} AS DECIMAL(20, 0)) AS CHAR)")
        }
        (Dialect::Postgres, NativeScalarKey::PostgresBoolean) => {
            format!("(CASE WHEN {raw} IS NULL THEN NULL WHEN {raw} THEN 'true' ELSE 'false' END)")
        }
        (Dialect::Postgres, NativeScalarKey::PostgresBytea) => {
            format!("pg_catalog.translate(pg_catalog.encode({raw}, 'hex'), 'abcdef', 'ABCDEF')")
        }
        (Dialect::MySql, NativeScalarKey::MysqlBinaryBytes) => format!("HEX({raw})"),
        // BIT's numeric HEX loses leading zero bytes. Its binary string is
        // exactly the width-preserving byte vector received by the wire decoder.
        (Dialect::MySql, NativeScalarKey::MysqlBit) => format!("HEX(CAST({raw} AS BINARY))"),
        (Dialect::MySql, NativeScalarKey::MysqlDecimal | NativeScalarKey::MysqlDate) => {
            // Keep the wire formatter's scale/ZEROFILL. Use CONVERT for the
            // charset: sqlparser does not admit CAST's CHARACTER SET suffix.
            format!("CONVERT(CAST({raw} AS CHAR) USING utf8mb4)")
        }
        (
            Dialect::MySql,
            NativeScalarKey::MysqlTimestamp
            | NativeScalarKey::MysqlDateTime
            | NativeScalarKey::MysqlTime,
        ) => {
            let text = format!("CONVERT(CAST({raw} AS CHAR) USING utf8mb4)");
            let whole = format!("SUBSTRING_INDEX({text}, '.', 1)");
            let fraction = format!("SUBSTRING_INDEX({text}, '.', -1)");
            let no_fraction =
                format!("(LOCATE('.', {text}) = 0 OR REPLACE({fraction}, '0', '') = '')");
            let whole = if matches!(
                key,
                NativeScalarKey::MysqlTimestamp | NativeScalarKey::MysqlDateTime
            ) {
                format!("REPLACE({whole}, ' ', 'T')")
            } else {
                whole
            };
            // Preserve zero timestamps, session-local timestamp fields,
            // signed total hours, and the wire decoder's exact six-digit micros.
            let zero = if key == NativeScalarKey::MysqlTime {
                format!("WHEN {no_fraction} AND {whole} = '-00:00:00' THEN '00:00:00' ")
            } else {
                String::new()
            };
            format!("(CASE WHEN {raw} IS NULL THEN NULL {zero}WHEN {no_fraction} THEN {whole} ELSE CONCAT({whole}, '.', RPAD({fraction}, 6, '0')) END)")
        }
        _ => {
            return Err(Error::Unsupported(
                "native decoder recipe belongs to another backend".into(),
            ))
        }
    })
}

fn scalar_template_key(key: NativeScalarKey, raw: &str, dialect: Dialect) -> Result<String> {
    let lexical = scalar_lexical(key, raw, dialect)?;
    // These live wire recipes emit restricted ASCII, not arbitrary text. Avoid
    // the generic per-byte SQL encoder only with this exhaustive alphabet proof.
    Ok(match key {
        NativeScalarKey::MysqlFloat8 => {
            return Err(Error::Unsupported(
                "MySQL DOUBLE value authority is not a template key".into(),
            ))
        }
        NativeScalarKey::MysqlTimestamp
        | NativeScalarKey::MysqlDateTime
        | NativeScalarKey::MysqlTime => {
            format!("REPLACE({lexical}, ':', '%3A')")
        }
        NativeScalarKey::Integer
        | NativeScalarKey::PostgresBoolean
        | NativeScalarKey::PostgresBytea
        | NativeScalarKey::PostgresNumeric
        | NativeScalarKey::PostgresFloat4
        | NativeScalarKey::PostgresFloat8
        | NativeScalarKey::MysqlFloat4
        | NativeScalarKey::MysqlBinaryBytes
        | NativeScalarKey::MysqlBit
        | NativeScalarKey::MysqlDate
        | NativeScalarKey::MysqlDecimal => lexical,
    })
}

pub(super) fn scalar_column(column: &ColRef, actuals: &ActualColumns) -> Option<NativeScalarKey> {
    actuals.get(&column.alias).and_then(|source| {
        source
            .scalar_columns
            .get(resolve_col(&column.column, Some(&source.columns)))
            .copied()
    })
}

pub(super) fn projected_scalars(
    columns: &[(Box<str>, TermMap)],
    inner: &AliasActuals,
) -> HashMap<String, NativeScalarKey> {
    columns
        .iter()
        .filter_map(|(name, term)| match term {
            TermMap::Column(raw, _) => inner
                .scalar_columns
                .get(resolve_col(raw, Some(&inner.columns)))
                .filter(|key| {
                    !matches!(
                        key,
                        NativeScalarKey::MysqlDate | NativeScalarKey::MysqlDateTime
                    )
                })
                .map(|key| (name.to_string(), *key)),
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
