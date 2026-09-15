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

pub(super) fn decoder_key(
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

/// Admit the complete borrowed decoder proof before invoking its raw oracle.
/// Full lookup envelopes keep accounting independent of hash iteration order.
pub(super) fn decoder_key_controlled(
    column: &ColRef,
    dialect: Dialect,
    actuals: &ActualColumns,
    modes: &[crate::iq::LexicalKey],
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Option<NativeScalarKey>> {
    use source_control::validation_error as error;
    work.checkpoint().map_err(error)?;
    if !matches!(dialect, Dialect::Postgres | Dialect::MySql) {
        return Ok(None);
    }
    // Include this admission pass's lookup as well as the raw proof. MySQL
    // additionally resolves the datatype fact, for three lookups in total.
    let lookups = if dialect == Dialect::MySql { 3 } else { 2 };
    work.charge(lookups).map_err(error)?;
    work.product(lookups, actuals.len()).map_err(error)?;
    if let Some(source) = actuals.get(&column.alias) {
        for name in &source.columns {
            work.charge(1 + 2 * lookups).map_err(error)?;
            work.product(2 * lookups, name.len().min(column.column.len()))
                .map_err(error)?;
        }
        let name = resolve_col(&column.column, Some(&source.columns));
        work.charge(name.len()).map_err(error)?;
        for candidate in source.scalar_columns.keys() {
            work.charge(2).map_err(error)?;
            work.product(2, candidate.len().min(name.len()))
                .map_err(error)?;
        }
        if dialect == Dialect::MySql {
            work.charge(name.len()).map_err(error)?;
            for candidate in source.datatype_columns.keys() {
                work.charge(2).map_err(error)?;
                work.product(2, candidate.len().min(name.len()))
                    .map_err(error)?;
            }
        }
    }
    for mode in modes {
        work.charge(2).map_err(error)?;
        work.product(2, mode.column.len().min(column.column.len()))
            .map_err(error)?;
    }
    let result = decoder_key(column, dialect, actuals, modes);
    work.checkpoint().map_err(error)?;
    Ok(result)
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

#[cfg(test)]
pub(super) fn distinct_keys(
    b: &Branch,
    dialect: Dialect,
    actuals: &ActualColumns,
) -> Vec<Option<NativeScalarKey>> {
    distinct_keys_for_projection(
        &BindingView::Direct(&b.bindings),
        dialect,
        actuals,
        &b.projection(),
        sf_sql::source_work::SourceWork::new(None),
    )
    .expect("uncontrolled native identity inventory")
}

pub(super) fn distinct_keys_for_projection(
    bindings: &BindingView<'_>,
    dialect: Dialect,
    actuals: &ActualColumns,
    projection: &[ColRef],
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Vec<Option<NativeScalarKey>>> {
    work.checkpoint()
        .map_err(source_control::validation_error)?;
    let mut numeric = false;
    for source in actuals.values() {
        work.charge(1).map_err(source_control::validation_error)?;
        for k in source.scalar_columns.values() {
            work.charge(1).map_err(source_control::validation_error)?;
            numeric |= *k == NativeScalarKey::PostgresNumeric
                || pg_float::is_float(*k)
                || mysql_float::is_float(*k);
        }
    }
    let mut output = work
        .vector(projection.len())
        .map_err(source_control::validation_error)?;
    if !numeric {
        output.resize(projection.len(), None);
        work.checkpoint()
            .map_err(source_control::validation_error)?;
        return Ok(output);
    }
    let modes = literal_roles::binding_modes(bindings, dialect, actuals, work)?;
    for column in projection {
        work.charge(1).map_err(source_control::validation_error)?;
        let keys = literal_roles::modes_for_alias(&modes, column.alias, work)?;
        output.push(match keys {
            Some(keys) => decoder_key_controlled(column, dialect, actuals, keys, work)?,
            None => None,
        });
    }
    work.checkpoint()
        .map_err(source_control::validation_error)?;
    Ok(output)
}

/// Already-compatible pooled arms still need decoded numeric keys across arms.
/// No arm may lend a pre-coercion key recipe to a different native result type.
pub(super) fn union_keys(
    branches: &[Branch],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Option<Vec<Option<NativeScalarKey>>>> {
    if dialect != Dialect::Postgres {
        return Ok(None);
    }
    let mut keys = Vec::new();
    for b in branches {
        let actuals = branch_actuals_controlled(b, dialect, catalog, work)?;
        keys.push(distinct_keys_for_projection(
            &BindingView::Direct(&b.bindings),
            dialect,
            &actuals,
            &b.projection(),
            work,
        )?);
    }
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

pub(super) fn order(
    b: &Branch,
    bindings: &BindingView<'_>,
    projection: &[ColRef],
    dialect: Dialect,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Option<String>> {
    let mut keys = Vec::new();
    for key in &b.order {
        let column = bindings
            .get(&key.var, work)?
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
#[test]
fn decoder_admission_funds_inventory_and_raw_proof_passes() {
    use sf_core::datatype::XsdTypeCode;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    use sf_sql::source_work::SourceWork;
    let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
    for (dialect, scalar, units) in [
        (Dialect::Postgres, NativeScalarKey::PostgresFloat4, 106),
        (Dialect::Postgres, NativeScalarKey::PostgresFloat8, 106),
        (Dialect::MySql, NativeScalarKey::MysqlFloat4, 159),
        (Dialect::MySql, NativeScalarKey::MysqlFloat8, 159),
    ] {
        for names in [["src", "aaa", "zzz"], ["zzz", "aaa", "src"]] {
            let mut source = source_actuals(
                &LogicalSource::Table("items".into()),
                &ColumnCatalog::default(),
            );
            source.columns = names.iter().map(|name| name.to_string()).collect();
            for name in names {
                source.scalar_columns.insert(name.into(), scalar);
                source
                    .datatype_columns
                    .insert(name.into(), Some(XsdTypeCode::Double));
            }
            let actuals = HashMap::from([(0, source)]);
            for last in ["src", "zzz"] {
                let modes = ["aaa", "bbb", last].map(|name| crate::iq::LexicalKey {
                    column: name.into(),
                    mode: LexicalMode::Decoded,
                });
                let column = ColRef::new(0, "src");
                let run = |control: &QueryBudget| {
                    decoder_key_controlled(
                        &column,
                        dialect,
                        &actuals,
                        &modes,
                        SourceWork::new(Some(control)),
                    )
                };
                // One alias; three 3-byte descriptors; three 3-byte modes.
                // PG: alias 4 + resolve 51 + hash 3 + scalar 24 + modes 24.
                // MySQL: alias 6 + resolve 75 + two*(hash 3 + map 24) + modes 24.
                let exact = budget(units);
                assert_eq!(run(&exact).unwrap(), (last == "src").then_some(scalar));
                assert_eq!(exact.consumed(QueryCharge::SourceWork), units);
                assert!(matches!(
                    run(&budget(units - 1)),
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
            }
        }
    }
}

#[cfg(test)]
#[path = "pg_numeric_tests.rs"]
mod tests;
