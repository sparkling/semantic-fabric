//! Raw source enumeration and the offline synthetic SubPlan catalog.
use super::*;

/// Every live logical source nested in `branches`, in deterministic encounter
/// order. The executor uses a paid, variant-tagged [`SourceSet`] before probing;
/// keeping enumeration and identity separate makes the fail-closed ordering
/// explicit at the I/O boundary. This raw traversal is the semantic oracle.
pub(crate) fn live_metadata_sources(branches: &[Branch]) -> Vec<&LogicalSource> {
    fn hop_sources<'a>(hop: &'a HopExpr, out: &mut Vec<&'a LogicalSource>) {
        match hop {
            HopExpr::Pred(relation) => out.push(&relation.source),
            HopExpr::Inverse(inner) => hop_sources(inner, out),
            HopExpr::Seq(left, right) => {
                hop_sources(left, out);
                hop_sources(right, out);
            }
            HopExpr::Alt(parts) | HopExpr::Nps(parts) => {
                for part in parts {
                    hop_sources(part, out);
                }
            }
        }
    }

    fn condition_sources<'a>(condition: &'a SqlCond, out: &mut Vec<&'a LogicalSource>) {
        match condition {
            SqlCond::Not(inner) => condition_sources(inner, out),
            SqlCond::And(parts) | SqlCond::Or(parts) => {
                for part in parts {
                    condition_sources(part, out);
                }
            }
            SqlCond::NotExists { scans, conds } | SqlCond::Exists { scans, conds } => {
                for scan in scans {
                    scan_sources(scan, out);
                }
                for condition in conds {
                    condition_sources(condition, out);
                }
            }
            SqlCond::PathExists { pc, conds, .. } => {
                hop_sources(&pc.hop, out);
                for condition in conds {
                    condition_sources(condition, out);
                }
            }
            _ => {}
        }
    }

    fn branch_sources<'a>(branch: &'a Branch, out: &mut Vec<&'a LogicalSource>) {
        for scan in branch
            .core
            .iter()
            .chain(branch.opts.iter().map(|join| &join.scan))
        {
            scan_sources(scan, out);
        }
        if let Some(path) = &branch.path {
            hop_sources(&path.hop, out);
        }
        for condition in &branch.where_conds {
            condition_sources(condition, out);
        }
        for join in &branch.opts {
            for condition in join.on.iter().chain(join.extra.iter()) {
                condition_sources(condition, out);
            }
        }
        for subplan in &branch.subplan_joins {
            for condition in &subplan.on {
                condition_sources(condition, out);
            }
            for inner in &subplan.plan.branches {
                branch_sources(inner, out);
            }
        }
    }

    fn scan_sources<'a>(scan: &'a crate::iq::Scan, out: &mut Vec<&'a LogicalSource>) {
        match &scan.source {
            crate::iq::ScanSource::Logical(source) => out.push(source),
            crate::iq::ScanSource::Path { closure, .. } => hop_sources(&closure.hop, out),
            crate::iq::ScanSource::Projection { input, .. } => scan_sources(input, out),
            crate::iq::ScanSource::RefAtom { input, .. } => branch_sources(input, out),
        }
    }

    let mut sources = Vec::new();
    for branch in branches {
        branch_sources(branch, &mut sources);
    }
    sources
}

/// A translate-time [`ColumnCatalog`] for offline SubPlan embedding
/// ([`crate::Plan::emitted`]). Offline derived-table SQL has no live DB connection,
/// so [`colref`]'s `actuals` lookup otherwise falls back to quoting every identifier
/// exact-case.
/// That is correct for a genuinely delimited column, but WRONG for a *regular*
/// (bare) `rr:sqlQuery` output alias: PostgreSQL folds it to lowercase at
/// declaration, so an exact-case-quoted reference to it does not exist (W3C
/// R2RMLTC0014b — `SELECT (…) AS jobTypeURI` folds to `jobtypeuri`; a downstream
/// `t5."jobTypeURI"` 42703s; confirmed live against PostgreSQL 17).
///
/// A typed walk identifies the referenced columns, then
/// [`crate::cascade::col_is_unquoted_alias`] lexically scans the owning
/// `rr:sqlQuery` for an explicit bare `AS` alias and seeds its folded name. That
/// bounded heuristic is not SQL-token/comment/string-literal aware and is therefore
/// non-authoritative. Live execution does not rely on it for base sources:
/// [`emit_subplan_sql`] overlays the recursively probed live catalog before nested
/// emission. A `LogicalSource::Table` has no alias declaration to inspect and is
/// left alone.
pub(crate) fn synthetic_subplan_catalog(branches: &[Branch]) -> ColumnCatalog {
    synthetic_subplan_catalog_controlled(branches, sf_sql::source_work::SourceWork::new(None))
        .expect("uncontrolled synthetic catalog cannot be refused")
}

pub(super) fn synthetic_subplan_catalog_controlled(
    branches: &[Branch],
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<ColumnCatalog> {
    let mut catalog = ColumnCatalog::default();
    for b in branches {
        work.charge(1 + b.core.len() + b.opts.len() + b.subplan_joins.len())
            .map_err(source_control::validation_error)?;
        let sources: HashMap<usize, &LogicalSource> = b.alias_sources().into_iter().collect();
        let mut cols: Vec<ColRef> = Vec::new();
        let mut refused = None;
        // Pay each column reference before copying it; a refusal stops the walk.
        let mut push = |c: &ColRef| {
            if refused.is_none() {
                match work.charge(1 + c.column.len()) {
                    Ok(()) => cols.push(c.clone()),
                    Err(error) => refused = Some(error),
                }
            }
        };
        for def in b.bindings.values() {
            for c in def.columns() {
                push(&c);
            }
        }
        for cond in &b.where_conds {
            collect_cond_cols(cond, &mut push);
        }
        for opt in &b.opts {
            for cond in opt.on.iter().chain(opt.extra.iter()) {
                collect_cond_cols(cond, &mut push);
            }
        }
        for sp in &b.subplan_joins {
            for cond in &sp.on {
                collect_cond_cols(cond, &mut push);
            }
        }
        if let Some(error) = refused {
            return Err(source_control::validation_error(error));
        }
        let mut seen: std::collections::HashSet<(usize, &str)> = std::collections::HashSet::new();
        let mut merged: HashMap<usize, usize> = HashMap::new();
        for c in &cols {
            let Some(LogicalSource::Query(sql)) = sources.get(&c.alias) else {
                continue;
            };
            // Each distinct column reference scans the query text once.
            work.charge(1 + c.column.len())
                .map_err(source_control::validation_error)?;
            if !seen.insert((c.alias, &c.column)) {
                continue;
            }
            // Lowercase copy and alias scans of the query text, the key copy,
            // the lowercased column and the membership scan over the columns
            // already merged for this alias.
            work.product(sql.len(), 4)
                .map_err(source_control::validation_error)?;
            work.charge(1 + c.column.len() + merged.get(&c.alias).copied().unwrap_or(0))
                .map_err(source_control::validation_error)?;
            if crate::cascade::col_is_unquoted_alias(sql, &c.column) {
                catalog.merge(sources[&c.alias], c.column.to_lowercase());
                *merged.entry(c.alias).or_insert(0) += 1;
            }
        }
    }
    Ok(catalog)
}
