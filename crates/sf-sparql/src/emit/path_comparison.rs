//! Comparison decoration preserves native values and outer RDF reconstruction.
//! Varying text needs byte-exact, NO PAD comparison. Other native type families
//! require separate decoder-equivalence work; a text cast is not that proof.
use super::*;
#[cfg(test)]
mod tests;
#[cfg(test)]
thread_local! { static METADATA_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

pub(super) fn exact_text(expression: String, dialect: Dialect) -> String {
    match dialect {
        Dialect::Sqlite => format!("({expression} COLLATE BINARY)"),
        Dialect::Postgres => format!("({expression} COLLATE \"C\")"),
        // Unlike utf8mb4_bin, the MySQL8 collation is NO PAD. Keep a text
        // result: projecting BINARY would change the driver decoder to hex.
        Dialect::MySql => format!("(CONVERT({expression} USING utf8mb4) COLLATE utf8mb4_0900_bin)"),
        _ => expression,
    }
}

fn source_text(source: &LogicalSource, column: &str, catalog: &ColumnCatalog) -> Option<TextKey> {
    let name = resolve_col(column, catalog.columns(source));
    catalog
        .text_by_source
        .get(&source_key(source))
        .and_then(|columns| columns.get(name))
        .copied()
}

#[cfg(test)]
pub(super) fn hop_text(hop: &HopExpr, catalog: &ColumnCatalog) -> (bool, bool) {
    match hop {
        HopExpr::Pred(rel) => (
            source_text(&rel.source, &rel.subj_col, catalog).is_some(),
            source_text(&rel.source, &rel.obj_col, catalog).is_some(),
        ),
        HopExpr::Inverse(inner) => {
            let (s, o) = hop_text(inner, catalog);
            (o, s)
        }
        HopExpr::Seq(a, b) => (hop_text(a, catalog).0, hop_text(b, catalog).1),
        HopExpr::Alt(parts) | HopExpr::Nps(parts) => {
            parts
                .iter()
                .fold((!parts.is_empty(), !parts.is_empty()), |(s, o), part| {
                    let (ps, po) = hop_text(part, catalog);
                    (s && ps, o && po)
                })
        }
    }
}

pub(super) fn path_actuals_controlled(
    path: &PathClosure,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<AliasActuals> {
    super::metadata_path::actuals(path, catalog, work)
}

#[cfg(test)]
fn condition_has_path(cond: &SqlCond) -> bool {
    match cond {
        SqlCond::PathExists { .. } => true,
        SqlCond::Exists { scans, conds } | SqlCond::NotExists { scans, conds } => {
            scans
                .iter()
                .any(|s| matches!(s.source, crate::iq::ScanSource::Path { .. }))
                || conds.iter().any(condition_has_path)
        }
        SqlCond::And(cs) | SqlCond::Or(cs) => cs.iter().any(condition_has_path),
        SqlCond::Not(c) => condition_has_path(c),
        _ => false,
    }
}

#[cfg(test)]
pub(super) fn branch_has_path(branch: &Branch) -> bool {
    branch.path.is_some()
        || branch
            .core
            .iter()
            .any(|s| matches!(s.source, crate::iq::ScanSource::Path { .. }))
        || branch.opts.iter().any(|j| {
            matches!(j.scan.source, crate::iq::ScanSource::Path { .. })
                || j.on.iter().chain(&j.extra).any(condition_has_path)
        })
        || branch.where_conds.iter().any(condition_has_path)
        || branch.subplan_joins.iter().any(|s| {
            s.plan.branches.iter().any(branch_has_path) || s.on.iter().any(condition_has_path)
        })
}

pub(super) fn column_text(column: &ColRef, actuals: &ActualColumns) -> Option<TextKey> {
    actuals.get(&column.alias).and_then(|a| {
        a.text_columns
            .get(resolve_col(&column.column, Some(&a.columns)))
            .copied()
    })
}

/// Normalize only live-proven decoder families, before SQL joins/deduplication.
/// Keep this expression in the prepare-only twin: its result is already text,
/// not an original CHAR column that should be padded again after a UNION.
fn decoded_text(
    expression: String,
    key: Option<TextKey>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> String {
    match (dialect, key) {
        (Dialect::Sqlite, Some(TextKey::SqliteCharacter(width))) => {
            catalog
                .character_keys
                .store(true, std::sync::atomic::Ordering::Relaxed);
            format!("__sf_character_key_v1({expression}, {width})")
        }
        (Dialect::Postgres, Some(TextKey::PostgresCharacter)) => {
            // Keep the function nested separately from COLLATE: sqlparser 0.62
            // otherwise backtracks a qualified function into a field reference.
            format!("(pg_catalog.convert_from(pg_catalog.bpcharsend({expression}), 'UTF8'))")
        }
        _ => expression,
    }
}

/// RDF comparison key, without converting unrelated native type families.
pub(super) fn rdf_column(
    column: &ColRef,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
) -> String {
    if dialect == Dialect::Sqlite && column_text(column, actuals).is_none() {
        if let Some(decode) = lexical_key::proven_column(column, actuals) {
            return lexical_key::expression(colref(column, dialect, actuals), decode, catalog);
        }
    }
    rdf_text_column(column, dialect, catalog, actuals)
}

/// Legacy value predicates may use decoded text, but cannot borrow a D1/RDF
/// lexical identity proof for a numeric or mixed-storage column.
pub(super) fn rdf_text_column(
    column: &ColRef,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
) -> String {
    let key = column_text(column, actuals);
    let expression = decoded_text(colref(column, dialect, actuals), key, dialect, catalog);
    if !catalog.suppress_path_collation && key.is_some() {
        exact_text(expression, dialect)
    } else {
        expression
    }
}

#[cfg(test)]
pub(super) fn subplan_actuals(
    plan: &crate::Plan,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> AliasActuals {
    super::metadata::subplan_actuals_controlled(
        plan,
        dialect,
        catalog,
        sf_sql::source_work::SourceWork::new(None),
    )
    .expect("uncontrolled subplan metadata")
}

#[cfg(test)]
pub(super) fn metadata_visit() {
    METADATA_VISITS.with(|visits| visits.set(visits.get() + 1));
}

pub(super) fn render_key_equality(
    a: &ColRef,
    b: &ColRef,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
) -> Result<String> {
    if let Some(sql) = mysql_float_value::identity::key_equality(a, b, dialect, actuals)? {
        return Ok(sql);
    }
    let comparison_decode = |column: &ColRef| {
        let source = actuals.get(&column.alias)?;
        source
            .lexical_comparison_columns
            .get(resolve_col(&column.column, Some(&source.columns)))
            .copied()
    };
    if dialect == Dialect::Sqlite
        && [a, b].iter().all(|column| {
            comparison_decode(column).is_some() || column_text(column, actuals).is_some()
        })
    {
        let comparison = |column: &ColRef| {
            comparison_decode(column)
                .map(|decode| {
                    lexical_key::expression(colref(column, dialect, actuals), decode, catalog)
                })
                .unwrap_or_else(|| rdf_text_column(column, dialect, catalog, actuals))
        };
        return Ok(format!("{} = {}", comparison(a), comparison(b)));
    }
    let (mut left, mut right) = (colref(a, dialect, actuals), colref(b, dialect, actuals));
    let path = [a, b]
        .iter()
        .any(|c| actuals.get(&c.alias).is_some_and(|a| a.path));
    if path || (column_text(a, actuals).is_some() && column_text(b, actuals).is_some()) {
        let (a_key, b_key) = (column_text(a, actuals), column_text(b, actuals));
        left = decoded_text(left, a_key, dialect, catalog);
        right = decoded_text(right, b_key, dialect, catalog);
        if !catalog.suppress_path_collation
            && (dialect == Dialect::Sqlite || (a_key.is_some() && b_key.is_some()))
        {
            left = exact_text(left, dialect);
            right = exact_text(right, dialect);
        }
    }
    Ok(format!("{left} = {right}"))
}
