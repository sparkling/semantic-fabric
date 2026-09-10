//! Only live decoder-owned, base-resolved values confer column-IRI equality.
use super::*;
use crate::iq::iri_cmp::{IriComparison, IriOperand, IriPart};
#[cfg(test)]
mod tests;

/// Copy only total integer IRI constraints below an already authorized D1 copy.
/// VALUES/OPTIONAL otherwise materializes every permitted float for each branch.
/// Keep the original ON predicate and policy barrier; fallible decoders stay out.
pub(super) fn restrict_optional<'a>(
    opt: &'a crate::iq::OptJoin,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> std::borrow::Cow<'a, crate::iq::Scan> {
    use crate::iq::ScanSource;
    let unchanged = std::borrow::Cow::Borrowed(&opt.scan);
    let ScanSource::Projection {
        input,
        columns,
        guards,
        distinct: true,
        ..
    } = &opt.scan.source
    else {
        return unchanged;
    };
    if dialect != Dialect::MySql
        || input.alias != opt.scan.alias
        || input.source.logical().is_none()
        || !guards
            .iter()
            .any(|guard| matches!(guard, SqlCond::NativeCmp(..)))
        || !columns
            .iter()
            .all(|(name, term)| matches!(term, TermMap::Column(raw, _) if raw == name))
    {
        return unchanged;
    }
    let actuals = HashMap::from([(input.alias, scan_actuals(input, dialect, catalog))]);
    let additional: Vec<_> = opt
        .on
        .iter()
        .chain(&opt.extra)
        .filter(|guard| {
            let SqlCond::IriCmp(cmp) = guard else {
                return false;
            };
            let parts = match (&cmp.left, &cmp.right) {
                (IriOperand::Template { parts, base: None }, IriOperand::Constant(_))
                | (IriOperand::Constant(_), IriOperand::Template { parts, base: None }) => parts,
                _ => return false,
            };
            parts.iter().any(|part| matches!(part, IriPart::Column(_)))
                && parts.iter().all(|part| match part {
                    IriPart::Literal(_) => true,
                    IriPart::Column(c) => {
                        c.alias == input.alias
                            && columns.iter().any(|(name, _)| name == &c.column)
                            && scalar_column(c, &actuals) == Some(NativeScalarKey::Integer)
                            && literal_datatype::fact(c, &actuals)
                                == Some(Some(sf_core::datatype::XsdTypeCode::Integer))
                    }
                })
        })
        .cloned()
        .collect();
    if additional.is_empty() {
        return unchanged;
    }
    let mut scan = opt.scan.clone();
    let ScanSource::Projection { guards, .. } = &mut scan.source else {
        unreachable!()
    };
    guards.extend(additional);
    std::borrow::Cow::Owned(scan)
}

pub(super) fn column(
    column: &ColRef,
    base: Option<&str>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    if dialect == Dialect::MySql && base.is_none() && static_iri_column(column, actuals) {
        return Ok(path_comparison::exact_text(
            colref(column, dialect, actuals),
            dialect,
        ));
    }
    let decode = (dialect == Dialect::Sqlite)
        .then(|| lexical_key::column_decode(column, actuals))
        .flatten()
        .ok_or_else(|| {
            Error::Unsupported("resolved column-IRI identity requires a live SQLite decoder".into())
        })?;
    let lexical = lexical_key::expression(colref(column, dialect, actuals), decode, catalog);
    finalize(lexical, base, dialect, catalog, params, pidx)
}

pub(super) fn unreserved_column(column: &ColRef, actuals: &ActualColumns) -> bool {
    actuals.get(&column.alias).is_some_and(|a| {
        let name = resolve_col(&column.column, Some(&a.columns));
        a.iri_unreserved_columns.contains(name)
            && a.text_columns.get(name) == Some(&TextKey::Verbatim)
            && !a.scalar_columns.contains_key(name)
            && a.datatype_columns.get(name) == Some(&Some(sf_core::datatype::XsdTypeCode::String))
    })
}

pub(super) fn encode_text_column(
    column: &ColRef,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
) -> Result<String> {
    let text = path_comparison::rdf_column(column, dialect, catalog, actuals);
    if dialect == Dialect::MySql && unreserved_column(column, actuals) {
        Ok(text)
    } else {
        percent_encode_col(&text, dialect)
    }
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
                        encode_text_column(column, dialect, catalog, actuals)?
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
        (Dialect::MySql, NativeScalarKey::MysqlFloat8) => mysql_float_value::double_lexical(raw),
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

pub(super) fn scalar_template_key(
    key: NativeScalarKey,
    raw: &str,
    dialect: Dialect,
) -> Result<String> {
    let lexical = scalar_lexical(key, raw, dialect)?;
    // These live wire recipes emit restricted ASCII, not arbitrary text. Avoid
    // the generic per-byte SQL encoder only with this exhaustive alphabet proof.
    Ok(match key {
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
        | NativeScalarKey::MysqlFloat8
        | NativeScalarKey::MysqlBinaryBytes
        | NativeScalarKey::MysqlBit
        | NativeScalarKey::MysqlDate
        | NativeScalarKey::MysqlDecimal => lexical,
    })
}

pub(super) fn static_iri_name(column: &str, actuals: &AliasActuals) -> bool {
    actuals
        .static_iri_columns
        .contains(resolve_col(column, Some(&actuals.columns)))
}

pub(super) fn static_iri_column(column: &ColRef, actuals: &ActualColumns) -> bool {
    actuals
        .get(&column.alias)
        .is_some_and(|source| static_iri_name(&column.column, source))
}

/// Mapping normalization has already proved IRI grammar when `base` is absent.
/// This adds decoder proof; arbitrary source text cannot originate this marker.
pub(super) fn qualified_static_template(
    template: &sf_core::ir::Template,
    spec: &sf_core::ir::TermSpec,
    dialect: Dialect,
    actuals: &AliasActuals,
) -> bool {
    dialect == Dialect::MySql
        && spec.term_type == sf_core::ir::TermType::Iri
        && spec.base.is_none()
        && template.segments().iter().all(|segment| match segment {
            sf_core::ir::Segment::Literal(_) => true,
            sf_core::ir::Segment::Column(name) => {
                let resolved = resolve_col(name, Some(&actuals.columns));
                actuals.text_columns.contains_key(resolved)
                    || actuals
                        .scalar_columns
                        .get(resolved)
                        .is_some_and(|key| supports_scalar_template_key(*key, dialect))
            }
        })
}

fn supports_scalar_template_key(key: NativeScalarKey, dialect: Dialect) -> bool {
    matches!(
        (dialect, key),
        (
            Dialect::MySql,
            NativeScalarKey::Integer
                | NativeScalarKey::MysqlFloat4
                | NativeScalarKey::MysqlFloat8
                | NativeScalarKey::MysqlBinaryBytes
                | NativeScalarKey::MysqlBit
                | NativeScalarKey::MysqlDate
                | NativeScalarKey::MysqlDecimal
                | NativeScalarKey::MysqlTimestamp
                | NativeScalarKey::MysqlDateTime
                | NativeScalarKey::MysqlTime
        )
    )
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
