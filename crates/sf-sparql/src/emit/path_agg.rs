//! Property-path closure and GROUP BY branch rendering.
use super::*;

/// Render a property-path closure branch to a (possibly `WITH RECURSIVE`) CTE
/// (ADR-0007 *recursive paths compile to source-dialect recursive CTEs*).
///
/// The hop relation ([`HopExpr`]) compiles to a subquery yielding the canonical
/// **raw key columns** `sf_s` / `sf_o` (term-gen lifting): a bare
/// predicate is a base scan, and `^p`/`p/q`/`p|q`/`!p` are nested subqueries over
/// the same keys. A second relation reduces it to the distinct reachable node
/// pairs the outer projection reads as `t{alias}` — SPARQL paths are set-semantics
/// over node **pairs**, so this `SELECT DISTINCT sf_s, sf_o` is what keeps
/// `SELECT ?s WHERE {?s P ?o}` a correct bag of `?s` even when `?o` is dropped.
///
/// Per [`PathKind`]:
/// * `One` (`^p`, `p/q`, `p|q`) — just the distinct hop pairs, no recursion; `!p`
///   (NPS) is the bag exception — its `UNION ALL` hop is kept un-`DISTINCT`ed.
/// * `ZeroOrOne` (`p?`) — the hop ∪ the reflexive `(x, x)` pairs over the active
///   graph's nodes (only over a single-predicate bare leaf, `unfold`-enforced).
/// * `OneOrMore` (`P+`) — a `WITH RECURSIVE` finite-pair fixed point. Its `UNION`
///   deduplicates on `(sf_s, sf_o)`, so cyclic revisits are discarded and recursion
///   ends only when no new reachable pair exists. This is exact for a finite source
///   relation; no successful result is cut off at an arbitrary walk depth.
/// * `ZeroOrMore` (`P*`) — `OneOrMore` plus the reflexive `(x, x)` pairs.
///
/// RDF terms are still built only at the outer projection ([`crate::exec`]).
/// The `WITH …` prelude defining the path closure's distinct-pairs relation
/// `t{alias}(sf_s, sf_o)` — a plain CTE for length-1 shapes, a `WITH RECURSIVE` for `+`/`*`.
/// Shared by [`emit_path_branch`] (a standalone path result) and the `PathExists`
/// correlated-EXISTS emission (ADR-0025 Tier-2 gap 1); both reference `t{alias}.sf_s`/`.sf_o`.
pub(super) async fn emit_path_branch(
    b: &Branch,
    pc: &PathClosure,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    modifiers: BranchModifiers,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<EmittedBranch> {
    // ORDER BY over a path result is handled at the exec layer (plan.order →
    // Rust-level order_cmp) — Branch.order is always empty here, so no guard needed.
    let projection = b.projection_with_distinct(modifiers.distinct);
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // `cte` is the distinct-pairs relation the outer projection reads (colref binds
    // `t{alias}`). Its columns are the canonical `sf_s` / `sf_o` keys, never base
    // columns, so the outer projection / WHERE resolve against an empty catalog.
    let cte = format!("t{}", pc.alias);
    let outer_actuals = HashMap::from([(pc.alias, path_actuals_controlled(pc, catalog, work)?)]);
    let with = path_sql::prelude(pc, pc.alias, dialect, catalog, work)?;

    let select_list = projection
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{} AS c{i}", colref(c, dialect, &outer_actuals)))
        .collect::<Vec<_>>()
        .join(", ");

    let distinct = if modifiers.distinct { "DISTINCT " } else { "" };
    work.charge(with.len() + select_list.len() + cte.len() + 64)
        .map_err(source_control::validation_error)?;
    let mut skeleton = format!("{with} SELECT {distinct}{select_list} FROM {cte}");
    if let Some(w) = render_where(
        &b.where_conds,
        dialect,
        catalog,
        &outer_actuals,
        &mut params,
        &mut pidx,
        work,
    )? {
        skeleton.push_str(" WHERE ");
        skeleton.push_str(&w);
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

/// Render a GROUP BY + aggregates branch (SPARQL §11) to one parameterised SQL
/// `SELECT … GROUP BY …`. The grouping keys are their **raw key columns** (term
/// construction is rebuilt at reconstruction, ADR-0007), grouped and projected;
/// each aggregate is `COUNT`/`SUM`/`AVG`/`MIN`/`MAX(…)` over its single raw column
/// (or `COUNT(*)`). Implicit grouping (no keys) emits no `GROUP BY`, yielding one
/// row over all inner rows — and one row even when the inner is empty (`COUNT(*)`
/// = 0). The projection is built in lockstep with the SELECT list so reconstruction
/// reads each key/aggregate by position. The aggregate result columns are synthetic
/// (computed in SQL), so their `§10` type is set explicitly at reconstruction (see
/// [`TermDef::Agg`]), never read from a base column.
pub(super) async fn emit_agg_branch(
    b: &Branch,
    agg: &Aggregation,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    modifiers: BranchModifiers,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<EmittedBranch> {
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // FROM (+ LEFT JOIN ON params) renders before WHERE so positional placeholders
    // bind in text order. A core-less inner with no SubPlan join and no opts
    // either (an empty BGP) renders without FROM; a core-less inner WITH a
    // SubPlan join (the SQL agg-over-UNION pushdown: the pooled arms' derived
    // table is the sole FROM relation) or opts (aggregating over `{} OPTIONAL
    // {...}}`) still needs `render_from` — mirrors `emit_branch_with`'s condition.
    let from = if b.core.is_empty() && b.subplan_joins.is_empty() && b.opts.is_empty() {
        None
    } else {
        Some(
            render_from_async_controlled(
                b,
                dialect,
                catalog,
                actuals,
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
        actuals,
        &mut params,
        &mut pidx,
        work,
    )?;

    // The projection + SELECT list, in lockstep: grouping-key raw columns first
    // (also the GROUP BY columns), then each aggregate expression.
    let layout = aggregate_projection(agg, dialect);
    let projection: Vec<ColRef> = layout.iter().map(|item| item.column().clone()).collect();
    let mut select_items: Vec<String> = Vec::new();
    let mut group_cols: Vec<String> = Vec::new();
    for (i, item) in layout.iter().enumerate() {
        let expression = match item {
            AggregateProjection::Key(column) => {
                let rendered = colref(column, dialect, actuals);
                if let Some(keys) = (dialect == Dialect::Sqlite)
                    .then(|| literal_roles::sqlite_group_keys(b, agg, column, catalog, actuals))
                    .flatten()
                {
                    group_cols.extend(keys);
                } else {
                    group_cols.push(rendered.clone());
                }
                rendered
            }
            AggregateProjection::Aggregate(aggregate) => agg_expr_sql(aggregate, dialect, actuals),
            AggregateProjection::AvgOperand(column) => colref(column, dialect, actuals),
        };
        select_items.push(format!("{expression} AS c{i}"));
    }
    let select_list = select_items.join(", ");

    let distinct = if modifiers.distinct { "DISTINCT " } else { "" };
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
    if !group_cols.is_empty() {
        // The joined list and its copy into the skeleton.
        let joined = group_cols
            .iter()
            .fold(10usize, |n, c| n.saturating_add(c.len() + 2));
        work.charge(joined.saturating_mul(2))
            .map_err(source_control::validation_error)?;
        skeleton.push_str(" GROUP BY ");
        skeleton.push_str(&group_cols.join(", "));
    }
    // ORDER BY over an aggregate result is applied in `exec` (never pushed to SQL —
    // it sorts the reconstructed terms type-aware). LIMIT/OFFSET on a single agg
    // branch were pushed by `Plan::prepared_branches` only when unordered.
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

/// The SQL for one aggregate output column (`COUNT(*)` / `COUNT`/`SUM`/`AVG`/`MIN`/
/// `MAX(<col>)`, with optional `DISTINCT`). `MIN`/`MAX` ignore DISTINCT (it never
/// changes the extremum), but it is rendered when requested for faithfulness.
pub(super) fn agg_expr_sql(a: &AggCol, dialect: Dialect, actuals: &ActualColumns) -> String {
    let d = if a.distinct { "DISTINCT " } else { "" };
    let func = match a.kind {
        AggKind::Count => "COUNT",
        AggKind::Sum => "SUM",
        AggKind::Avg => "AVG",
        AggKind::Min => "MIN",
        AggKind::Max => "MAX",
    };
    match &a.arg {
        // COUNT(*) — the only argument-less form (DISTINCT is rejected upstream).
        None => format!("{func}(*)"),
        Some(col) => format!("{func}({d}{})", colref(col, dialect, actuals)),
    }
}
