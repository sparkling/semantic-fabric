//! FROM clause assembly, SubPlan joins and condition rendering entry points.
use super::*;

#[cfg(test)]
pub(super) fn scan_ref(
    scan: &crate::iq::Scan,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    scan_ref_controlled(
        scan,
        dialect,
        catalog,
        params,
        pidx,
        sf_sql::source_work::SourceWork::new(None),
    )
}

// Reference-atom validation guarantees exactly two base scans and no optional
// or nested-plan joins, so that narrow path cannot suspend.
pub(super) fn render_from_controlled(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    _actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    let mut scans = b.core.iter();
    let first = scans.next().expect("validated reference atom left scan");
    let mut from = scan_ref_controlled(first, dialect, catalog, params, pidx, work)?;
    for scan in scans {
        from.push_str(" CROSS JOIN ");
        let piece = scan_ref_controlled(scan, dialect, catalog, params, pidx, work)?;
        work.charge(piece.len())
            .map_err(source_control::validation_error)?;
        from.push_str(&piece);
    }
    Ok(from)
}

pub(super) async fn render_from_async_controlled(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    work.checkpoint()
        .map_err(source_control::validation_error)?;
    // SubPlan derived-table joins (ADR-0023 M5 Wave 2): nested Plans inlined as
    // `(SELECT …) AS t{alias}`. Nested params are spliced at this text-order position
    // (load-bearing for prepared-statement binding — positional order matters).
    //
    // When `core` is non-empty the first core scan is the FROM anchor; SubPlan joins
    // follow as INNER/LEFT JOIN. When `core` is empty AND there are SubPlan joins the
    // first SubPlan becomes the FROM anchor (no CROSS JOIN keyword before it).
    // Pay each rendered piece before copying it into the FROM text.
    let append = |from: &mut String, piece: String| -> Result<()> {
        work.charge(piece.len())
            .map_err(source_control::validation_error)?;
        from.push_str(&piece);
        Ok(())
    };

    let from = if b.core.is_empty() {
        // No base (core) scans. A synthetic one-row anchor stands in for the
        // `IqNode::True` / empty-BGP identity this branch's own left side represents
        // (SPARQL's `{} OPTIONAL {X}` always has exactly one solution to extend), and
        // EVERY opt / SubPlan attaches to it via its own JOIN — never as a hard FROM
        // anchor that would wrongly drop that guaranteed row on zero matches.
        //
        // Opts render BEFORE SubPlans — the SAME order as the core-bearing branch below
        // — so a SubPlan whose `on` correlates on a prior opt (or earlier SubPlan)
        // references a table already emitted to its LEFT (valid SQL). Previously this
        // path made the FIRST SubPlan the FROM anchor and emitted it with NO `ON` clause
        // AND rendered opts AFTER SubPlans: a SubPlan correlated on an opt then either
        // silently DROPPED its correlation (a wrong answer — an uncorrelated cross join)
        // or referenced an opt emitted to its right (a crash) — both ADR-0007 violations.
        // The uniform "(SELECT 1) anchor, then opts, then SubPlans" order fixes them and
        // matches the already-shipped core-less-plus-opts fix
        // (`bare_group_as_leftjoin_left_no_longer_mis_aliases`). A one-row cross join is
        // `=_bag`-transparent, so the previously-anchor-was-a-SubPlan cases (an
        // uncorrelated `{} OPTIONAL {sub}`, the SQL agg-over-UNION pushdown) keep their
        // meaning — only their cosmetic SQL shape changes.
        let mut from = "(SELECT 1) t_empty".to_owned();
        for opt in &b.opts {
            from.push_str(" LEFT JOIN ");
            let scan = scan::template::restrict_optional_controlled(opt, dialect, catalog, work)?;
            append(
                &mut from,
                scan_ref_controlled(&scan, dialect, catalog, params, pidx, work)?,
            )?;
            from.push_str(" ON ");
            let conds: Vec<&SqlCond> = opt.on.iter().chain(opt.extra.iter()).collect();
            append(
                &mut from,
                render_conjunction(&conds, dialect, catalog, actuals, params, pidx, work)?,
            )?;
        }
        for sp in &b.subplan_joins {
            let join_kw = if sp.left {
                " LEFT JOIN "
            } else {
                " INNER JOIN "
            };
            append(
                &mut from,
                emit_subplan_join_controlled(sp, dialect, catalog, params, pidx, join_kw, work)
                    .await?,
            )?;
            if !sp.on.is_empty() {
                from.push_str(" ON ");
                let conds: Vec<&SqlCond> = sp.on.iter().collect();
                append(
                    &mut from,
                    render_conjunction(&conds, dialect, catalog, actuals, params, pidx, work)?,
                )?;
            } else {
                from.push_str(" ON 1 = 1");
            }
        }
        from
    } else {
        let mut scans = b.core.iter();
        let first = scans.next().expect("core non-empty — checked above");
        let mut from = scan_ref_controlled(first, dialect, catalog, params, pidx, work)?;
        for s in scans {
            from.push_str(" CROSS JOIN ");
            append(
                &mut from,
                scan_ref_controlled(s, dialect, catalog, params, pidx, work)?,
            )?;
        }
        for opt in &b.opts {
            from.push_str(" LEFT JOIN ");
            let scan = scan::template::restrict_optional_controlled(opt, dialect, catalog, work)?;
            append(
                &mut from,
                scan_ref_controlled(&scan, dialect, catalog, params, pidx, work)?,
            )?;
            from.push_str(" ON ");
            let conds: Vec<&SqlCond> = opt.on.iter().chain(opt.extra.iter()).collect();
            append(
                &mut from,
                render_conjunction(&conds, dialect, catalog, actuals, params, pidx, work)?,
            )?;
        }
        for sp in &b.subplan_joins {
            let join_kw = if sp.left {
                " LEFT JOIN "
            } else {
                " INNER JOIN "
            };
            append(
                &mut from,
                emit_subplan_join_controlled(sp, dialect, catalog, params, pidx, join_kw, work)
                    .await?,
            )?;
            if !sp.on.is_empty() {
                from.push_str(" ON ");
                let conds: Vec<&SqlCond> = sp.on.iter().collect();
                append(
                    &mut from,
                    render_conjunction(&conds, dialect, catalog, actuals, params, pidx, work)?,
                )?;
            } else {
                from.push_str(" ON 1 = 1");
            }
        }
        from
    };
    Ok(from)
}

pub(super) async fn emit_subplan_join_controlled(
    sp: &crate::iq::SubPlanJoin,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    params: &mut Vec<String>,
    pidx: &mut usize,
    join_kw: &str,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    let (nested_sql, nested_params) =
        emit_subplan_sql_async_controlled(&sp.plan, dialect, catalog, work).await?;
    let rebased = rebase_placeholders_controlled(&nested_sql, dialect, *pidx, work)?;
    work.append_parameters(params, pidx, nested_params)
        .map_err(source_control::validation_error)?;
    work.charge(join_kw.len() + rebased.len() + 24)
        .map_err(source_control::validation_error)?;
    Ok(format!("{join_kw}({rebased}) t{}", sp.alias))
}

pub(super) fn render_where(
    conds: &[SqlCond],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Option<String>> {
    work.charge(1).map_err(source_control::validation_error)?;
    if conds.is_empty() {
        return Ok(None);
    }
    let mut refs = work
        .vector(conds.len())
        .map_err(source_control::validation_error)?;
    refs.extend(conds.iter());
    Ok(Some(render_conjunction(
        &refs, dialect, catalog, actuals, params, pidx, work,
    )?))
}

pub(super) fn render_conjunction(
    conds: &[&SqlCond],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    condition_control::conjunction(conds, dialect, catalog, actuals, params, pidx, work)
}

#[cfg(test)]
pub(super) fn render_cond(
    cond: &SqlCond,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    render_cond_controlled(
        cond,
        dialect,
        catalog,
        actuals,
        params,
        pidx,
        sf_sql::source_work::SourceWork::new(None),
    )
}

pub(super) fn render_cond_controlled(
    cond: &SqlCond,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    condition_control::condition(cond, dialect, catalog, actuals, params, pidx, work)
}
