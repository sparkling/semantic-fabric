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

pub(super) fn path_key_expression(
    expression: String,
    source: &LogicalSource,
    column: &str,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> String {
    let key = source_text(source, column, catalog);
    let expression = decoded_text(expression, key, dialect, catalog);
    if !catalog.suppress_path_collation && (dialect == Dialect::Sqlite || key.is_some()) {
        exact_text(expression, dialect)
    } else {
        expression
    }
}

fn hop_text(hop: &HopExpr, catalog: &ColumnCatalog) -> (bool, bool) {
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

pub(super) fn path_actuals(path: &PathClosure, catalog: &ColumnCatalog) -> AliasActuals {
    let (s, o) = hop_text(&path.hop, catalog);
    let mut text_columns = HashMap::new();
    if s {
        text_columns.insert("sf_s".into(), TextKey::Verbatim);
    }
    if o {
        text_columns.insert("sf_o".into(), TextKey::Verbatim);
    }
    AliasActuals {
        scalar_columns: HashMap::new(),
        sqlite_columns: HashMap::new(),
        lexical_columns: HashMap::new(),
        source_kind: AliasSourceKind::Derived,
        columns: vec!["sf_s".into(), "sf_o".into()],
        path: true,
        text_columns,
    }
}

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

pub(super) fn subplan_actuals(
    plan: &crate::Plan,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> AliasActuals {
    #[cfg(test)]
    METADATA_VISITS.with(|visits| visits.set(visits.get() + 1));
    let mut width = 0;
    let mut common: Option<HashMap<usize, TextKey>> = None;
    let mut common_scalars: Option<HashMap<usize, NativeScalarKey>> = None;
    for branch in &plan.branches {
        let effective_distinct = if plan.branches.len() == 1 {
            plan.distinct
        } else {
            branch.distinct
        };
        let projection = source_projection(branch, effective_distinct, dialect);
        width = width.max(projection.len());
        let actuals = branch_actuals(branch, dialect, catalog);
        let scalars: HashMap<_, _> = projection
            .iter()
            .enumerate()
            .filter_map(|(index, column)| {
                column
                    .as_ref()
                    .and_then(|column| iri_cmp::scalar_column(column, &actuals))
                    // UNION can widen decimal scale/display metadata. Do not
                    // inherit a pre-coercion lexical proof through that boundary.
                    .filter(|key| plan.branches.len() == 1 || *key != NativeScalarKey::MysqlDecimal)
                    .map(|key| (index, key))
            })
            .collect();
        match common_scalars.as_mut() {
            None => common_scalars = Some(scalars),
            Some(common) => common.retain(|index, key| scalars.get(index) == Some(key)),
        }
        let text: HashMap<_, _> = projection
            .iter()
            .enumerate()
            .filter_map(|(index, column)| {
                column
                    .as_ref()
                    .and_then(|column| column_text(column, &actuals))
                    .map(|key| {
                        (
                            index,
                            if branch.agg.is_none()
                                && branch.path.is_none()
                                && (plan.distinct || effective_distinct)
                                && !crate::cascade::eligible_for_term_dedup_with_distinct(
                                    branch,
                                    effective_distinct,
                                )
                            {
                                TextKey::Verbatim
                            } else {
                                key
                            },
                        )
                    })
            })
            .collect();
        match common.as_mut() {
            None => common = Some(text),
            Some(common) => common.retain(|index, key| text.get(index) == Some(key)),
        }
    }
    let columns: Vec<_> = (0..width).map(|i| format!("c{i}")).collect();
    let text_columns = common
        .unwrap_or_default()
        .into_iter()
        .map(|(i, key)| (format!("c{i}"), key))
        .collect();
    AliasActuals {
        scalar_columns: common_scalars
            .unwrap_or_default()
            .into_iter()
            .map(|(i, key)| (format!("c{i}"), key))
            .collect(),
        sqlite_columns: HashMap::new(),
        lexical_columns: HashMap::new(),
        source_kind: AliasSourceKind::Derived,
        columns,
        path: plan.branches.iter().any(branch_has_path),
        text_columns,
    }
}

pub(super) fn render_key_equality(
    a: &ColRef,
    b: &ColRef,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
) -> String {
    if dialect == Dialect::Sqlite
        && [a, b].iter().all(|column| {
            lexical_key::proven_column(column, actuals).is_some()
                || column_text(column, actuals).is_some()
        })
    {
        return format!(
            "{} = {}",
            rdf_column(a, dialect, catalog, actuals),
            rdf_column(b, dialect, catalog, actuals)
        );
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
    format!("{left} = {right}")
}
