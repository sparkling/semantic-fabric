//! Backend-generic pull-cursor execution and plan-modifier sequencing: reconstruct,
//! dedup, ORDER, then slice.

use std::future::Future;
use std::sync::Arc;

use sf_core::query_control::{
    QueryCharge, QueryControl, QueryControlError, UncontrolledQueryControl,
};
use sf_core::Term;
use sf_sql::{BranchStream, Dialect, RawTuple, SqlBackend};

use crate::emit::{self, ColumnCatalog};
use crate::iq::{Branch, OrderKey};
use crate::{DedupScope, Error, Plan, PlanForm, Result};

use super::batch::{reconstruct_batch, TERM_GEN_BATCH_SIZE, TERM_GEN_FIRST_BATCH_SIZE};
use super::expression::eval_expr;
use super::forms::rust_group_execute;
use super::order::{compact_to_window, sorted_indices};
use super::row::{build_col_index, canonical_pairs, intern_bindings, Bindings};
use super::sql_error::map_sql_err;

/// Drive an always-ready future to completion with no runtime (design §5 M2).
/// SQLite's cooperative `Pending` checkpoint is immediately re-polled here.
pub(crate) fn block_on<F: Future>(fut: F) -> F::Output {
    use std::task::{Context, Poll, Waker};
    let mut cx = Context::from_waker(Waker::noop());
    let mut fut = std::pin::pin!(fut);
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

/// Yield once without tying this driver-agnostic crate to an async runtime.
///
/// Tokio-backed callers regain control at the next pull checkpoint, while the
/// SQLite `block_on` shim simply polls again. This is cooperative scheduling;
/// it does not cancel a source statement or pre-empt work within a batch.
async fn cooperative_yield() {
    struct YieldOnce(bool);

    impl Future for YieldOnce {
        type Output = ();

        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<()> {
            if self.0 {
                std::task::Poll::Ready(())
            } else {
                self.0 = true;
                cx.waker().wake_by_ref();
                std::task::Poll::Pending
            }
        }
    }

    YieldOnce(false).await;
}

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
                    // Not pre-interned like `intern_bindings` below (Run 4 Wave
                    // C1): an expression-based ORDER BY key is rare and this
                    // fires O(order.len()) times per row, nowhere near the
                    // O(branch.bindings.len())-per-row volume that makes
                    // `reconstruct`'s interning worth it.
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
    if let Some(rg) = &plan.rust_group {
        return rust_group_execute(plan, b, rg, control, sink).await;
    }
    let branches = plan.prepared_branches();
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
    run_branches(&branches, ctx, b, sink).await
}
/// The scalar [`Plan`] fields [`run_branches`] needs, threaded independently of
/// `branches` — decoupled so [`rust_group_execute`]'s inner-collection call can
/// override just the modifiers ([`Plan::prepared_branches`]'s single-branch
/// push-down values) without cloning the whole `Plan` first (see `run_branches`'
/// doc comment).
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

/// Non-recursive streaming loop reused by `rust_group_execute` for inner
/// collection. It takes prepared branches plus [`PlanCtx`] so that caller does
/// not clone a whole `Plan` only to repeat [`Plan::prepared_branches`]
/// (ADR-0024/M4).
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
    if !ctx.dedup_scopes.is_empty() && ctx.dedup_scopes.len() != branches.len() {
        return Err(Error::Unsupported(
            "shared term-dedup branch ownership is malformed -> 501".to_owned(),
        ));
    }
    let mut scope_keys =
        std::collections::HashMap::<usize, std::collections::BTreeSet<String>>::new();
    let mut scope_counts = std::collections::HashMap::<usize, usize>::new();
    for (branch, scope) in branches.iter().zip(ctx.dedup_scopes) {
        let Some(scope) = scope else { continue };
        super::dedup_scope_runtime::validate_runtime_scope(branch, scope)?;
        let keys: std::collections::BTreeSet<String> = scope.key_bindings.keys().cloned().collect();
        if keys.is_empty()
            || scope_keys
                .insert(scope.group_id, keys.clone())
                .is_some_and(|existing| existing != keys)
        {
            return Err(Error::Unsupported(
                "shared term-dedup key metadata is malformed -> 501".to_owned(),
            ));
        }
        *scope_counts.entry(scope.group_id).or_default() += 1;
    }
    if scope_counts.values().any(|count| *count < 2) {
        return Err(Error::Unsupported(
            "shared term-dedup group no longer spans two executable branches -> 501".to_owned(),
        ));
    }
    // Overlay hidden BGP-boundary key definitions only onto this execution's
    // private branch clone. The cached Plan and public SELECT projection remain
    // unchanged, while SQL emission/reconstruction can still dedup before the
    // later outer projection.
    let mut execution_branches = if ctx.dedup_scopes.is_empty() {
        None
    } else {
        Some(branches.to_vec())
    };
    if let Some(scoped) = &mut execution_branches {
        for (branch, scope) in scoped.iter_mut().zip(ctx.dedup_scopes) {
            if let Some(scope) = scope {
                super::dedup_scope::overlay_key_bindings(branch, &scope.key_bindings)?;
            }
        }
    }
    let branches = execution_branches.as_deref().unwrap_or(branches);
    // Fail-closed live metadata preflight: probe every distinct logical source
    // before opening any branch. Dedup uses the tagged Table/Query identity, not
    // probe SQL text (the two variants can deliberately render identical probes).
    let mut catalog = ColumnCatalog::default();
    let mut seen_sources = std::collections::HashSet::new();
    for source in emit::live_metadata_sources(branches) {
        if !seen_sources.insert(emit::logical_source_identity(source)) {
            continue;
        }
        let probe = ctx.dialect.probe_sql(source);
        ctx.control.consume(QueryCharge::SourceWork, 1)?;
        let columns = b.result_columns(&probe).await.map_err(map_sql_err)?;
        catalog.insert_live_result(source, columns)?;
    }
    ctx.control.checkpoint()?;
    emit::validate_live_columns(branches, ctx.dialect, &catalog)?;
    // Emission is part of the same preflight. A malformed later branch must fail
    // before an earlier branch can open a cursor or expose a partial result.
    let emitted_branches = branches
        .iter()
        .map(|branch| emit::emit_branch_with(branch, ctx.dialect, &catalog))
        .collect::<Result<Vec<_>>>()?;
    let multi = branches.len() > 1;
    // DISTINCT over a multi-branch bag-union: SQL dedups only within each branch, so
    // dedup the projected solutions here — before OFFSET/LIMIT (SPARQL evaluates
    // DISTINCT before slicing). The single-branch case pushes DISTINCT into SQL.
    let distinct_vars: Option<Vec<String>> = match (ctx.distinct && multi, ctx.form) {
        (true, PlanForm::Select { vars }) => Some(vars.clone()),
        _ => None,
    };
    let mut seen_tuples: std::collections::HashSet<Vec<Option<Term>>> =
        std::collections::HashSet::new();
    // ADR-0034 C0e restoration: one seen-set PER shared dedup group, keyed by
    // `ctx.dedup_scopes`' ids — declared OUTSIDE the branch loop below (unlike
    // the per-branch `own_term_seen` further down) so every branch tagged with
    // the SAME group id contributes to and checks against the SAME set, giving
    // the cross-branch dedup `unfold::pool_group`'s SQL `UNION` used to provide,
    // without ever emitting one.
    let mut group_seen: std::collections::HashMap<
        usize,
        std::collections::HashSet<Vec<Option<Term>>>,
    > = std::collections::HashMap::new();
    let mut seen = 0usize; // solutions observed (for offset)
    let mut emitted = 0usize; // solutions passed downstream (for limit)
                              // ORDER stays here, not under a source collation; ASK needs no buffer.
    let ordered = !ctx.order.is_empty() && !ctx.stop_after_first;
    let order_window = crate::resource_profile::retained_order_window(ctx.offset, ctx.limit);
    let mut buffer: Vec<(usize, Bindings)> = Vec::new();
    let mut retained_payload = 0_u64;
    let mut charged_payload_peak = 0_u64;
    for (bi, (branch, e)) in branches.iter().zip(&emitted_branches).enumerate() {
        // Run 4 Wave C0d (ADR-0034 D1's term-level dedup path — see `cascade::
        // eligible_for_term_dedup`'s doc comment for the full mechanism and its sound-
        // scope rule): `e.sql` above omitted DISTINCT even though `branch.distinct` is
        // set, because `emit_branch_with` deferred to this dedup instead of refusing.
        // `own_term_seen` is fresh PER BRANCH (unlike `seen_tuples` above, which is
        // shared across branches and keys on the OUTER projected vars) — a different
        // scope and question: this collapses duplicates WITHIN this one branch's own
        // relation, on its FULL reconstructed solution tuple (every bound variable),
        // independent of whatever the outer query later projects or whether it asked
        // for DISTINCT at all. `group_scope` (ADR-0034 C0e restoration), when set, means
        // this branch is one member of a D2 standalone group sharing `group_seen`'s
        // entry instead — a DIFFERENT branch, tagged with the SAME id, may already
        // have inserted the key this branch's own row reconstructs to (the cross-
        // branch same-triple case a fresh-per-branch set could never catch).
        let group_scope = ctx.dedup_scopes.get(bi).and_then(Option::as_ref);
        let term_dedup = group_scope.is_some() || crate::cascade::eligible_for_term_dedup(branch);
        let mut own_term_seen: std::collections::HashSet<Vec<Term>> =
            std::collections::HashSet::new();
        // The column schema is fixed for this branch's whole row stream, so index
        // it ONCE here rather than per row (ADR-0024/M4 perf — `RawRow::code_for`/
        // `AliasRow::value` used to `schema.iter().position(...)` on every lookup).
        let col_index = build_col_index(&e.projection);
        // `branch.bindings`' variable names, interned ONCE here for the whole
        // branch stream — see `intern_bindings`'s doc comment (Run 4 Wave C1,
        // the same "once per branch, not per row" idiom as `col_index` above).
        let interned = intern_bindings(branch);
        // The ONLY bind site: `e.params` bound as N positional params by the adapter.
        ctx.control.consume(QueryCharge::SourceWork, 1)?;
        let mut s = b
            .open_branch_with_decoder(
                &e.sql,
                &e.params,
                e.metadata_sql.as_deref(),
                e.sqlite_character_keys,
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
        // ~nothing — see `reconstruct_batch`'s doc comment); only whether a big
        // batch may fan out to rayon changes. `first_batch` ramps the very first
        // fill down to `TERM_GEN_FIRST_BATCH_SIZE` so a branch with many rows
        // still yields its first result quickly (the streaming invariant), then
        // grows to the full `TERM_GEN_BATCH_SIZE` for throughput.
        let mut first_batch = true;
        'branch_rows: loop {
            let target = if ctx.stop_after_first {
                1
            } else if first_batch {
                TERM_GEN_FIRST_BATCH_SIZE
            } else {
                TERM_GEN_BATCH_SIZE
            };
            let mut raw_batch: Vec<RawTuple> = Vec::with_capacity(target);
            while raw_batch.len() < target {
                // Charge the observable pull attempt before source I/O. The final
                // EOF attempt is work too, and a zero budget therefore rejects
                // before metadata/open/pull can touch the source.
                ctx.control.consume(QueryCharge::SourceWork, 1)?;
                match s.next_row().await.map_err(map_sql_err)? {
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
            // Reconstruct first: DISTINCT needs the projected terms, and dedup must
            // precede OFFSET/LIMIT (SPARQL order). `raw_batch`'s raw SQL lexical
            // values are dropped HERE, right after `reconstruct_batch` has consumed
            // them — nothing downstream (DISTINCT/ORDER BY/OFFSET/LIMIT/sink) needs
            // them again, only the reconstructed terms, so there is no reason to
            // keep `raw_batch` alive for the whole sink loop below. NOTE (measured,
            // not assumed): this does NOT move `sf-bench`'s constant-memory peak —
            // profiling found the peak is reached DURING `reconstruct_batch`'s own
            // construction (raw_batch and the growing reconstructed batch are both
            // live then regardless), not after it returns. Run 4 Wave C1 replaced
            // the per-row binding map itself (`Bindings`, this file — see its doc
            // comment) for exactly this reason: the many small (1-3-entry) per-row
            // maps live at once were previously `BTreeMap<String, Term>`, whose
            // per-node allocation dominated this peak — see `TERM_GEN_BATCH_SIZE`'s
            // doc comment for the re-tuned batch size the leaner representation
            // affords. Dropping `raw_batch` here is kept anyway as unambiguously
            // correct hygiene, not as the memory fix.
            let reconstructed =
                reconstruct_batch(&interned, &raw_batch, &col_index, ctx.parallel_term_gen);
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
                    if let Some(limit) = ctx.limit {
                        if emitted >= limit {
                            break 'branch_rows;
                        }
                    }
                }
                emitted += 1;
                sink(branch, &bindings)?.await?;
                if ctx.stop_after_first {
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
