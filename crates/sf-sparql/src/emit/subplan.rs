//! Nested SubPlan derived-table SQL and placeholder rebasing.
use super::*;

type SubplanSqlFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(String, Vec<String>)>> + Send + 'a>>;

/// Render all prepared branches of a nested [`Plan`] to a single SQL SELECT string
/// (for embedding as a derived table). Recursively probed live names override the
/// offline lexical fallback. Multi-branch plans become a `UNION ALL`. Returns
/// `(sql_text, params)` — params in text order, placeholders starting from 1.
#[cfg(test)]
pub(super) fn emit_subplan_sql(
    plan: &crate::Plan,
    dialect: Dialect,
    live_catalog: &ColumnCatalog,
) -> Result<(String, Vec<String>)> {
    crate::exec_core::block_on(emit_subplan_sql_async_controlled(
        plan,
        dialect,
        live_catalog,
        sf_sql::source_work::SourceWork::new(None),
    ))
}

#[cfg(test)]
pub(super) fn emit_subplan_sql_controlled<'a>(
    plan: &'a crate::Plan,
    dialect: Dialect,
    live_catalog: &'a ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'a>,
) -> Result<(String, Vec<String>)> {
    crate::exec_core::block_on(emit_subplan_sql_async_controlled(
        plan,
        dialect,
        live_catalog,
        work,
    ))
}

pub(super) fn emit_subplan_sql_async_controlled<'a>(
    plan: &'a crate::Plan,
    dialect: Dialect,
    live_catalog: &'a ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'a>,
) -> SubplanSqlFuture<'a> {
    Box::pin(async move {
        work.charge(1).map_err(source_control::validation_error)?;
        // Preparation changes only root scalar modifiers. Borrow the forest so a
        // unary wrapper chain does not recursively copy every remaining subtree.
        let branches = &plan.branches;
        let mut catalog = synthetic_subplan_catalog_controlled(branches, work)?;
        catalog.suppress_path_collation = live_catalog.suppress_path_collation;
        catalog.character_keys = std::sync::Arc::clone(&live_catalog.character_keys);
        catalog.lexical_keys = std::sync::Arc::clone(&live_catalog.lexical_keys);
        // Live top-level execution has already probed every recursively reachable
        // base source. Overlay those authoritative names so nested SubPlan emission
        // does not depend on the offline lexical alias-folding heuristic. The
        // synthetic entries remain only for dialect-neutral/offline emission and
        // source-free derived columns.
        let sources = match work.control() {
            Some(control) => source_control::live_metadata_sources_controlled(branches, control)
                .map_err(source_control::validation_error)?,
            None => live_metadata_sources(branches),
        };
        for source in sources {
            // One paid key copy per source, cloned once per overlaid map.
            let text_len = match source {
                LogicalSource::Table(table) => table.len(),
                LogicalSource::Query(query) => query.len(),
            };
            work.product(text_len + 2, 6)
                .map_err(source_control::validation_error)?;
            let key = source_key(source);
            let Some(columns) = live_catalog.by_source.get(&key) else {
                continue;
            };
            work.product(columns.len(), std::mem::size_of::<String>())
                .map_err(source_control::validation_error)?;
            for column in columns {
                work.charge(column.len())
                    .map_err(source_control::validation_error)?;
            }
            std::sync::Arc::make_mut(&mut catalog.by_source).insert(key.clone(), columns.clone());
            macro_rules! overlay {
                ($field:ident) => {
                    let map = std::sync::Arc::make_mut(&mut catalog.$field);
                    match live_catalog.$field.get(&key) {
                        Some(live) => {
                            source_control::map_copy(live, work)
                                .map_err(source_control::validation_error)?;
                            map.insert(key.clone(), live.clone());
                        }
                        None => {
                            map.remove(&key);
                        }
                    }
                };
            }
            overlay!(text_by_source);
            overlay!(sqlite_by_source);
            overlay!(scalars_by_source);
            overlay!(datatypes_by_source);
        }
        pg_float::validate_union(branches, dialect, &catalog, plan.distinct, work)?;
        mysql_float_value::identity::validate_union_controlled(branches, dialect, &catalog, work)?;
        let mut emitted = Vec::with_capacity(branches.len());
        for branch in branches {
            let modifiers = BranchModifiers::prepared(
                branch,
                branches.len() == 1,
                plan.distinct,
                plan.order.is_empty(),
                plan.limit,
                plan.offset,
            );
            emitted.push(
                emit_branch_keys(
                    branch,
                    &BindingView::Direct(&branch.bindings),
                    dialect,
                    &catalog,
                    plan.distinct || modifiers.distinct,
                    modifiers,
                    work,
                )
                .await?,
            );
        }
        if emitted.is_empty() {
            // Empty inner plan — a values-empty derived table: return a SELECT with no rows.
            // Use a dummy column so it is syntactically valid as a derived table.
            return Ok(("SELECT 1 AS __sf_empty WHERE 1 = 0".to_owned(), Vec::new()));
        }
        if emitted.len() == 1 {
            let e = emitted.into_iter().next().expect("one emitted branch");
            return Ok((e.sql, e.params));
        }
        // Multiple branches: `UNION ALL` (bag semantics) by default, or `UNION` (dedup) when the
        // plan carries a DISTINCT (a multi-branch DISTINCT SubPlan, ADR-0025 Tier-2 gap 2 — the
        // pooling requires injective cross-arm reconstruction, so SQL `UNION`'s raw-column dedup
        // equals SPARQL DISTINCT on the reconstructed terms). SQLite's compound-select grammar
        // does NOT accept a parenthesised `select-core` as a UNION operand (`(SELECT …) UNION …`
        // is a syntax error there — the q9 agg-pushdown wave's first live failure); PG/MySQL
        // accept it. So SQLite joins the arms bare.
        let mut all_sql = Vec::new();
        let mut all_params = Vec::new();
        let numeric_keys = if plan.distinct {
            pg_numeric::union_keys(branches, dialect, &catalog, work)?
        } else {
            None
        };
        for e in emitted {
            let sql = rebase_placeholders_controlled(&e.sql, dialect, all_params.len(), work)?;
            if dialect == Dialect::Sqlite {
                all_sql.push(sql);
            } else {
                work.charge(sql.len() + 2)
                    .map_err(source_control::validation_error)?;
                all_sql.push(format!("({sql})"));
            }
            let mut parameter_index = all_params.len();
            work.append_parameters(&mut all_params, &mut parameter_index, e.params)
                .map_err(source_control::validation_error)?;
        }
        let op = if plan.distinct && numeric_keys.is_none() {
            " UNION "
        } else {
            " UNION ALL "
        };
        for sql in &all_sql {
            work.charge(sql.len() + op.len())
                .map_err(source_control::validation_error)?;
        }
        let raw = all_sql.join(op);
        let sql = match numeric_keys {
            Some(keys) => {
                work.charge(raw.len())
                    .map_err(source_control::validation_error)?;
                work.product(keys.len(), 96)
                    .map_err(source_control::validation_error)?;
                pg_numeric::distinct_sql(raw, &keys)
            }
            None => raw,
        };
        Ok((sql, all_params))
    })
}

/// Rebase positional `$N` placeholders in `sql` from base 1 to start at `base+1`,
/// for PostgreSQL numbered placeholders. SQLite uses `?` (positional by text order,
/// no numbering), so for SQLite (or when `base == 0`) returns `sql` unchanged.
pub(super) fn rebase_placeholders(sql: &str, dialect: Dialect, base: usize) -> Result<String> {
    sf_sql::dialect::rebase_placeholders(sql, dialect, base)
        .map_err(|error| Error::Sql(error.to_string()))
}

/// The output copy, plus the tokenizer pass and character scan on PostgreSQL.
pub(super) fn rebase_placeholders_controlled(
    sql: &str,
    dialect: Dialect,
    base: usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    let passes = if dialect == Dialect::Postgres && base > 0 {
        3
    } else {
        1
    };
    work.product(sql.len(), passes)
        .map_err(source_control::validation_error)?;
    rebase_placeholders(sql, dialect, base)
}
