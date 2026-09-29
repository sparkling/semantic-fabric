//! Backend-generic pull-cursor execution and plan-modifier sequencing: reconstruct,
//! dedup, ORDER, then slice.

use std::future::Future;
use std::sync::Arc;

use sf_core::query_control::{
    QueryCharge, QueryControl, QueryControlError, UncontrolledQueryControl,
};
use sf_core::term_work::TermWork;
use sf_core::Term;
use sf_sql::{BranchStream, Dialect, RawTuple, SqlBackend};

use crate::emit::{self, ColumnCatalog};
use crate::iq::{Branch, OrderKey};
use crate::{DedupScope, Error, Plan, PlanForm, Result};

use super::batch::{reconstruct_batch, TERM_GEN_BATCH_SIZE, TERM_GEN_FIRST_BATCH_SIZE};
use super::cooperative_yield;
use super::expression::eval_expr;
use super::forms::rust_group_execute;
use super::order::{compact_to_window, sorted_indices};
use super::row::{build_col_index_controlled, canonical_pairs, Bindings};
use super::sql_error::map_sql_err;

#[path = "source_prepare.rs"]
pub(super) mod source_prepare;

fn accounting_overflow(control: &dyn QueryControl) -> Error {
    Error::QueryControl(control.terminate(QueryControlError::AccountingOverflow))
}

/// Evaluate ORDER expressions and inject synthetic bindings for the comparator
/// (design §2 — extracted from the old SQLite-only `exec.rs` path).
fn inject_order_expr_keys(order: &[OrderKey], bindings: Bindings) -> Bindings {
    if order.iter().any(|k| k.expr.is_some()) {
        let mut b = bindings;
        for key in order {
            if let Some(expr) = &key.expr {
                if let Some(val) = eval_expr(expr, &b) {
                    // ORDER expression keys are sparse; recipe names are interned separately.
                    b.insert(Arc::from(key.var.as_str()), val);
                }
            }
        }
        b
    } else {
        bindings
    }
}

/// Iterate every WHERE solution across all branches; dispatch the multi-branch
/// GROUP BY (`rust_group`) to the buffered path, else the streaming branches loop.
pub(super) async fn for_each_solution<B, F, Fut>(plan: &Plan, b: &mut B, sink: F) -> Result<()>
where
    B: SqlBackend,
    F: FnMut(&Branch, &Bindings) -> Result<Fut>,
    Fut: Future<Output = Result<()>>,
{
    for_each_solution_controlled(plan, b, &UncontrolledQueryControl, sink).await
}

pub(super) fn parallel_term_gen_for(plan: &Plan) -> bool {
    plan.order.is_empty()
}

/// Controlled sibling of [`for_each_solution`]. Production serving supplies one
/// request-scoped control; raw/conformance entry points use the explicit
/// [`UncontrolledQueryControl`] wrapper above.
pub(super) async fn for_each_solution_controlled<B, F, Fut>(
    plan: &Plan,
    b: &mut B,
    control: &dyn QueryControl,
    sink: F,
) -> Result<()>
where
    B: SqlBackend,
    F: FnMut(&Branch, &Bindings) -> Result<Fut>,
    Fut: Future<Output = Result<()>>,
{
    control.checkpoint()?;
    // LIMIT 0 is source-independent for every form, grouped plans included: it
    // answers empty before any source probe, SQL or deep-plan descent, so a
    // missing or failing source is not reported. Serving admission relies on
    // this order, and it keeps a modifier difference from cloning a deep
    // single-branch plan.
    if plan.limit == Some(0) {
        return Ok(());
    }
    if let Some(rg) = &plan.rust_group {
        return rust_group_execute(plan, b, rg, control, sink).await;
    }
    let ctx = PlanCtx {
        dialect: plan.dialect,
        distinct: plan.distinct,
        form: &plan.form,
        order: &plan.order,
        offset: plan.offset,
        limit: plan.limit,
        // ORDER stays sequential: repeated Rayon batches spread short-lived
        // allocations across system-allocator arenas and violate its RSS gate.
        parallel_term_gen: parallel_term_gen_for(plan),
        stop_after_first: matches!(plan.form, PlanForm::Ask),
        dedup_scopes: &plan.dedup_scopes,
        control,
    };
    run_branches(&plan.branches, ctx, b, sink).await
}
/// Scalar plan fields allow [`rust_group_execute`] to override inner modifiers
/// without cloning the whole plan before [`run_branches`].
pub(super) struct PlanCtx<'a> {
    pub(super) dialect: Dialect,
    pub(super) distinct: bool,
    pub(super) form: &'a PlanForm,
    pub(super) order: &'a [OrderKey],
    pub(super) offset: usize,
    pub(super) limit: Option<usize>,
    /// Whether [`reconstruct_batch`] may dispatch a large batch to rayon.
    pub(super) parallel_term_gen: bool,
    /// Stop after the first final sink call; false for aggregate inner collection.
    pub(super) stop_after_first: bool,
    /// ADR-0034 C0e restoration — post-cascade, executor-branch-aligned shared
    /// term-dedup scopes. Empty means no shared groups; otherwise its length
    /// must equal `branches.len()` and each slot corresponds to the same index.
    pub(super) dedup_scopes: &'a [Option<DedupScope>],
    /// One request-scoped governance identity. Raw APIs pass an explicit no-op.
    pub(super) control: &'a dyn QueryControl,
}

/// Borrowed branches plus scalar modifier overlays; grouped inner execution uses
/// the same loop without copying the recursive plan. Sinks consume bindings and
/// branch source identity, not the prepared DISTINCT/LIMIT/OFFSET fields.
pub(super) async fn run_branches<B, F, Fut>(
    branches: &[Branch],
    ctx: PlanCtx<'_>,
    b: &mut B,
    mut sink: F,
) -> Result<()>
where
    B: SqlBackend,
    F: FnMut(&Branch, &Bindings) -> Result<Fut>,
    Fut: Future<Output = Result<()>>,
{
    ctx.control.checkpoint()?;
    // LIMIT 0 is source-independent: return before metadata, cursor, or retention.
    if ctx.limit == Some(0) {
        return Ok(());
    }
    // The post-cascade lift normally guarantees alignment. Keep this boundary
    // fail-closed for hand-built/internal Plans before metadata probing or SQL
    // emission can perform I/O.
    let single = branches.len() == 1;
    let modifiers = |branch: &Branch| {
        emit::BranchModifiers::prepared(
            branch,
            single,
            ctx.distinct,
            ctx.order.is_empty(),
            ctx.limit,
            ctx.offset,
        )
    };
    let has_scopes = super::dedup_scope_runtime::validate_runtime_scopes_with_slice(
        branches,
        ctx.dedup_scopes,
        ctx.control,
        (single && ctx.order.is_empty()).then_some((ctx.limit, ctx.offset)),
    )?;
    // Prove collisions before borrowing hidden keys; never copy cached branch forests.
    if has_scopes {
        source_prepare::requires_key_overlay(
            branches,
            ctx.dedup_scopes,
            sf_sql::source_work::SourceWork::new(Some(ctx.control)),
        )?;
    }
    // Preflight all tagged Table/Query identities before any branch opens.
    let mut catalog = ColumnCatalog::default();
    let mut seen_sources = emit::SourceSet::default();
    for source in
        emit::live_metadata_sources_controlled(branches, ctx.control).map_err(map_sql_err)?
    {
        if !seen_sources
            .insert(source, ctx.control)
            .map_err(map_sql_err)?
        {
            continue;
        }
        let probe =
            emit::source_probe_controlled(source, ctx.dialect, ctx.control).map_err(map_sql_err)?;
        ctx.control.consume(QueryCharge::SourceWork, 1)?;
        let columns = b
            .result_columns_controlled(&probe, ctx.control)
            .await
            .map_err(map_sql_err)?;
        catalog
            .insert_live_result_controlled(source, columns, ctx.control)
            .map_err(map_sql_err)?;
    }
    ctx.control.checkpoint()?;
    emit::validate_execution_columns(
        branches,
        ctx.dedup_scopes,
        ctx.dialect,
        &catalog,
        sf_sql::source_work::SourceWork::new(Some(ctx.control)),
    )?;
    // Emission is part of the same preflight. A malformed later branch must fail
    // before an earlier branch can open a cursor or expose a partial result.
    // An explicit loop (not `.map().collect()`) so each branch's emission is
    // genuinely awaited: when it reaches the isolated SQL-canonicalization
    // peer, that peer's bounded child-I/O steps cooperatively yield here
    // instead of blocking this task's own thread, so unrelated concurrent
    // requests keep making progress on the same executor.
    let mut emitted_branches = Vec::with_capacity(branches.len());
    for (index, branch) in branches.iter().enumerate() {
        let work = sf_sql::source_work::SourceWork::new(Some(ctx.control));
        let bindings = emit::BindingView::merged(
            &branch.bindings,
            ctx.dedup_scopes
                .get(index)
                .and_then(Option::as_ref)
                .map(|s| &s.key_bindings),
            work,
        )?;
        let emitted = emit::emit_branch_binding_view(
            branch,
            &bindings,
            ctx.dialect,
            &catalog,
            modifiers(branch),
            work,
        )
        .await?;
        emitted_branches.push(emitted);
    }
    let multi = branches.len() > 1;
    // Cross-branch DISTINCT precedes OFFSET/LIMIT; SQL only dedups within branches.
    let distinct_vars: Option<&[String]> = match (ctx.distinct && multi, ctx.form) {
        (true, PlanForm::Select { vars }) => Some(vars),
        _ => None,
    };
    let mut seen_tuples: std::collections::HashSet<Vec<Option<Term>>> =
        std::collections::HashSet::new();
    // ADR-0034: each shared group owns one seen-set across all its branches;
    // this preserves pooled UNION deduplication without emitting that UNION.
    let mut group_seen: std::collections::HashMap<
        usize,
        std::collections::HashSet<Vec<Option<Term>>>,
    > = std::collections::HashMap::new();
    let mut seen = 0usize; // solutions observed (for offset)
    let mut emitted = 0usize; // solutions passed downstream (for limit)
                              // ORDER stays here, not under a source collation; ASK needs no buffer.
    let ordered = !ctx.order.is_empty() && !ctx.stop_after_first;
    // Rows are demand-driven only where this loop may stop before EOF: ASK's
    // first solution, or a LIMIT applied here across unordered branches rather
    // than in SQL. Full scans and ordered plans keep the backend's prefetch.
    let early_stop = ctx.stop_after_first || (multi && !ordered && ctx.limit.is_some());
    let order_window = crate::resource_profile::retained_order_window(ctx.offset, ctx.limit);
    let mut buffer: Vec<(usize, Bindings)> = Vec::new();
    let mut retained_payload = 0_u64;
    let mut charged_payload_peak = 0_u64;
    for (bi, (branch, e)) in branches.iter().zip(&emitted_branches).enumerate() {
        // ADR-0034: effective DISTINCT may defer non-injective raw rows to a
        // fresh per-branch full-term set. A shared scope instead dedups against
        // other arms with the same group id, before outer projection/slicing.
        let group_scope = ctx.dedup_scopes.get(bi).and_then(Option::as_ref);
        let term_dedup = group_scope.is_some()
            || crate::cascade::eligible_for_term_dedup_with_distinct(
                branch,
                modifiers(branch).distinct,
            );
        let mut own_term_seen: std::collections::HashSet<Vec<Term>> =
            std::collections::HashSet::new();
        // Build the fixed column index once per stream (ADR-0024/M4).
        let col_index = build_col_index_controlled(
            &e.projection,
            sf_sql::source_work::SourceWork::new(Some(ctx.control)),
        )?;
        let bindings = emit::BindingView::merged(
            &branch.bindings,
            group_scope.map(|scope| &scope.key_bindings),
            sf_sql::source_work::SourceWork::new(Some(ctx.control)),
        )?;
        let interned = super::row::intern_binding_view(
            &bindings,
            sf_sql::source_work::SourceWork::new(Some(ctx.control)),
        )?;
        // Parameters bind once, in their emitted positional order.
        ctx.control.consume(QueryCharge::SourceWork, 1)?;
        let mut s = b
            .open_branch_with_demand(
                &e.sql,
                &e.params,
                e.metadata_sql.as_deref(),
                e.sqlite_character_keys,
                e.sqlite_lexical_keys,
                early_stop,
            )
            .await
            .map_err(map_sql_err)?;
        // Buffer -> term-gen (parallel only when `ctx.parallel_term_gen`, see
        // `reconstruct_batch`) -> emit-in-order (ADR-0006 M4 wave-2 batch
        // restructure): pull a bounded batch of raw rows off the cursor,
        // reconstruct their bound terms, then run the SAME per-row DISTINCT /
        // ORDER BY / OFFSET/LIMIT / sink logic sequentially over the batch, in the
        // original row order — so this is behaviorally identical to the old
        // one-row-at-a-time loop, just with term-gen's CPU work batched. The
        // batch-and-reconstruct-as-a-unit SHAPE stays the same regardless of
        // `parallel_term_gen` (ledger F8 measured this indirection alone costs
        // ~nothing); the first batch uses `TERM_GEN_FIRST_BATCH_SIZE` so many rows
        // still yields its first result quickly (the streaming invariant), then
        // grows to the full `TERM_GEN_BATCH_SIZE` for throughput.
        let mut first_batch = true;
        loop {
            let mut target = if ctx.stop_after_first {
                1
            } else if first_batch {
                TERM_GEN_FIRST_BATCH_SIZE
            } else {
                TERM_GEN_BATCH_SIZE
            };
            // A row yields at most one solution: pull no more than OFFSET and LIMIT still need.
            if let (true, Some(limit)) = (early_stop, ctx.limit) {
                let needed = ctx.offset.saturating_sub(seen);
                target = target.min(needed.saturating_add(limit.saturating_sub(emitted)));
            }
            let mut raw_batch: Vec<RawTuple> = Vec::with_capacity(target);
            while raw_batch.len() < target {
                // Charge the observable pull attempt before source I/O. The final
                // EOF attempt is work too, and a zero budget therefore rejects
                // before metadata/open/pull can touch the source.
                ctx.control.consume(QueryCharge::SourceWork, 1)?;
                match s
                    .next_row_controlled(ctx.control)
                    .await
                    .map_err(map_sql_err)?
                {
                    Some(t) => raw_batch.push(t),
                    None => break,
                }
            }
            if raw_batch.is_empty() {
                break;
            }
            // Pull-side cooperative checkpoint for the serve lane's outer absolute
            // deadline: an always-ready cursor whose rows are later discarded might
            // otherwise never reach the sink (and never return `Pending`). This is
            // one checkpoint per bounded batch, not source cancellation or CPU
            // pre-emption within reconstruction of that batch.
            cooperative_yield().await;
            ctx.control.checkpoint()?;
            let exhausted = raw_batch.len() < target;
            first_batch = false;
            // Reconstruct before DISTINCT and slicing; release raw values before
            // the sink. Peak memory remains inside reconstruction, where both raw
            // and reconstructed batches coexist (see Bindings/TERM_GEN_BATCH_SIZE).
            // Term generation is charged per row against the same request
            // control that admitted the source pull, so a batch cannot spend
            // unbounded CPU/allocation past its budget, and a terminated
            // request stops within the batch rather than at its end.
            let reconstructed = reconstruct_batch(
                &interned,
                &raw_batch,
                &col_index,
                ctx.parallel_term_gen,
                TermWork::new(Some(ctx.control)),
            );
            drop(raw_batch);
            for bindings in reconstructed {
                let bindings = bindings?;
                if multi {
                    if let Some(vars) = &distinct_vars {
                        let key: Vec<Option<Term>> =
                            vars.iter().map(|v| bindings.get(v).cloned()).collect();
                        if !seen_tuples.insert(key) {
                            continue; // duplicate projected solution
                        }
                    }
                }
                if term_dedup {
                    // Run 4 Wave C1: `Bindings` preserves INSERTION order, not
                    // the old `BTreeMap`'s alphabetical-by-var-name order —
                    // canonicalize via `canonical_pairs` so two equal solutions
                    // whose vars got bound in a different sequence still hash
                    // the same (see `Bindings`'s doc comment).
                    let inserted = match group_scope {
                        Some(scope) => {
                            let key = scope
                                .key_bindings
                                .keys()
                                .map(|variable| bindings.get(variable).cloned())
                                .collect();
                            group_seen.entry(scope.group_id).or_default().insert(key)
                        }
                        None => {
                            let key: Vec<Term> = canonical_pairs(&bindings)
                                .into_iter()
                                .map(|(_, value)| value.clone())
                                .collect();
                            own_term_seen.insert(key)
                        }
                    };
                    if !inserted {
                        // duplicate reconstructed solution (ADR-0034 D1 term dedup,
                        // shared cross-branch when `group_id` is set — C0e restoration)
                        continue;
                    }
                }
                // ORDER BY (any branch count): defer slicing — buffer for the global
                // type-aware sort after every row (OFFSET/LIMIT applied after the sort).
                if ordered {
                    let bindings = inject_order_expr_keys(ctx.order, bindings);
                    let payload = bindings
                        .retained_payload_bytes()
                        .ok_or_else(|| accounting_overflow(ctx.control))?;
                    retained_payload = retained_payload
                        .checked_add(payload)
                        .ok_or_else(|| accounting_overflow(ctx.control))?;
                    if retained_payload > charged_payload_peak {
                        ctx.control.consume(
                            QueryCharge::RetainedBytes,
                            retained_payload - charged_payload_peak,
                        )?;
                        charged_payload_peak = retained_payload;
                    }
                    buffer.push((bi, bindings));
                    continue;
                }
                // Streaming OFFSET/LIMIT only when SQL didn't apply them (a multi-branch
                // bag-union; a single unordered branch sliced in SQL).
                if multi || (ctx.stop_after_first && !ctx.order.is_empty()) {
                    if seen < ctx.offset {
                        seen += 1;
                        continue;
                    }
                }
                emitted += 1;
                sink(branch, &bindings)?.await?;
                if ctx.stop_after_first {
                    return Ok(());
                }
                if multi && ctx.limit.is_some_and(|limit| emitted >= limit) {
                    return Ok(());
                }
            }
            if let (true, Some(window)) = (ordered, order_window) {
                if buffer.len() > window {
                    ctx.control.checkpoint()?;
                    compact_to_window(&mut buffer, ctx.order, window);
                    retained_payload = buffer
                        .iter()
                        .try_fold(0_u64, |total, (_, bindings)| {
                            total.checked_add(bindings.retained_payload_bytes()?)
                        })
                        .ok_or_else(|| accounting_overflow(ctx.control))?;
                }
            }
            if exhausted {
                break;
            }
        }
    }
    // The buffered bag-union ORDER BY: stable-sort by the keys, then OFFSET/LIMIT.
    // Schwartzian transform (ADR-0024/M4 perf): precompute each row's sort keys
    // ONCE — the O(n log n)-comparison sort then looks them up instead of
    // re-deriving `cmp_term`'s (possibly-allocating) fallback string from the
    // bound `Term`s on every comparison. Sorting INDICES (not `buffer` itself)
    // keeps the precomputed keys' borrow of `buffer` and the final read of
    // `buffer` both immutable, and preserves `sort_by`'s stability identically to
    // sorting `buffer` directly (the indices start in `buffer`'s original order).
    if ordered {
        ctx.control.checkpoint()?;
        let idx = sorted_indices(&buffer, ctx.order);
        let take = ctx.limit.unwrap_or(usize::MAX);
        for &i in idx.iter().skip(ctx.offset).take(take) {
            let (bi, bindings) = &buffer[i];
            sink(&branches[*bi], bindings)?.await?;
            if ctx.stop_after_first {
                return Ok(());
            }
        }
    }
    Ok(())
}
