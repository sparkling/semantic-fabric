//! Raw-preserving native numeric identity wrappers.
use super::*;
use crate::iq::scan::LexicalMode;
use mysql_float_value::identity as mysql_float;
pub(super) fn lexical_mode(mode: &LexicalMode) -> bool {
    matches!(mode, LexicalMode::Decoded | LexicalMode::DecodedWithNatural)
}
pub(super) fn native_companion(
    column: &ColRef,
    dialect: Dialect,
    actuals: &ActualColumns,
    modes: &[crate::iq::LexicalKey],
) -> bool {
    if dialect == Dialect::MySql {
        return iri_cmp::scalar_column(column, actuals) == Some(NativeScalarKey::Integer)
            && modes.iter().any(|k| {
                k.column == column.column
                    && (lexical_mode(&k.mode) || k.mode == LexicalMode::Natural)
            });
    }
    if dialect != Dialect::Postgres {
        return false;
    }
    match iri_cmp::scalar_column(column, actuals) {
        Some(
            NativeScalarKey::Integer
            | NativeScalarKey::PostgresBoolean
            | NativeScalarKey::PostgresBytea,
        ) => modes.iter().any(|key| {
            key.column == column.column
                && (lexical_mode(&key.mode) || key.mode == LexicalMode::Natural)
        }),
        Some(NativeScalarKey::PostgresNumeric) => modes
            .iter()
            .any(|key| key.column == column.column && key.mode == LexicalMode::Natural),
        _ => false,
    }
}

pub(super) fn is_numeric(column: &ColRef, dialect: Dialect, actuals: &ActualColumns) -> bool {
    dialect == Dialect::Postgres
        && iri_cmp::scalar_column(column, actuals) == Some(NativeScalarKey::PostgresNumeric)
}
pub(super) fn lexical(raw: &str) -> String {
    // PostgreSQL JSON, unlike JSONB, preserves its validated input text.
    format!("CAST(CAST(CAST({raw} AS TEXT) AS JSON) AS TEXT)")
}
pub(super) fn validates(condition: &SqlCond, actuals: &ActualColumns) -> bool {
    match condition {
        SqlCond::DecodedIsNotNull(column) => is_numeric(column, Dialect::Postgres, actuals),
        SqlCond::IriCmp(cmp) => cmp
            .columns()
            .any(|c| is_numeric(c, Dialect::Postgres, actuals)),
        SqlCond::LiteralCmp(cmp) => cmp
            .columns()
            .any(|c| is_numeric(c, Dialect::Postgres, actuals)),
        SqlCond::Not(inner) => validates(inner, actuals),
        SqlCond::And(cs) | SqlCond::Or(cs) => cs.iter().any(|c| validates(c, actuals)),
        _ => false,
    }
}

pub(super) fn key(
    column: &ColRef,
    dialect: Dialect,
    actuals: &ActualColumns,
    modes: &[crate::iq::LexicalKey],
) -> Option<String> {
    decoder_key(column, dialect, actuals, modes)
        .map(|key| identity(&colref(column, dialect, actuals), key))
}

fn decoder_key(
    column: &ColRef,
    dialect: Dialect,
    actuals: &ActualColumns,
    modes: &[crate::iq::LexicalKey],
) -> Option<NativeScalarKey> {
    if dialect == Dialect::MySql {
        return mysql_float::decoder_key(column, actuals, modes);
    }
    if dialect != Dialect::Postgres {
        return None;
    }
    let key = iri_cmp::scalar_column(column, actuals)?;
    modes
        .iter()
        .any(|mode| {
            mode.column == column.column
                && ((key == NativeScalarKey::PostgresNumeric && lexical_mode(&mode.mode))
                    || (pg_float::is_float(key)
                        && (lexical_mode(&mode.mode) || mode.mode == LexicalMode::Natural)))
        })
        .then_some(key)
}

fn identity(raw: &str, key: NativeScalarKey) -> String {
    if mysql_float::is_float(key) {
        // This helper emits PARTITION BY key lists, never a scalar RDF value.
        format!("{raw}, {}", mysql_float::negative_zero(raw))
    } else if pg_float::is_float(key) {
        pg_float::identity(raw, key)
    } else {
        path_comparison::exact_text(lexical(raw), Dialect::Postgres)
    }
}

pub(super) fn distinct_keys(
    b: &Branch,
    dialect: Dialect,
    actuals: &ActualColumns,
) -> Vec<Option<NativeScalarKey>> {
    if !actuals.values().any(|a| {
        a.scalar_columns.values().any(|k| {
            *k == NativeScalarKey::PostgresNumeric
                || pg_float::is_float(*k)
                || mysql_float::is_float(*k)
        })
    }) {
        return vec![None; b.projection().len()];
    }
    let modes: HashMap<_, _> = actuals
        .keys()
        .map(|alias| {
            (
                *alias,
                literal_roles::resolved(
                    &crate::cascade::distinct_scan::binding_lexical_keys(b, *alias),
                    dialect,
                    &actuals[alias],
                ),
            )
        })
        .collect();
    b.projection()
        .iter()
        .map(|column| {
            modes
                .get(&column.alias)
                .and_then(|keys| decoder_key(column, dialect, actuals, keys))
        })
        .collect()
}

/// Already-compatible pooled arms still need decoded numeric keys across arms.
/// No arm may lend a pre-coercion key recipe to a different native result type.
pub(super) fn union_keys(
    branches: &[Branch],
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<Option<Vec<Option<NativeScalarKey>>>> {
    if dialect != Dialect::Postgres {
        return Ok(None);
    }
    let keys: Vec<_> = branches
        .iter()
        .map(|b| distinct_keys(b, dialect, &branch_actuals(b, dialect, catalog)))
        .collect();
    if !keys.iter().flatten().any(Option::is_some) {
        return Ok(None);
    }
    if branches.iter().any(|b| b.agg.is_some() || b.path.is_some())
        || keys.iter().any(|key| key != &keys[0])
    {
        return Err(Error::Unsupported(
            "native UNION requires agreeing raw decoder and output consumer roles".into(),
        ));
    }
    Ok(keys.into_iter().next())
}

pub(super) fn distinct_sql(raw: String, keys: &[Option<NativeScalarKey>]) -> String {
    let columns = (0..keys.len())
        .map(|i| format!("__sf_numeric_raw.c{i}"))
        .collect::<Vec<_>>();
    let partition = columns
        .iter()
        .zip(keys)
        .map(|(column, key)| key.map_or_else(|| column.clone(), |key| identity(column, key)))
        .collect::<Vec<_>>()
        .join(", ");
    let output = (0..keys.len())
        .map(|i| format!("__sf_numeric_distinct.c{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("SELECT {output} FROM (SELECT {}, ROW_NUMBER() OVER (PARTITION BY {partition}) AS __sf_rank FROM ({raw}) __sf_numeric_raw) __sf_numeric_distinct WHERE __sf_numeric_distinct.__sf_rank = 1", columns.join(", "))
}

pub(super) fn order(b: &Branch, projection: &[ColRef], dialect: Dialect) -> Result<Option<String>> {
    let mut keys = Vec::new();
    for key in &b.order {
        let column = b
            .bindings
            .get(&key.var)
            .and_then(order_column)
            .ok_or_else(|| {
                Error::Unsupported(format!(
                    "ORDER BY ?{} is not a bound rr:column term",
                    key.var
                ))
            })?;
        let index = projection
            .iter()
            .position(|c| c == &column)
            .ok_or_else(|| {
                Error::Unsupported("ORDER BY column missing from DISTINCT output".into())
            })?;
        keys.push(format!(
            "__sf_numeric_distinct.c{index} {}",
            if dialect == Dialect::MySql && key.descending {
                "DESC"
            } else if dialect == Dialect::MySql {
                "ASC"
            } else if key.descending {
                "DESC NULLS LAST"
            } else {
                "ASC NULLS FIRST"
            }
        ));
    }
    Ok((!keys.is_empty()).then(|| format!(" ORDER BY {}", keys.join(", "))))
}

#[cfg(test)]
#[path = "pg_numeric_tests.rs"]
mod tests;
