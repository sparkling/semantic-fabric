//! Public SELECT, ASK, CONSTRUCT, and mapping-dump entry points.

/// Multi-branch GROUP BY: collect every inner solution (no DISTINCT/OFFSET/LIMIT/
/// ORDER on the inner — those apply AFTER grouping), then group + aggregate + slice
/// via the backend-independent [`rust_group_result_rows`] and stream the grouped
/// rows. Drives [`run_branches`] directly (see its doc comment) instead of
/// wrapping the inner modifiers into a freshly-cloned `Plan`.
pub(super) async fn rust_group_execute<B, F, Fut>(
    plan: &Plan,
    b: &mut B,
    rg: &RustGroup,
    mut sink: F,
) -> Result<()>
where
    B: SqlBackend,
    F: FnMut(&Branch, &Bindings) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    // The inner collection's "prepared branches" — `Plan::prepared_branches`'s
    // single-branch push-down (distinct/limit/offset onto branches[0]), specialised
    // to the KNOWN inner-modifier values (no DISTINCT/OFFSET/LIMIT/ORDER: those
    // apply AFTER grouping) so `plan.branches` is cloned exactly ONCE here, not
    // once here and once more inside `prepared_branches`.
    let mut inner_branches = plan.branches.clone();
    if inner_branches.len() == 1 {
        let branch = &mut inner_branches[0];
        branch.distinct = false;
        branch.limit = None;
        branch.offset = 0;
    }
    let inner_ctx = PlanCtx {
        dialect: plan.dialect,
        distinct: false,
        form: &plan.form,
        order: &[],
        offset: 0,
        limit: None,
        // Unlike the plain streaming path, this inner collection ALWAYS fully
        // materializes every row into `inner_rows` below before grouping can even
        // start — there is no streaming-to-a-live-sink downside to amortize a
        // rayon dispatch against, and this is the shape (aggregate-heavy,
        // `canonical_lexical` numeric formatting) that measurably benefits from
        // it (ledger F8 / `micro_distinct_agg`, `micro_group_avg_rust`).
        parallel_term_gen: true,
        dedup_groups: &plan.dedup_groups,
    };
    let mut inner_rows: Vec<Bindings> = Vec::new();
    run_branches(&inner_branches, inner_ctx, b, |_, bindings| {
        inner_rows.push(bindings.clone());
        std::future::ready(Ok(()))
    })
    .await?;

    let dummy = plan.branches.first().cloned().unwrap_or_else(Branch::empty);
    for result in rust_group_result_rows(plan, rg, inner_rows)? {
        sink(&dummy, &result).await?;
    }
    Ok(())
}

// --- per-form entry points (generic over B) ----------------------------------

/// Execute a SELECT, collecting solutions.
pub async fn select<B: SqlBackend>(plan: &Plan, b: &mut B) -> Result<Solutions> {
    let vars = match &plan.form {
        PlanForm::Select { vars } => vars.clone(),
        _ => {
            return Err(Error::Unsupported(
                "select() requires a SELECT plan".to_owned(),
            ))
        }
    };
    let mut rows = Vec::new();
    for_each_solution(plan, b, |_branch, bindings| {
        rows.push(vars.iter().map(|v| bindings.get(v).cloned()).collect());
        std::future::ready(Ok(()))
    })
    .await?;
    Ok(Solutions { vars, rows })
}

/// Stream a SELECT's solutions, invoking `sink` per projected row (one row in flight).
pub async fn select_each<B, S>(plan: &Plan, b: &mut B, mut sink: S) -> Result<()>
where
    B: SqlBackend,
    S: FnMut(&[Option<Term>]) -> Result<()>,
{
    let vars = match &plan.form {
        PlanForm::Select { vars } => vars.clone(),
        _ => {
            return Err(Error::Unsupported(
                "select() requires a SELECT plan".to_owned(),
            ))
        }
    };
    let mut row: Vec<Option<Term>> = Vec::with_capacity(vars.len());
    for_each_solution(plan, b, |_branch, bindings| {
        row.clear();
        row.extend(vars.iter().map(|v| bindings.get(v).cloned()));
        std::future::ready(sink(&row))
    })
    .await
}

/// Stream a SELECT's solutions into an ASYNC sink (per projected row, plan order,
/// `None` = unbound). The PostgreSQL/MySQL serve-lane form: `sink(..).await`
/// backpressures the server-side cursor (ADR-0006 / ADR-0010 §C). SQLite's serve
/// lane keeps the sync [`select_each`]. Written once over the shared core, so it
/// inherits rust_group / DISTINCT / ORDER / OFFSET / LIMIT.
pub async fn select_each_async<B, F, Fut>(plan: &Plan, b: &mut B, mut sink: F) -> Result<()>
where
    B: SqlBackend + Send,
    for<'s> B::Stream<'s>: Send,
    F: FnMut(Vec<Option<Term>>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let vars = match &plan.form {
        PlanForm::Select { vars } => vars.clone(),
        _ => {
            return Err(Error::Unsupported(
                "select() requires a SELECT plan".to_owned(),
            ))
        }
    };
    for_each_solution(plan, b, |_branch, bindings| {
        let row: Vec<Option<Term>> = vars.iter().map(|v| bindings.get(v).cloned()).collect();
        sink(row)
    })
    .await
}

/// ADR-0034 item 3 (Run 5) — whether `lib.rs`'s per-branch template-projection
/// dedup (`dedup_construct_template_projected_vars`) cannot see far enough to
/// answer §16.2's "CONSTRUCT output is a SET" on its own: MULTIPLE branches
/// exist (that pass pushes a SQL-level `DISTINCT` into ONE branch's own
/// `SELECT` — it can never see a SIBLING branch, e.g. a UNION arm over a
/// DIFFERENT triple pattern, instantiating the identical triple) AND
/// `plan.construct_drops_some_branch_var` — captured by that SAME pass, from
/// each branch's ORIGINAL bindings, before its own narrowing loop overwrites
/// them to match the template exactly (a narrowed branch is afterward
/// indistinguishable from one that never bound anything extra, so this CANNOT
/// be recomputed here from `plan.branches` alone — found the hard way: an
/// earlier version of this function recomputed it from the post-narrowing
/// bindings and both never fired, per the `s7b`-shaped case, and — the more
/// dangerous direction — a naive `branches.len() > 1` shortcut with no drop
/// check at all over-fired on an ordinary multi-TriplesMap `?s ?p ?o` dump,
/// where the template keeps every var every branch binds: `sf-bench`'s
/// `engine_memory_is_bounded_under_growing_source`/`_pg` measured that
/// regression to LINEAR memory growth, `mem_ratio` 13.72x at 16x scale,
/// before it could land). "Nothing dropped anywhere" is safe regardless of
/// branch count: when the template keeps every bound variable, two branches
/// instantiating the identical triple is exactly two branches producing the
/// identical WHERE solution — D2's own cross-branch mechanism (`unfold::
/// pool_pattern_relation` / `iq::resolve`'s Intensional arm: provable
/// disjointness, SQL pooling, or the C0e shared seen-set) already resolves
/// that BEFORE the template ever sees it, which is why BRANCH COUNT ALONE
/// must never be the gate either. `false` is the fast path and MUST stay
/// untouched — it is the unbounded `?s ?p ?o`-shaped dump case ADR-0006's
/// constant-memory invariant exists for. Where `true`, [`construct`]/
/// [`construct_each_async`] dedup the PRODUCED triples with a Rust-side
/// `HashSet` — bounded by DISTINCT OUTPUT triples, not total input rows, the
/// same documented trade `cascade::eligible_for_term_dedup`'s single-branch
/// term dedup already makes.
pub(crate) fn construct_may_need_cross_branch_dedup(plan: &Plan) -> bool {
    plan.branches.len() > 1 && plan.construct_drops_some_branch_var
}

/// Stream a CONSTRUCT's per-solution triples into an ASYNC sink (bounded by the
/// template size — never the whole graph). The PostgreSQL/MySQL serve-lane form of
/// [`construct`], written once over the shared core.
pub async fn construct_each_async<B, F, Fut>(plan: &Plan, b: &mut B, mut sink: F) -> Result<()>
where
    B: SqlBackend + Send,
    for<'s> B::Stream<'s>: Send,
    F: FnMut(Vec<Triple>) -> Fut + Send,
    Fut: Future<Output = Result<()>> + Send,
{
    let template = match &plan.form {
        PlanForm::Construct { template } => template.clone(),
        _ => {
            return Err(Error::Unsupported(
                "construct() requires a CONSTRUCT plan".to_owned(),
            ))
        }
    };
    let mut seen: Option<std::collections::HashSet<Triple>> =
        construct_may_need_cross_branch_dedup(plan).then(std::collections::HashSet::new);
    // Run 5 W6 fix: one id per SOLUTION (not per triple) — every triple
    // pattern instantiated below for the SAME `bindings` shares this SAME id,
    // so a template blank node label repeated across the template's triples
    // stays one identity within a solution, while the NEXT solution advances
    // it, freshening (`instantiate_term`'s `TermPattern::BlankNode` arm).
    let mut solution_id: u64 = 0;
    for_each_solution(plan, b, |_branch, bindings| {
        let sid = solution_id;
        solution_id += 1;
        let mut triples: Vec<Triple> = template
            .iter()
            .filter_map(|tp| instantiate(tp, bindings, sid))
            .collect();
        if let Some(seen) = &mut seen {
            triples.retain(|t| seen.insert(t.clone()));
        }
        sink(triples)
    })
    .await
}

/// Execute an ASK — true iff at least one solution exists.
pub async fn ask<B: SqlBackend>(plan: &Plan, b: &mut B) -> Result<bool> {
    let mut any = false;
    for_each_solution(plan, b, |_b, _s| {
        any = true;
        std::future::ready(Ok(()))
    })
    .await?;
    Ok(any)
}

/// Stream the triples of a CONSTRUCT (or the `?s ?p ?o` dump), invoking `sink` per
/// well-formed triple; ill-formed instantiations are skipped. Returns the count.
pub async fn construct<B, S>(plan: &Plan, b: &mut B, mut sink: S) -> Result<u64>
where
    B: SqlBackend,
    S: FnMut(Triple),
{
    let template = match &plan.form {
        PlanForm::Construct { template } => template.clone(),
        _ => {
            return Err(Error::Unsupported(
                "construct() requires a CONSTRUCT plan".to_owned(),
            ))
        }
    };
    let mut seen: Option<std::collections::HashSet<Triple>> =
        construct_may_need_cross_branch_dedup(plan).then(std::collections::HashSet::new);
    let mut count = 0u64;
    // Run 5 W6 fix: see the matching comment in `construct_each_async` — one
    // id per SOLUTION, shared by every triple pattern instantiated below for
    // that SAME solution.
    let mut solution_id: u64 = 0;
    for_each_solution(plan, b, |_branch, bindings| {
        let sid = solution_id;
        solution_id += 1;
        for tp in &template {
            if let Some(triple) = instantiate(tp, bindings, sid) {
                if let Some(seen) = &mut seen {
                    if !seen.insert(triple.clone()) {
                        continue;
                    }
                }
                count += 1;
                sink(triple);
            }
        }
        std::future::ready(Ok(()))
    })
    .await?;
    Ok(count)
}

/// Collect a CONSTRUCT's triples (test/diagnostic convenience).
pub async fn construct_triples<B: SqlBackend>(plan: &Plan, b: &mut B) -> Result<Vec<Triple>> {
    let mut out = Vec::new();
    construct(plan, b, |t| out.push(t)).await?;
    Ok(out)
}

/// Stream the whole mapping as **quads** (ADR-0005), invoking `sink` per well-formed
/// quad — each triple carries the graph term from the applicable `rr:graphMap`(s).
pub async fn dump_quads_stream<B, S>(
    maps: &[sf_core::ir::TriplesMap],
    b: &mut B,
    dialect: Dialect,
    mut sink: S,
) -> Result<()>
where
    B: SqlBackend,
    S: FnMut(sf_core::Quad),
{
    use crate::dump::{VAR_G, VAR_O, VAR_P, VAR_S};
    use sf_core::GraphName;

    let plan = Plan {
        branches: crate::dump::build_branches(maps),
        form: PlanForm::Select { vars: Vec::new() },
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        rust_group: None,
        dialect,
        dedup_groups: std::collections::HashMap::new(),
        construct_drops_some_branch_var: false,
    };
    for_each_solution(&plan, b, |branch, bindings| {
        let quad = (|| {
            let (Some(s), Some(p), Some(o)) = (
                bindings.get(VAR_S),
                bindings.get(VAR_P),
                bindings.get(VAR_O),
            ) else {
                return None; // a NULL s/p/o column ⇒ no term ⇒ no triple (§11)
            };
            let graph = if branch.bindings.contains_key(VAR_G) {
                match bindings.get(VAR_G) {
                    // A generated graph IRI equal to the reserved `rr:defaultGraph`
                    // constant means the default graph, even when produced by a
                    // row-dependent (column/template) graph map (R2RML §6.1) — the
                    // well-known IRI is checked by VALUE, not by how it was produced.
                    Some(Term::NamedNode(n))
                        if n.as_str() == crate::graph_map::RR_DEFAULT_GRAPH =>
                    {
                        GraphName::DefaultGraph
                    }
                    Some(Term::NamedNode(n)) => GraphName::NamedNode(n.clone()),
                    _ => return None, // graph map yielded no value ⇒ drop this quad
                }
            } else {
                GraphName::DefaultGraph
            };
            Triple::from_terms(s.clone(), p.clone(), o.clone())
                .ok()
                .map(|t| t.in_graph(graph))
        })();
        if let Some(q) = quad {
            sink(q);
        }
        std::future::ready(Ok(()))
    })
    .await
}

/// Collect the mapping-IR quad dump (conformance convenience).
pub async fn dump_quads<B: SqlBackend>(
    maps: &[sf_core::ir::TriplesMap],
    b: &mut B,
    dialect: Dialect,
) -> Result<Vec<sf_core::Quad>> {
    let mut out = Vec::new();
    dump_quads_stream(maps, b, dialect, |q| out.push(q)).await?;
    Ok(out)
}
/// A SELECT solution: the projected variables (plan order) paired with each
/// row's bound terms (`None` = unbound).
pub struct Solutions {
    pub vars: Vec<String>,
    pub rows: Vec<Vec<Option<Term>>>,
}
use std::future::Future;

use sf_core::{Term, Triple};
use sf_sql::{Dialect, SqlBackend};

use crate::iq::{Branch, RustGroup};
use crate::{Error, Plan, PlanForm, Result};

use super::aggregation::rust_group_result_rows;
use super::driver::{for_each_solution, run_branches, PlanCtx};
use super::row::Bindings;
use super::template::instantiate;
