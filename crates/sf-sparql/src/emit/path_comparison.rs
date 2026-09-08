//! Comparison decoration preserves native values and outer RDF reconstruction.
//! Varying text needs byte-exact, NO PAD comparison. Other native type families
//! require separate decoder-equivalence work; a text cast is not that proof.
use super::*;
#[cfg(test)]
mod tests;
#[cfg(test)]
thread_local! { static METADATA_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

fn exact_text(expression: String, dialect: Dialect) -> String {
    match dialect {
        Dialect::Sqlite => format!("({expression} COLLATE BINARY)"),
        Dialect::Postgres => format!("({expression} COLLATE \"C\")"),
        // Unlike utf8mb4_bin, the MySQL8 collation is NO PAD. Keep a text
        // result: projecting BINARY would change the driver decoder to hex.
        Dialect::MySql => format!("(CONVERT({expression} USING utf8mb4) COLLATE utf8mb4_0900_bin)"),
        _ => expression,
    }
}

fn source_text(source: &LogicalSource, column: &str, catalog: &ColumnCatalog) -> bool {
    let name = resolve_col(column, catalog.columns(source));
    catalog
        .text_by_source
        .get(&source_key(source))
        .is_some_and(|columns| columns.contains(name))
}

pub(super) fn path_key_expression(
    expression: String,
    source: &LogicalSource,
    column: &str,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> String {
    if !catalog.suppress_path_collation
        && (dialect == Dialect::Sqlite || source_text(source, column, catalog))
    {
        exact_text(expression, dialect)
    } else {
        expression
    }
}

fn hop_text(hop: &HopExpr, catalog: &ColumnCatalog) -> (bool, bool) {
    match hop {
        HopExpr::Pred(rel) => (
            source_text(&rel.source, &rel.subj_col, catalog),
            source_text(&rel.source, &rel.obj_col, catalog),
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
    let mut text_columns = HashSet::new();
    if s {
        text_columns.insert("sf_s".into());
    }
    if o {
        text_columns.insert("sf_o".into());
    }
    AliasActuals {
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

fn column_text(column: &ColRef, actuals: &ActualColumns) -> bool {
    actuals.get(&column.alias).is_some_and(|a| {
        a.text_columns
            .contains(resolve_col(&column.column, Some(&a.columns)))
    })
}

pub(super) fn subplan_actuals(
    plan: &crate::Plan,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> AliasActuals {
    #[cfg(test)]
    METADATA_VISITS.with(|visits| visits.set(visits.get() + 1));
    let mut width = 0;
    let mut common: Option<HashSet<usize>> = None;
    for branch in &plan.branches {
        let projection: Vec<_> = match &branch.agg {
            Some(agg) if branch.path.is_none() => aggregate_projection(agg, dialect)
                .iter()
                .map(|item| item.source_column().cloned())
                .collect(),
            _ => branch
                .projection_with_distinct(if plan.branches.len() == 1 {
                    plan.distinct
                } else {
                    branch.distinct
                })
                .into_iter()
                .map(Some)
                .collect(),
        };
        width = width.max(projection.len());
        let actuals = branch_actuals(branch, dialect, catalog);
        let text: HashSet<_> = projection
            .iter()
            .enumerate()
            .filter_map(|(index, column)| {
                column
                    .as_ref()
                    .is_some_and(|column| column_text(column, &actuals))
                    .then_some(index)
            })
            .collect();
        match common.as_mut() {
            None => common = Some(text),
            Some(common) => common.retain(|index| text.contains(index)),
        }
    }
    let columns: Vec<_> = (0..width).map(|i| format!("c{i}")).collect();
    let text_columns = common
        .unwrap_or_default()
        .into_iter()
        .map(|i| format!("c{i}"))
        .collect();
    AliasActuals {
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
    let (mut left, mut right) = (colref(a, dialect, actuals), colref(b, dialect, actuals));
    let path = [a, b]
        .iter()
        .any(|c| actuals.get(&c.alias).is_some_and(|a| a.path));
    if path
        && !catalog.suppress_path_collation
        && (dialect == Dialect::Sqlite || (column_text(a, actuals) && column_text(b, actuals)))
    {
        left = exact_text(left, dialect);
        right = exact_text(right, dialect);
    }
    format!("{left} = {right}")
}
