//! Backend-generic pull-cursor execution and plan-modifier sequencing.
//!
//! The corrected per-branch sequence (design §2, mirroring the old `exec.rs`
//! SQLite loop): reconstruct → DISTINCT dedup (before slice) → if ordered {buffer,
//! defer} else {streaming OFFSET/LIMIT} → after the loop: sort THEN slice.

use std::future::Future;
use std::sync::Arc;

use sf_core::Term;
use sf_sql::{BranchStream, Dialect, RawTuple, SqlBackend};

use crate::emit::{self, ColumnCatalog};
use crate::iq::{Branch, OrderKey};
use crate::{Error, Plan, PlanForm, Result};

use super::batch::{reconstruct_batch, TERM_GEN_BATCH_SIZE, TERM_GEN_FIRST_BATCH_SIZE};
use super::expression::eval_expr;
use super::forms::rust_group_execute;
use super::order::{order_cmp_precomputed, precompute_order_keys, TermSortKey};
use super::row::{build_col_index, canonical_pairs, intern_bindings, Bindings};

/// Flatten an `sf-sql` driver error's source chain into the message (the SQLite
/// chain is usually empty, so this is byte-identical to the old `Error::Sql(e)`).
fn map_sql_err(e: sf_sql::Error) -> Error {
    use std::error::Error as _;
    // An uncovered PG result type (adapter `pg_value`) is preserved as a distinct
    // 501 skip — byte-identical to the pre-M3 `exec_pg` path, which returned
    // `sf_sparql::Error::Unsupported` directly from `pg_value` (never `Sql`).
    if let sf_sql::Error::Unsupported(m) = &e {
        return Error::Unsupported(m.clone());
    }
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        msg.push_str(": ");
        msg.push_str(&s.to_string());
        src = s.source();
    }
    Error::Sql(msg)
}

/// Drive an always-ready future to completion with no runtime (design §5 M2
/// sync↔async bridge). SQLite backend waits resolve synchronously; the explicit
/// cooperative checkpoint below returns `Pending` once and is immediately
/// re-polled. A `noop` waker + poll loop therefore needs no tokio runtime and never
/// nests / panics inside `sf-serve`'s `spawn_blocking`.
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

/// Evaluate ORDER BY expression keys (e.g. `STRLEN(?n)`) and inject each result as a
/// synthetic binding so `order_cmp` finds it (design §2 — the extraction of the old
/// SQLite-only `exec.rs` injection, now backend-uniform).
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
    F: FnMut(&Branch, &Bindings) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    if let Some(rg) = &plan.rust_group {
        return rust_group_execute(plan, b, rg, sink).await;
    }
    let branches = plan.prepared_branches();
    let ctx = PlanCtx {
        dialect: plan.dialect,
        distinct: plan.distinct,
        form: &plan.form,
        order: &plan.order,
        offset: plan.offset,
        limit: plan.limit,
        // This is the plain streaming path (design §2: reconstruct -> DISTINCT ->
        // ORDER/slice -> sink, ONE row in flight downstream) — parallel term-gen
        // is now allowed here too, see `reconstruct_batch`'s `parallel_allowed`
        // doc comment and [`TERM_GEN_MIN_PARALLEL_ROWS`]'s doc comment for the
        // measured reason (ledger F8, un-gated on an idle-machine re-measurement).
        parallel_term_gen: true,
        dedup_groups: &plan.dedup_groups,
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
    /// Whether [`reconstruct_batch`] may dispatch a large-enough batch to rayon
    /// (see its `parallel_allowed` doc comment, ledger F8) — `true` only for
    /// [`rust_group_execute`]'s inner collection.
    pub(super) parallel_term_gen: bool,
    /// ADR-0034 C0e restoration — [`Plan::dedup_groups`] verbatim: a branch's own
    /// representative scan/opt/subplan alias → shared term-dedup-set id. See
    /// [`run_branches`]'s own doc comment for how this replaces a fresh
    /// per-branch seen-set with one shared across every same-id branch.
    pub(super) dedup_groups: &'a std::collections::HashMap<usize, usize>,
}

/// The scan/opt alias to look up in [`PlanCtx::dedup_groups`] for `branch`'s
/// own shared dedup-group id, if any (ADR-0034 C0e restoration). A standalone
/// group member tagged `distinct = true` (`unfold::pool_pattern_relation` /
/// `iq::resolve`'s Intensional arm) that lands as a `Union` child on the TREE
/// engine is never the query's own top-level spine, so `iq::lower::
/// lower_as_subplan` wraps it in a `SubPlanJoin` regardless — the SAME
/// wrapping a lone `eligible_for_term_dedup` branch already gets there (e.g.
/// W3C TC0005b). `branch.core`/`.opts` end up empty (`Branch::alias_sources`
/// finds nothing), but the scan whose alias was registered survives
/// unchanged, just nested one level down inside the SubPlan's own inner
/// `Plan` — `lower_as_subplan` mints a FRESH alias for the `SubPlanJoin`
/// itself, never for the scan it wraps. The flat engine never wraps anything
/// (`alias_sources` alone always finds it there), so this only recurses on
/// tree output.
pub(crate) fn dedup_group_alias(branch: &Branch) -> Option<usize> {
    if let Some((alias, _)) = branch.alias_sources().into_iter().next() {
        return Some(alias);
    }
    let inner = branch.subplan_joins.first()?.plan.branches.first()?;
    inner
        .alias_sources()
        .into_iter()
        .next()
        .map(|(alias, _)| alias)
}

/// [`for_each_solution`]'s non-`rust_group` streaming loop — does NOT check
/// `rust_group` (the non-recursive split, so `rust_group_execute` can reuse it to
/// collect inner solutions). One row in flight. Takes already-prepared branches
/// ([`Plan::prepared_branches`]) plus the plan's scalar fields via [`PlanCtx`]
/// rather than `&Plan`, so a caller that already holds a `Vec<Branch>` (the
/// `rust_group` inner-collection path) is not forced to clone a whole `Plan` just
/// to get one straight back out of `Plan::prepared_branches` again (ADR-0024/M4
/// perf: this used to clone `plan.branches` twice — once building a throwaway
/// `inner_plan`, once more inside `prepared_branches` — for exactly that reason).
pub(super) async fn run_branches<B, F, Fut>(
    branches: &[Branch],
    ctx: PlanCtx<'_>,
    b: &mut B,
    mut sink: F,
) -> Result<()>
where
    B: SqlBackend,
    F: FnMut(&Branch, &Bindings) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    // Catalog: one probe per distinct source; SWALLOW column_names errors (design
    // A4 — a source whose metadata cannot be read is omitted; resolution falls back
    // to the raw identifier).
    let mut catalog = ColumnCatalog::default();
    let mut seen_probe = std::collections::HashSet::new();
    for branch in branches {
        for (_, source) in branch.alias_sources() {
            let probe = ctx.dialect.probe_sql(source);
            if !seen_probe.insert(probe.clone()) {
                continue;
            }
            if let Ok(names) = b.column_names(&probe).await {
                catalog.insert(source, names);
            }
        }
    }
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
    // ADR-0034 C0e restoration: one seen-set PER shared dedup-group id, keyed by
    // `ctx.dedup_groups`' values — declared OUTSIDE the branch loop below (unlike
    // the per-branch `own_term_seen` further down) so every branch tagged with
    // the SAME group id contributes to and checks against the SAME set, giving
    // the cross-branch dedup `unfold::pool_group`'s SQL `UNION` used to provide,
    // without ever emitting one.
    let mut group_seen: std::collections::HashMap<usize, std::collections::HashSet<Vec<Term>>> =
        std::collections::HashMap::new();
    let mut seen = 0usize; // solutions observed (for offset)
    let mut emitted = 0usize; // solutions passed downstream (for limit)
                              // ORDER BY is applied HERE for every plan, never in SQL (a SQL ORDER BY inherits
                              // the column's collation/affinity). Buffer, stable-sort via the type-aware
                              // order_cmp, then OFFSET/LIMIT (SPARQL §15: order, then slice).
    let ordered = !ctx.order.is_empty();
    let mut buffer: Vec<(usize, Bindings)> = Vec::new();
    for (bi, branch) in branches.iter().enumerate() {
        let e = emit::emit_branch_with(branch, ctx.dialect, &catalog)?;
        // Run 4 Wave C0d (ADR-0034 D1's term-level dedup path — see `cascade::
        // eligible_for_term_dedup`'s doc comment for the full mechanism and its sound-
        // scope rule): `e.sql` above omitted DISTINCT even though `branch.distinct` is
        // set, because `emit_branch_with` deferred to this dedup instead of refusing.
        // `own_term_seen` is fresh PER BRANCH (unlike `seen_tuples` above, which is
        // shared across branches and keys on the OUTER projected vars) — a different
        // scope and question: this collapses duplicates WITHIN this one branch's own
        // relation, on its FULL reconstructed solution tuple (every bound variable),
        // independent of whatever the outer query later projects or whether it asked
        // for DISTINCT at all. `group_id` (ADR-0034 C0e restoration), when set, means
        // this branch is one member of a D2 standalone group sharing `group_seen`'s
        // entry instead — a DIFFERENT branch, tagged with the SAME id, may already
        // have inserted the key this branch's own row reconstructs to (the cross-
        // branch same-triple case a fresh-per-branch set could never catch).
        let group_id: Option<usize> =
            dedup_group_alias(branch).and_then(|alias| ctx.dedup_groups.get(&alias).copied());
        let term_dedup = group_id.is_some() || crate::cascade::eligible_for_term_dedup(branch);
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
        let mut s = b
            .open_branch(&e.sql, &e.params)
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
            let target = if first_batch {
                TERM_GEN_FIRST_BATCH_SIZE
            } else {
                TERM_GEN_BATCH_SIZE
            };
            let mut raw_batch: Vec<RawTuple> = Vec::with_capacity(target);
            while raw_batch.len() < target {
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
                    let key: Vec<Term> = canonical_pairs(&bindings)
                        .into_iter()
                        .map(|(_, v)| v.clone())
                        .collect();
                    let inserted = match group_id {
                        Some(gid) => group_seen.entry(gid).or_default().insert(key),
                        None => own_term_seen.insert(key),
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
                    buffer.push((bi, bindings));
                    continue;
                }
                // Streaming OFFSET/LIMIT only when SQL didn't apply them (a multi-branch
                // bag-union; a single unordered branch sliced in SQL).
                if multi {
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
                sink(branch, &bindings).await?;
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
        let keys: Vec<Vec<Option<TermSortKey>>> = buffer
            .iter()
            .map(|(_, bindings)| precompute_order_keys(ctx.order, bindings))
            .collect();
        let mut idx: Vec<usize> = (0..buffer.len()).collect();
        idx.sort_by(|&i, &j| order_cmp_precomputed(ctx.order, &keys[i], &keys[j]));
        let take = ctx.limit.unwrap_or(usize::MAX);
        for &i in idx.iter().skip(ctx.offset).take(take) {
            let (bi, bindings) = &buffer[i];
            sink(&branches[*bi], bindings).await?;
        }
    }
    Ok(())
}
