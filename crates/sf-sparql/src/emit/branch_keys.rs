//! Plain (non-path, non-aggregate) branch SELECT rendering and its ORDER/slice tail.
use super::*;

pub(super) async fn emit_branch_keys(
    b: &Branch,
    bindings: &BindingView<'_>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    normalize_projection: bool,
    modifiers: BranchModifiers,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<EmittedBranch> {
    let actuals = branch_actuals_controlled(b, dialect, catalog, work)?;
    if let Some(pc) = &b.path {
        return emit_path_branch(b, pc, dialect, catalog, modifiers, work).await;
    }
    if let Some(agg) = &b.agg {
        return emit_agg_branch(b, agg, dialect, catalog, &actuals, modifiers, work).await;
    }
    // ADR-0025 (C.3): SQL `DISTINCT` dedups RAW columns, so it implements SPARQL DISTINCT
    // (dedup on the RECONSTRUCTED term) only when every projected term is INJECTIVE in its
    // raw columns. A non-injective template (distinct raw tuples → the same RDF term, e.g.
    // `http://ex/{a}{b}` over `(1,23)`/`(12,3)`) would survive SQL DISTINCT as duplicates
    // SPARQL must collapse — a silent wrong answer. Sound 501 — UNLESS `b` also qualifies
    // for Run 4 Wave C0d's term-level dedup ([`crate::cascade::eligible_for_term_dedup`]):
    // a branch whose relation stands alone in its plan slot answers instead of refusing —
    // `term_dedup` below suppresses the SQL `DISTINCT` this function would otherwise emit,
    // leaving the raw, duplicate-bearing rows for `exec_core::run_branches` to dedup AFTER
    // reconstruction, on the actual term values. Injective templates need neither path —
    // SQL DISTINCT already implements SPARQL DISTINCT for them.
    let (projection, term_dedup) = aggregate_projection::projection_from_bindings(
        b,
        bindings,
        dialect,
        modifiers.distinct,
        work,
    )?;
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // FROM (+ LEFT JOIN ON params) MUST render before WHERE so positional
    // placeholders bind in text order. A core-less branch with no SubPlan joins
    // AND no opts (an inline `VALUES` constant row — all `Const` bindings)
    // renders as a one-row `SELECT <const exprs>` with no FROM. A core-less
    // branch WITH SubPlan joins (ADR-0023 M5 Wave 2: SubPlan anchor) or opts (a
    // core-less `LeftJoin` left side, e.g. `{} OPTIONAL {...}`) needs
    // `render_from`'s synthetic/SubPlan anchor — omitting it left the opt's own
    // columns referenced with no FROM clause ever introducing their alias.
    let from = if b.core.is_empty() && b.subplan_joins.is_empty() && b.opts.is_empty() {
        None
    } else {
        Some(
            render_from_async_controlled(
                b,
                dialect,
                catalog,
                &actuals,
                &mut params,
                &mut pidx,
                work,
            )
            .await?,
        )
    };
    let where_sql = render_where(
        &b.where_conds,
        dialect,
        catalog,
        &actuals,
        &mut params,
        &mut pidx,
        work,
    )?;

    let select_list = if projection.is_empty() {
        "1 AS c0".to_owned()
    } else {
        projection
            .iter()
            .enumerate()
            .map(|(i, c)| {
                format!(
                    "{} AS c{i}",
                    if normalize_projection && !term_dedup {
                        path_comparison::rdf_column(c, dialect, catalog, &actuals)
                    } else {
                        colref(c, dialect, &actuals)
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    // `term_dedup` skips SQL DISTINCT even though `b.distinct` is set (see the C.3 gate
    // above) — its raw, non-injective duplicates are collapsed downstream, by TERM, not
    // by raw-column SQL DISTINCT (which would be the unsound operation C.3 refuses).
    let numeric_keys = if modifiers.distinct
        && !term_dedup
        && matches!(dialect, Dialect::Postgres | Dialect::MySql)
    {
        pg_numeric::distinct_keys_for_projection(bindings, dialect, &actuals, &projection, work)?
    } else {
        Vec::new()
    };
    let numeric_distinct =
        modifiers.distinct && !term_dedup && numeric_keys.iter().any(Option::is_some);
    let literal_window = if modifiers.distinct && !term_dedup && dialect == Dialect::Sqlite {
        literal_roles::sqlite_distinct(bindings, catalog, &actuals, &projection, work)?
    } else {
        None
    };
    let (select_list, literal_output) = match &literal_window {
        Some(keys) => {
            let raw = projection
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{} AS c{i}", colref(c, dialect, &actuals)))
                .collect::<Vec<_>>()
                .join(", ");
            let (select, output) = literal_roles::window(&raw, keys, projection.len());
            (select, Some(output))
        }
        None => (select_list, None),
    };
    let distinct =
        if modifiers.distinct && !term_dedup && !numeric_distinct && literal_window.is_none() {
            "DISTINCT "
        } else {
            ""
        };
    work.charge(select_list.len() + from.as_deref().map_or(0, str::len) + 64)
        .map_err(source_control::validation_error)?;
    let mut skeleton = match from {
        Some(f) => format!("SELECT {distinct}{select_list} FROM {f}"),
        None => format!("SELECT {distinct}{select_list}"),
    };
    if let Some(w) = where_sql {
        work.charge(w.len() + 7)
            .map_err(source_control::validation_error)?;
        skeleton.push_str(" WHERE ");
        skeleton.push_str(&w);
    }
    if numeric_distinct {
        work.charge(skeleton.len())
            .map_err(source_control::validation_error)?;
        work.product(numeric_keys.len(), 96)
            .map_err(source_control::validation_error)?;
        skeleton = pg_numeric::distinct_sql(skeleton, &numeric_keys);
    }
    if let Some(output) = literal_output {
        work.charge(skeleton.len() + output.len() + 96)
            .map_err(source_control::validation_error)?;
        skeleton = format!("SELECT {output} FROM ({skeleton}) __sf_literal_raw WHERE __sf_literal_raw.__sf_literal_rank = 1");
    }
    // ORDER BY precedes LIMIT/OFFSET (SPARQL §15: order, then slice).
    if let Some(order) = if numeric_distinct || literal_window.is_some() {
        pg_numeric::order(b, bindings, &projection, dialect, work)?
    } else {
        render_order(
            &b.order,
            bindings,
            dialect,
            catalog,
            &actuals,
            normalize_projection && !term_dedup,
            work,
        )?
    } {
        work.charge(order.len())
            .map_err(source_control::validation_error)?;
        skeleton.push_str(&order);
    }
    push_limit_offset(&mut skeleton, modifiers, dialect);

    let sql = emit_via_ast_governed(dialect, &skeleton, work).await?;
    Ok(EmittedBranch {
        sql,
        metadata_sql: None,
        sqlite_character_keys: false,
        sqlite_lexical_keys: false,
        projection,
        params,
    })
}

/// Push a `LIMIT`/`OFFSET` tail onto `skeleton` (called after `WHERE`/`ORDER BY`
/// have already been rendered, per each caller's own SQL clause order). A BARE
/// `OFFSET` (no `LIMIT`) is a genuine SPARQL shape (`OFFSET n` with no `LIMIT`
/// is valid syntax) but not every dialect's grammar accepts a standalone
/// `OFFSET` clause — `Dialect::bare_offset_limit_sentinel` renders an explicit
/// "no limit" `LIMIT` first for the dialects that need one (confirmed live: a
/// bare `OFFSET n` is a SQLite/MySQL syntax error, so this genuinely fixed a
/// live-emission failure, not a hypothetical one).
pub(super) fn push_limit_offset(skeleton: &mut String, m: BranchModifiers, dialect: Dialect) {
    match m.limit {
        Some(limit) => skeleton.push_str(&format!(" LIMIT {limit}")),
        None if m.offset > 0 => {
            if let Some(sentinel) = dialect.bare_offset_limit_sentinel() {
                skeleton.push_str(&format!(" LIMIT {sentinel}"));
            }
        }
        None => {}
    }
    if m.offset > 0 {
        skeleton.push_str(&format!(" OFFSET {}", m.offset));
    }
}

/// Render an `ORDER BY` clause that pins the SPARQL value-space order, or `None`
/// when there are no keys. Each key is a bound variable; its `rr:column` term map
/// lowers to a raw column whose SQL order equals the RDF lexical/value order (a
/// homogeneous literal column, or an IRI whose value *is* the column). NULL
/// placement is **always explicit** — `NULLS FIRST` for ASC, `NULLS LAST` for DESC
/// — never the dialect default (PostgreSQL: NULLS LAST; SQLite: NULLS FIRST), so an
/// unbound key sorts first (ASC) / last (DESC) on every engine. A key bound to a
/// non-`rr:column` term (a constructed template IRI, COALESCE, …) cannot be ordered
/// soundly in SQL (the constructed string ≠ the column order) → honest 501.
pub(super) fn render_order(
    order: &[OrderKey],
    bindings: &BindingView<'_>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    normalize: bool,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Option<String>> {
    if order.is_empty() {
        return Ok(None);
    }
    let mut terms = Vec::with_capacity(order.len());
    for key in order {
        let def = bindings.get(&key.var, work)?.ok_or_else(|| {
            Error::Unsupported(format!(
                "ORDER BY ?{} is not a bound variable → 501",
                key.var
            ))
        })?;
        let col = order_column(def).ok_or_else(|| {
            Error::Unsupported(format!(
                "ORDER BY ?{} is not an rr:column term — a constructed/derived sort \
                 key cannot be ordered soundly in SQL → 501",
                key.var
            ))
        })?;
        let dir = if key.descending {
            "DESC NULLS LAST"
        } else {
            "ASC NULLS FIRST"
        };
        terms.push(format!(
            "{} {dir}",
            if normalize {
                path_comparison::rdf_column(&col, dialect, catalog, actuals)
            } else {
                colref(&col, dialect, actuals)
            }
        ));
    }
    Ok(Some(format!(" ORDER BY {}", terms.join(", "))))
}

/// The single raw column an ORDER BY key lowers to, iff the key is a bound
/// `rr:column` term map (the SQL-order-sound case). A template / COALESCE / CONCAT /
/// constant has no such column → `None` (the caller defers to 501).
pub(super) fn order_column(def: &TermDef) -> Option<ColRef> {
    match def {
        TermDef::Derived {
            term_map: TermMap::Column(c, _),
            alias,
        } => Some(ColRef::new(*alias, c.clone())),
        TermDef::R2rmlBlank {
            term_map: TermMap::Column(c, _),
            alias,
            graph:
                R2rmlGraphScope::Default
                | R2rmlGraphScope::Mapped {
                    term_map: TermMap::Constant(_),
                    ..
                },
        } => Some(ColRef::new(*alias, c.clone())),
        _ => None,
    }
}
