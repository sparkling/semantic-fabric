//! `exec_core` — the driver-agnostic execution core (ADR-0024). One async pull-cursor
//! loop generic over [`SqlBackend`], plus the per-form entry points. The SQLite entry
//! points in [`crate::exec`] delegate here through a `block_on` shim; PostgreSQL /
//! MySQL keep their parallel loops until M3/M4 (design §5).
//!
//! The term-gen helpers (`reconstruct` / `order_cmp` / `eval_expr` / `instantiate` /
//! `Solutions` / `rust_group_result_rows` / `RawRow` and their private helpers) are
//! physically single-homed **here** (ADR-0024 M5, design §2). [`crate::exec`]
//! re-exports `Solutions`; the PostgreSQL / MySQL executors import it by that path.
//!
//! The corrected per-branch sequence (design §2, mirroring the old `exec.rs`
//! SQLite loop): reconstruct → DISTINCT dedup (before slice) → if ordered {buffer,
//! defer} else {streaming OFFSET/LIMIT} → after the loop: sort THEN slice.

use std::cmp::Ordering;
use std::future::Future;
use std::sync::Arc;

use oxsdatatypes::Decimal;
use sf_core::datatype::{self, XsdTypeCode};
use sf_core::ir::{TermMap, TermType};
use sf_core::{Literal, Row, Term, Triple};
use sf_sql::{BranchStream, Dialect, RawTuple, SqlBackend};
use spargebra::algebra::{Expression, Function};

use crate::emit::{self, ColumnCatalog};
use crate::iq::{AggKind, Branch, ColRef, OrderKey, RustAgg, RustGroup, TermDef};
use crate::{Error, Plan, PlanForm, Result};

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
/// sync↔async bridge). The SQLite backend's stream is fully synchronous, so every
/// `.await` resolves `Ready` immediately; a `noop` waker + poll loop needs no tokio
/// runtime and therefore never nests / panics inside `sf-serve`'s `spawn_blocking`.
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
async fn for_each_solution<B, F, Fut>(plan: &Plan, b: &mut B, sink: F) -> Result<()>
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
        // ORDER/slice -> sink, ONE row in flight downstream) — never parallelize
        // term-gen here, see `reconstruct_batch`'s `parallel_allowed` doc comment
        // for the measured reason (ledger F8).
        parallel_term_gen: false,
        dedup_groups: &plan.dedup_groups,
    };
    run_branches(&branches, ctx, b, sink).await
}

/// The scalar [`Plan`] fields [`run_branches`] needs, threaded independently of
/// `branches` — decoupled so [`rust_group_execute`]'s inner-collection call can
/// override just the modifiers ([`Plan::prepared_branches`]'s single-branch
/// push-down values) without cloning the whole `Plan` first (see `run_branches`'
/// doc comment).
struct PlanCtx<'a> {
    dialect: Dialect,
    distinct: bool,
    form: &'a PlanForm,
    order: &'a [OrderKey],
    offset: usize,
    limit: Option<usize>,
    /// Whether [`reconstruct_batch`] may dispatch a large-enough batch to rayon
    /// (see its `parallel_allowed` doc comment, ledger F8) — `true` only for
    /// [`rust_group_execute`]'s inner collection.
    parallel_term_gen: bool,
    /// ADR-0034 C0e restoration — [`Plan::dedup_groups`] verbatim: a branch's own
    /// representative scan/opt/subplan alias → shared term-dedup-set id. See
    /// [`run_branches`]'s own doc comment for how this replaces a fresh
    /// per-branch seen-set with one shared across every same-id branch.
    dedup_groups: &'a std::collections::HashMap<usize, usize>,
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
fn dedup_group_alias(branch: &Branch) -> Option<usize> {
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
async fn run_branches<B, F, Fut>(
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

/// Multi-branch GROUP BY: collect every inner solution (no DISTINCT/OFFSET/LIMIT/
/// ORDER on the inner — those apply AFTER grouping), then group + aggregate + slice
/// via the backend-independent [`rust_group_result_rows`] and stream the grouped
/// rows. Drives [`run_branches`] directly (see its doc comment) instead of
/// wrapping the inner modifiers into a freshly-cloned `Plan`.
async fn rust_group_execute<B, F, Fut>(
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
    for_each_solution(plan, b, |_branch, bindings| {
        let mut triples: Vec<Triple> = template
            .iter()
            .filter_map(|tp| instantiate(tp, bindings))
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
    for_each_solution(plan, b, |_branch, bindings| {
        for tp in &template {
            if let Some(triple) = instantiate(tp, bindings) {
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

// --- single-homed term-gen helpers (relocated from exec.rs, ADR-0024 M5) ------

/// `(alias, column) -> index` into a branch's fixed row schema — built ONCE per
/// branch ([`build_col_index`]), since the projection schema doesn't change row
/// to row, so [`RawRow`]'s per-row column lookups are an O(log n) binary search
/// instead of an O(n) `schema.iter().position(...)` scan per var, per row
/// (ADR-0024/M4 perf). A SORTED `Vec` + binary search, not a `HashMap`: a
/// branch's schema is typically a handful of columns, small enough that a
/// `HashMap`'s constant-factor overhead (table allocation, `SipHash` over the
/// `(usize, &str)` key) measurably LOST to the plain linear scan in a criterion
/// bench — a sorted `Vec` avoids both the allocation and the hashing while still
/// beating an O(n) scan once a branch's schema is large (e.g. a multi-table join).
type ColIndex<'a> = Vec<((usize, &'a str), usize)>;

/// Build a [`ColIndex`] over a branch's projection schema (see its doc comment).
fn build_col_index(schema: &[ColRef]) -> ColIndex<'_> {
    let mut index: ColIndex<'_> = schema
        .iter()
        .enumerate()
        .map(|(i, c)| ((c.alias, &*c.column), i))
        .collect();
    index.sort_unstable_by_key(|&(key, _)| key);
    index
}

/// Look up `(alias, column)` in a [`ColIndex`] via binary search.
fn col_index_get(index: &ColIndex<'_>, alias: usize, column: &str) -> Option<usize> {
    index
        .binary_search_by_key(&(alias, column), |&(key, _)| key)
        .ok()
        .map(|pos| index[pos].1)
}

/// One projected result row's raw column values plus each value's resolved §10
/// type (declared type, else storage-class fallback), addressed by [`ColRef`] via
/// a precomputed [`ColIndex`]. `pub(crate)` so the PostgreSQL executor
/// ([`crate::exec_pg`]) drives the same single term-gen path (ADR-0003 R3) with
/// PG-extracted values.
pub(crate) struct RawRow<'a> {
    pub(crate) values: &'a [Option<String>],
    pub(crate) codes: &'a [Option<XsdTypeCode>],
    pub(crate) index: &'a ColIndex<'a>,
}

impl RawRow<'_> {
    /// The resolved §10 XSD type of `column` under `alias`, if any.
    fn code_for(&self, alias: usize, column: &str) -> Option<XsdTypeCode> {
        col_index_get(self.index, alias, column).and_then(|i| self.codes[i])
    }
}

/// A view of a [`RawRow`] scoped to one scan alias, so a mapping term map's
/// column lookups resolve to that scan's projected columns ([`sf_core::Row`]).
struct AliasRow<'a> {
    raw: &'a RawRow<'a>,
    alias: usize,
}

impl Row for AliasRow<'_> {
    fn value(&self, column: &str) -> Option<&str> {
        col_index_get(self.raw.index, self.alias, column)
            .and_then(|i| self.raw.values[i].as_deref())
    }
}

/// Materialise a term definition into an `oxrdf` term, or `None` if a referenced
/// column is NULL/absent (R2RML §11: no value ⇒ no term ⇒ unbound).
fn build_term(def: &TermDef, raw: &RawRow<'_>) -> Result<Option<Term>> {
    match def {
        TermDef::Const(t) => Ok(Some(t.clone())),
        TermDef::Derived { term_map, alias } => derived_term(term_map, *alias, raw),
        // R2 COALESCE: the preserved (left) side wins when bound; otherwise the
        // optional (right) value (ADR-0007). `None` from `left` = its source
        // columns were NULL (the optional did not match), so fall back to `right`.
        TermDef::Coalesce(l, r) => match build_term(l, raw)? {
            Some(t) => Ok(Some(t)),
            None => build_term(r, raw),
        },
        // BIND(CONCAT(…)) — SPARQL §17.4.5.4. Every operand must be a string literal
        // (xsd:string, simple, or lang-tagged); an unbound / IRI / blank-node operand
        // or a non-string *typed* literal is an expression error, so the BIND variable
        // is left unbound (Ok(None)) — never a wrong value. The result carries the
        // common language tag iff every operand shares it, else a simple literal.
        TermDef::Concat(parts) => {
            let mut s = String::new();
            let mut common_lang: Option<Option<String>> = None; // unset | mixed | lang
            for p in parts {
                let Some(Term::Literal(l)) = build_term(p, raw)? else {
                    return Ok(None);
                };
                let lang = l.language();
                if lang.is_none() && l.datatype() != sf_core::vocab::xsd::STRING {
                    return Ok(None); // a non-string typed literal ⇒ type error
                }
                s.push_str(l.value());
                let this = lang.map(str::to_owned);
                common_lang = Some(match common_lang {
                    None => this,                       // first operand sets it
                    Some(prev) if prev == this => prev, // still consistent
                    Some(_) => None,                    // diverged ⇒ no common tag
                });
            }
            let term = match common_lang.flatten() {
                Some(lang) => Literal::new_language_tagged_literal(s, lang)
                    .map_err(|e| Error::Core(e.to_string()))?,
                None => Literal::new_simple_literal(s),
            };
            Ok(Some(Term::Literal(term)))
        }
        // An aggregate result (SPARQL §11): the value is the SQL aggregate computed
        // at `col`. A NULL value is an empty multiset: SUM (and COUNT, defensively —
        // SQL `COUNT` never NULLs) over an empty multiset is `"0"^^xsd:integer`,
        // while AVG/MIN/MAX (and SAMPLE) are UNBOUND (§11). The §10 type is
        // `fixed_type` when the function pins it (COUNT ⇒ integer), else the
        // column's resolved decltype/storage class (SUM/MIN/MAX keep the source
        // numeric type). AVG (§11.4) follows the OPERAND numeric type under XPath
        // promotion — resolved from `operand`'s §10 type, since SQLite's `AVG`
        // always yields a REAL (the operand is projected bare on SQLite; on PG it is
        // absent and `avg()`'s own promoted result type is used).
        TermDef::Agg {
            col,
            kind,
            operand,
            fixed_type,
        } => {
            let row = AliasRow {
                raw,
                alias: col.alias,
            };
            let Some(value) = row.value(&col.column) else {
                // A NULL SQL aggregate value on the single-branch SQL-pushdown path. ADR-0025
                // C.7: SUM/AVG/COUNT over an EMPTY group ⇒ "0"^^xsd:integer (SPARQL §11); only
                // MIN/MAX of an empty multiset are UNBOUND. This is sound HERE specifically
                // because ADR-0025 C.6 routes any NULLABLE-operand aggregate to `rust_group` —
                // so on this SQL path the operand is MANDATORY (bound in every row), hence a
                // NULL aggregate value means 0 rows (empty group), never "non-empty but all
                // operands unbound" (which must be UNBOUND and is handled correctly by
                // `rust_agg` C.4/C.5). Pre-C.6 this branch conflated the two for AVG.
                return match kind {
                    AggKind::Sum | AggKind::Count | AggKind::Avg => {
                        Ok(Some(natural_literal("0", XsdTypeCode::Integer)?))
                    }
                    AggKind::Min | AggKind::Max => Ok(None),
                };
            };
            let code = match kind {
                AggKind::Avg => {
                    let operand_code = operand
                        .as_ref()
                        .and_then(|o| raw.code_for(o.alias, &o.column))
                        .or_else(|| raw.code_for(col.alias, &col.column))
                        .unwrap_or(XsdTypeCode::Decimal);
                    avg_result_code(operand_code)
                }
                _ => fixed_type
                    .or_else(|| raw.code_for(col.alias, &col.column))
                    .unwrap_or(XsdTypeCode::String),
            };
            Ok(Some(natural_literal(value, code)?))
        }
        // ADR-0032 D2 — the ONLY route by which this engine ever produces a native
        // `Term::Triple`: recursively realize the three components, then compose via
        // `Triple::from_terms`, which is fallible and enforces RDF 1.2 §3.1 position
        // legality (subject IRI/bnode, predicate IRI) for free. A failed composition
        // (illegal shape) OR an unbound component ⇒ unbound (`None`) — never an error,
        // matching SPARQL's usual "error in construction ⇒ unbound" discipline at
        // projection. Deliberately bypasses `sf_core::term::generate` (`GenTerm` has
        // no triple arm by design, ADR-0006 zero-alloc — see the module-level note on
        // `TermDef::ComposedTriple`).
        TermDef::ComposedTriple {
            subject,
            predicate,
            object,
        } => {
            let (Some(s), Some(p), Some(o)) = (
                build_term(subject, raw)?,
                build_term(predicate, raw)?,
                build_term(object, raw)?,
            ) else {
                return Ok(None);
            };
            Ok(Triple::from_terms(s, p, o).ok().map(Term::from))
        }
    }
}

/// Build a derived term, applying the R2RML §10 natural datatype mapping
/// (ADR-0015) when — and only when — the term map is a column-valued literal with
/// no explicit `rr:datatype` / `rr:language`. Templates, IRIs, blank nodes, and
/// explicitly-typed/lang-tagged literals go through the plain `sf-core` term-gen
/// path unchanged.
fn derived_term(term_map: &TermMap, alias: usize, raw: &RawRow<'_>) -> Result<Option<Term>> {
    if let TermMap::Column(col, spec) = term_map {
        if spec.term_type == TermType::Literal && spec.datatype.is_none() && spec.language.is_none()
        {
            let row = AliasRow { raw, alias };
            let Some(value) = row.value(col) else {
                return Ok(None);
            };
            let code = raw.code_for(alias, col).unwrap_or(XsdTypeCode::String);
            return Ok(Some(natural_literal(value, code)?));
        }
    }
    let row = AliasRow { raw, alias };
    sf_core::term::generate(term_map, &row).map_err(|e| Error::Core(e.to_string()))
}

/// Produce the RDF literal for a value under its §10 natural XSD type, in the
/// XSD-canonical lexical form (ADR-0015 chokepoint, `sf_core::datatype`).
/// `HexBinary` values arrive already uppercase-hex-encoded from blob extraction.
fn natural_literal(value: &str, code: XsdTypeCode) -> Result<Term> {
    let literal = match code {
        XsdTypeCode::String => Literal::new_simple_literal(value),
        XsdTypeCode::HexBinary => Literal::new_typed_literal(value, code.iri()),
        _ => {
            let mut buf = String::new();
            datatype::canonical_lexical(value, code, &mut buf)
                .map_err(|e| Error::Core(e.to_string()))?;
            Literal::new_typed_literal(buf, code.iri())
        }
    };
    Ok(Term::Literal(literal))
}

/// The §10 result datatype of `AVG(operand)` (SPARQL §11.4: AVG = SUM/COUNT under
/// XPath numeric type promotion). The result follows the operand numeric type:
/// `xsd:double` is preserved (so is `xsd:float`, which this codebase folds into
/// `xsd:double`); `xsd:integer` and `xsd:decimal` promote to `xsd:decimal`.
fn avg_result_code(operand: XsdTypeCode) -> XsdTypeCode {
    match operand {
        XsdTypeCode::Double => XsdTypeCode::Double,
        _ => XsdTypeCode::Decimal,
    }
}

/// One reconstructed SPARQL solution row's bound-variable -> term mapping
/// (Run 4 Wave C1, replacing the former `BTreeMap<String, Term>`): a small
/// linear-scan `Vec`, not a tree. `sf-bench`'s `constant_memory` peak-heap
/// profiling (see [`TERM_GEN_BATCH_SIZE`]'s doc comment) found `BTreeMap`'s
/// per-node allocation overhead — not the term data itself — dominated peak
/// heap in the buffered-batch window, because a typical branch binds only a
/// handful (1-3) of variables per row: far below where a tree's O(log n)
/// lookup would ever beat a linear scan (the same reasoning [`ColIndex`]
/// documents for a branch's column schema). Var names are `Arc<str>`, not
/// `String`: every row [`reconstruct`] builds for one branch's stream shares
/// that branch's SAME interned handles ([`intern_bindings`]), so a per-row
/// insert clones an `Arc` (refcount bump) instead of allocating a fresh
/// `String`.
///
/// Preserves INSERTION order, NOT the old `BTreeMap`'s alphabetical-by-key
/// order. [`Bindings::get`]/[`contains_key`](Bindings::contains_key) (keyed
/// lookup) are unaffected by this, but a site that needs a canonical,
/// order-independent view of the WHOLE row — hashing it or structurally
/// comparing it, as opposed to looking up one named variable — must go
/// through [`canonical_pairs`] first, or two equal solutions whose vars
/// happened to get bound/inserted in a different sequence would compare
/// unequal. The `derive`d [`PartialEq`] below is therefore ALSO
/// insertion-order sensitive (structural, element-by-element) — fine for the
/// one place this file compares `Bindings` values directly
/// (`order_sort_key_tests`, where both sides are clones of the same original
/// rows, never rebuilt), but not a substitute for [`canonical_pairs`]
/// anywhere a value could have been built along a different path.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Bindings(Vec<(Arc<str>, Term)>);

impl Bindings {
    fn new() -> Self {
        Bindings(Vec::new())
    }

    /// The term bound to `var`, if any.
    fn get(&self, var: &str) -> Option<&Term> {
        self.0.iter().find(|(k, _)| &**k == var).map(|(_, v)| v)
    }

    fn contains_key(&self, var: &str) -> bool {
        self.0.iter().any(|(k, _)| &**k == var)
    }

    /// `BTreeMap::insert`'s replace-on-existing-key semantics: overwrite
    /// `var`'s slot if already bound, else append a new one.
    fn insert(&mut self, var: Arc<str>, term: Term) {
        match self.0.iter_mut().find(|(k, _)| *k == var) {
            Some(slot) => slot.1 = term,
            None => self.0.push((var, term)),
        }
    }

    /// Append `(var, term)` WITHOUT checking for an existing key — sound only
    /// when the caller already guarantees `var` is not yet bound. Prefer
    /// [`Bindings::insert`] anywhere that isn't true; [`reconstruct`] is the
    /// one caller that can (its `interned` source is unique-by-construction,
    /// see [`intern_bindings`]).
    fn push(&mut self, var: Arc<str>, term: Term) {
        self.0.push((var, term));
    }

    fn iter(&self) -> impl Iterator<Item = (&str, &Term)> {
        self.0.iter().map(|(k, v)| (&**k, v))
    }
}

/// [`Bindings`]'s pairs in CANONICAL (var-name-sorted) order — see
/// [`Bindings`]'s doc comment for why any whole-row hash/structural-equality
/// site needs this instead of raw [`Bindings::iter`] order. The two sites
/// that hash a FULL solution row rather than looking up one named variable:
/// `run_branches`' ADR-0034 D1 term-dedup key, and `rust_agg`'s
/// `COUNT(DISTINCT *)` key.
fn canonical_pairs(b: &Bindings) -> Vec<(&str, &Term)> {
    let mut pairs: Vec<(&str, &Term)> = b.iter().collect();
    pairs.sort_unstable_by_key(|&(k, _)| k);
    pairs
}

/// [`Branch::bindings`]'s variable names, pre-interned as [`Arc<str>`] and
/// paired with their [`TermDef`] — built ONCE per branch (`run_branches`,
/// mirroring [`build_col_index`]'s "once per branch, not per row" idiom, see
/// its doc comment). Every row [`reconstruct`] builds for this branch then
/// clones an already-allocated `Arc` (a refcount bump) into its [`Bindings`]
/// instead of allocating a fresh `String` per variable per row (Run 4 Wave
/// C1 — the ADR-0006 correction note's "leaner per-row binding
/// representation"). Does NOT touch [`Branch::bindings`] itself, which stays
/// a `BTreeMap<String, TermDef>` — its alphabetical iteration order is
/// load-bearing elsewhere (`iq::lower`'s positional `c{i}` alias assignment).
type InternedBindings<'a> = Vec<(Arc<str>, &'a TermDef)>;

fn intern_bindings(branch: &Branch) -> InternedBindings<'_> {
    branch
        .bindings
        .iter()
        .map(|(var, def)| (Arc::from(var.as_str()), def))
        .collect()
}

/// Reconstruct all bound variables of one raw row from `interned` — a
/// branch's [`intern_bindings`] output, built ONCE per branch (see its doc
/// comment). `pub(crate)` so the PostgreSQL executor reuses the identical
/// reconstruction (ADR-0003 R3).
pub(crate) fn reconstruct(interned: &InternedBindings<'_>, raw: &RawRow<'_>) -> Result<Bindings> {
    let mut out = Bindings::new();
    for (var, def) in interned {
        if let Some(term) = build_term(def, raw)? {
            // `push`, not `insert`: `interned` comes from a `BTreeMap` (unique
            // keys), so `var` can never already be bound in `out`.
            out.push(var.clone(), term);
        }
    }
    Ok(out)
}

/// [`run_branches`]'s steady-state term-gen batch size: the number of raw rows
/// buffered off the cursor before [`reconstruct_batch`] runs (and, above
/// [`TERM_GEN_MIN_PARALLEL_ROWS`], parallelizes) term generation and the batch is
/// emitted downstream in order. Bounds the extra memory to O(batch), never
/// O(result) (the bounded-memory invariant, ADR-0006): a batch buffer IS real
/// memory, but a FIXED amount independent of source scale — `sf-bench`'s
/// `engine_memory_is_batch_bounded_past_the_batch_size_threshold` confirms the
/// peak plateaus (near-identical) once a single branch's row count exceeds
/// this, at 20k vs 80k rows.
///
/// **Re-measured, twice, not the ADR's original "1000 rows/task" figure.**
/// ADR-0006's M4 wave-2 note measured per-row rayon dispatch (~10ns/row, one
/// task per row) ~2x SLOWER than inline, and a "1000 rows/task chunked
/// dispatch" ~6x FASTER — but that number came from a ONE-SHOT `par_chunks`
/// call over a whole dataset at once. This loop's streaming shape instead
/// issues one FRESH `par_chunks` call PER BATCH (a cursor can't be buffered
/// whole without breaking the streaming invariant below), and re-measuring at
/// THAT granularity (`sf-bench`'s `micro_term_gen_batch`, ~100k synthetic
/// `rr:template` rows) found 1000-row batches ~1.8x SLOWER than plain inline —
/// the fixed per-call dispatch cost (thread wake/join) dominates a batch this
/// small the same way it dominated a single row; throughput alone would want
/// 10 000+ (a measured, comfortably-margined ~1.6-1.7x faster there). But a
/// SECOND, independent constraint caps it much lower: each buffered-and-
/// reconstructed row costs far more than its raw bytes, so `sf-bench`'s own
/// `constant_memory` peak-heap invariant test — which THIS same restructure
/// must keep passing — is what actually picks the value. Originally (pre-C1,
/// see below) this was a `BTreeMap<String, Term>`'s per-node allocation
/// overhead on the 1-3-entry maps a typical branch binds, which measured the
/// mem_ratio blowing well past the test's `4.0` tolerance at 5000 (5.44) and
/// 10 000 (9.05), while 3000 stayed comfortably under it (~3.4, vs 3500's
/// fragile ~3.9). See [`reconstruct_batch`] for why a batch this size is
/// still chunked FURTHER within each dispatch, not handed to rayon as a
/// single task.
///
/// **Re-tuned again, Run 4 Wave C1**, once [`Bindings`] (this file's own doc
/// comment) replaced that `BTreeMap` with a leaner `Vec<(Arc<str>, Term)>` —
/// exactly the "a leaner per-row binding representation would raise this
/// ceiling" open question the ADR-0006 correction note above used to leave
/// for a future wave. Re-running the SAME `engine_memory_is_bounded_under_
/// growing_source` sweep at successive batch sizes, with `Bindings` in
/// place, measured: 3000 → mem_ratio 3.0 (down from `BTreeMap`'s ~3.4 at the
/// SAME batch size — the leaner representation alone lowers the ratio), 3500
/// → 3.35, 4000 → 3.69, 4500 → 4.01 (just past tolerance), 5000 → 4.31
/// (fails, vs `BTreeMap`'s 5.44 at the same size — lower, but still over).
/// The ceiling rose, as predicted, but not without bound: 4000 is the new
/// memory-constrained choice, picked with the same margin-below-the-
/// fragile-edge judgment that rejected 3500 over 3000 originally (4500's
/// 4.01 is exactly that kind of fragile-close call).
/// `engine_memory_is_batch_bounded_past_the_batch_size_threshold` (the
/// 20k-vs-80k-rows plateau proof) stays exactly 1.0x at 4000. End-to-end
/// throughput corroborates the choice — `sf-bench`'s `obda_construct_dump`
/// (CONSTRUCT-dump wall-clock) improved ~15% (1x) / ~19% (10x), and the
/// `rust_group`-routed `micro_distinct_agg`/`micro_group_avg_rust` improved
/// ~33%/~30% — all measured with BOTH this batch bump and the `Bindings`
/// swap together (not decomposed further); only the mem_ratio comparison
/// above isolates the representation's OWN contribution (3.4 → 3.0 at the
/// unchanged batch=3000).
const TERM_GEN_BATCH_SIZE: usize = 4_000;

/// The size of ONLY the first batch pulled from a branch's cursor (every batch
/// after it uses the full [`TERM_GEN_BATCH_SIZE`]). Filling a full batch before
/// the first `sink` call would make a branch with many rows hold up the
/// caller's first streamed result — the streaming invariant (ADR-0006: "first
/// result must not wait for the whole result set") bounds added latency by the
/// batch size, so the first batch stays small regardless of how large the
/// steady-state batch is. 64 keeps first-result latency in the same order of
/// magnitude as the old per-row loop (measured in `sf-bench`'s `obda_latency`
/// `first_result_µs`) — well under [`TERM_GEN_MIN_PARALLEL_ROWS`], so the first
/// batch is always reconstructed sequentially (no dispatch overhead on the
/// latency-critical path either).
const TERM_GEN_FIRST_BATCH_SIZE: usize = 64;

/// Below this many rows, [`reconstruct_batch`] reconstructs the WHOLE batch
/// sequentially — no rayon dispatch at all. A separate, smaller concern from
/// [`TERM_GEN_BATCH_SIZE`]: this is the floor on whether a batch is worth
/// dispatching to the pool AT ALL (a fresh `par_chunks` call has a real, mostly
/// fixed cost — thread wake/join — that a small batch's own work cannot repay);
/// [`reconstruct_batch`]'s internal chunk size is a separate, much smaller floor
/// governing fan-out WITHIN an already-dispatched batch. Always true of
/// [`TERM_GEN_FIRST_BATCH_SIZE`] (64) and of a stream's final partial batch when
/// it undershoots this. MUST stay at or below [`TERM_GEN_BATCH_SIZE`], or a
/// full-size batch would never parallelize at all — 2000 sits just under it
/// (the `micro_term_gen_batch` sweep found 2000 alone still a throughput wash,
/// but the full 3000-row batch this gates comes out ahead — see that bench's
/// own numbers for the batch-size-vs-dispatch-count tradeoff).
///
/// **Row count alone does not predict whether dispatch pays off (ledger F8).**
/// This threshold only says a batch is BIG enough to amortize `par_chunks`'
/// fixed per-call cost — it says nothing about whether each row's own
/// reconstruction work is expensive enough to be worth amortizing in the first
/// place. `sf-bench`'s streamed CONSTRUCT dump (`constant_memory_dump`) crosses
/// this threshold at 10x/100x scale (some GTFS branches run tens of thousands
/// of rows) yet REGRESSED 31-35% under dispatch: its rows are plain
/// column/template copies (`Literal::new_simple_literal`, no numeric
/// formatting), cheap enough that `par_chunks`' thread wake/join cost exceeds
/// the compute saved. Toggle-isolated against the OTHER candidate cause (the
/// batch-buffer indirection itself): forcing every batch sequential while
/// LEAVING the buffering exactly as-is reproduced the pre-batch, zero-buffer
/// baseline almost exactly (within ~2%), which rules the buffer out — the
/// dispatch is the entire cost. See [`reconstruct_batch`]'s `parallel_allowed`
/// parameter for the fix: only [`rust_group_execute`]'s inner collection (the
/// `micro_distinct_agg` / `micro_group_avg_rust` shape this constant was
/// tuned against — `AVG`/`SUM(DISTINCT)`/`COUNT(DISTINCT)` over
/// `canonical_lexical`-formatted numeric literals) is allowed past this gate.
///
/// **Re-tested, Run 4 Wave C1, once [`Bindings`] made per-row reconstruction
/// leaner — inconclusive, gate KEPT.** The hypothesis: a cheaper per-row
/// build might shift the win/lose line for the dump path too. Re-running
/// `constant_memory_dump` itself with the plain streaming path's gate
/// temporarily forced `true` still regressed (+16.6%/+18.7% at 10x/100x,
/// same direction as the original 31-35% figure above) — but that bench
/// installs `sf-bench::mem::Tracking` as a global allocator (`mem.rs`) to
/// track peak BYTES via two process-wide atomics every alloc/dealloc
/// touches; under multi-threaded `par_chunks` dispatch those atomics see
/// real cross-core contention that a single-threaded run never does, which
/// is a property of THAT bench's own instrumentation, not of
/// `reconstruct_batch`. Re-running the SAME forced-`true` experiment on
/// `sf-bench`'s OTHER, uninstrumented CONSTRUCT-dump bench
/// (`obda_latency`'s `obda_construct_dump`, plain `System` allocator) gave
/// the OPPOSITE signal: no significant change at 1x (p > 0.05, both
/// replicates), but dispatch ~7-9% FASTER at 10x across two independent
/// same-session replicates (p < 0.05 both times). The gate stays `false`
/// here anyway: both readings came from one heavily-loaded shared 18-core
/// dev machine mid-swarm-session (`uptime` load average swung 11 → 5 during
/// this very testing), and `par_chunks`' win margin is inherently sensitive
/// to core contention in a way a quiet/dedicated re-run could easily
/// overturn — a hot path this wide (every CONSTRUCT dump and streaming
/// SELECT) deserves cleaner verification before its default flips. Left as
/// a concrete, evidenced follow-up: re-run `obda_construct_dump` (not
/// `constant_memory_dump`, per the confound above) on an idle machine before
/// deciding whether to un-gate.
const TERM_GEN_MIN_PARALLEL_ROWS: usize = 2_000;

/// The floor on `par_chunks`' chunk size WITHIN one already-dispatched batch
/// (see [`TERM_GEN_MIN_PARALLEL_ROWS`] for the separate whole-batch gate). Once a
/// batch is worth dispatching at all, a single `par_chunks` call's per-call
/// overhead is already paid — so this floor only needs to keep individual
/// chunks well above the measured-slower per-row granularity, not repeat
/// [`TERM_GEN_MIN_PARALLEL_ROWS`]'s much larger bar.
const TERM_GEN_MIN_CHUNK_ROWS: usize = 128;

/// Reconstruct every row of `batch` against `interned`'s bindings, in ORIGINAL row order —
/// [`run_branches`]'s buffer -> maybe-parallel-map -> emit-in-order step. A
/// plain sequential map when `!parallel_allowed` (ledger F8 — see
/// [`TERM_GEN_MIN_PARALLEL_ROWS`]'s doc comment for the measured reason a
/// caller may want this) or below [`TERM_GEN_MIN_PARALLEL_ROWS`] (a fresh
/// `par_chunks` dispatch's own overhead would dominate a batch this small —
/// see its doc comment for the measured break-even). Otherwise `batch` is
/// split into `rayon::current_num_threads()`-many chunks (floored at
/// [`TERM_GEN_MIN_CHUNK_ROWS`]) via `par_chunks`, and each chunk is
/// reconstructed sequentially by ONE rayon task — chunks run in parallel, but no
/// task is ever a single row (the measured-slower shape this restructure
/// replaces). `rayon`'s own lazily-initialized global pool is used directly (no
/// hand-rolled `ThreadPool`, never built per call) — separate from `tokio` by
/// construction (ADR-0006 pool separation), since it is a wholly different set
/// of OS threads.
///
/// Ordering: `par_chunks` is an `IndexedParallelIterator`, so mapping each chunk
/// to its own `Vec` and collecting preserves chunk order exactly; flattening
/// those chunk-`Vec`s then reproduces the sequential per-row order with no extra
/// bookkeeping — indexed chunks are the strict-order-preservation design this
/// restructure requires (downstream DISTINCT/ORDER BY/OFFSET/LIMIT all assume
/// original row order). This ordering guarantee holds regardless of
/// `parallel_allowed` — the sequential branch is already in order by
/// construction, so a caller never needs to know which branch ran.
fn reconstruct_batch(
    interned: &InternedBindings<'_>,
    batch: &[RawTuple],
    col_index: &ColIndex<'_>,
    parallel_allowed: bool,
) -> Vec<Result<Bindings>> {
    let one_row = |t: &RawTuple| {
        let raw = RawRow {
            values: &t.values,
            codes: &t.codes,
            index: col_index,
        };
        reconstruct(interned, &raw)
    };
    if !parallel_allowed || batch.len() < TERM_GEN_MIN_PARALLEL_ROWS {
        return batch.iter().map(one_row).collect();
    }
    use rayon::prelude::*;
    let chunk_size =
        (batch.len() / rayon::current_num_threads().max(1)).max(TERM_GEN_MIN_CHUNK_ROWS);
    batch
        .par_chunks(chunk_size)
        .map(move |chunk| chunk.iter().map(one_row).collect::<Vec<_>>())
        .collect::<Vec<_>>()
        .into_iter()
        .flatten()
        .collect()
}

/// SPARQL term order extended to a total order for sorting: blank node < IRI <
/// literal; within a kind by value.
fn cmp_term(a: &Term, b: &Term) -> Ordering {
    match (a, b) {
        (Term::BlankNode(x), Term::BlankNode(y)) => x.as_str().cmp(y.as_str()),
        (Term::NamedNode(x), Term::NamedNode(y)) => x.as_str().cmp(y.as_str()),
        (Term::Literal(x), Term::Literal(y)) => cmp_literal(x, y),
        _ => term_rank(a)
            .cmp(&term_rank(b))
            .then_with(|| a.to_string().cmp(&b.to_string())),
    }
}

/// [`cmp_term`]'s kind ordering, factored out so [`term_sort_key`] shares it.
fn term_rank(t: &Term) -> u8 {
    match t {
        Term::BlankNode(_) => 0,
        Term::NamedNode(_) => 1,
        Term::Literal(_) => 2,
        // Quoted triple (RDF-star / ADR-0032 D2's `Term::Triple`, including a
        // reconstructed `TermDef::ComposedTriple`) — SPARQL §15.1: triple
        // terms are the HIGHEST category, and order AMONG them is spec-
        // undefined. This engine's choice — sort last (this rank), by lexical
        // form (`cmp_term`'s wildcard tie-break / `TermSortKey::Other`) — is
        // therefore a PERMISSIBLE, merely DETERMINISTIC one, not a spec
        // requirement: ordering AMONG values sharing this rank is by the
        // triple's own `Display` (N-Triples-like) text, stable and repeatable
        // across runs (no hashing / no non-deterministic input anywhere in
        // this comparison), never mixed with a non-triple-term rank (a
        // `TermDef::ComposedTriple`-composed variable and an ordinary
        // variable can never land in the same result column — the
        // uniform-composedness law, `star::rewrite_union`'s doc comment — so
        // "highest category" never needs to interleave with another kind's
        // ordering here). See `differential_star.rs`'s
        // `order_by_composed_var_is_deterministic_across_runs` for the
        // end-to-end proof over a REAL env-composed `?t`.
        _ => 3,
    }
}

/// A [`Term`]'s [`cmp_term`]-relevant shape, precomputed ONCE per term rather than
/// re-derived on every comparison a sort makes (Schwartzian transform, ADR-0024/M4
/// perf). `BlankNode`/`NamedNode` borrow their `&str`; `Literal` borrows the whole
/// literal (its own comparison, `cmp_literal`, is already allocation-free). `Other`
/// (any kind besides those three — currently only a quoted triple, RDF-star) is the
/// ONLY variant that allocates, and does so HERE, once, instead of inside
/// `cmp_term`'s wildcard tie-break on every comparison it participates in.
enum TermSortKey<'a> {
    BlankNode(&'a str),
    NamedNode(&'a str),
    Literal(&'a Literal),
    Other(String),
}

/// Build a term's [`TermSortKey`]. See its doc comment for why this is where the
/// (possibly) allocating work happens.
fn term_sort_key(t: &Term) -> TermSortKey<'_> {
    match t {
        Term::BlankNode(n) => TermSortKey::BlankNode(n.as_str()),
        Term::NamedNode(n) => TermSortKey::NamedNode(n.as_str()),
        Term::Literal(l) => TermSortKey::Literal(l),
        other => TermSortKey::Other(other.to_string()),
    }
}

/// [`cmp_term`], comparing precomputed [`TermSortKey`]s instead of the `Term`s
/// directly. Byte-identical order to `cmp_term` by construction: the SAME three
/// same-kind arms (borrowed, not cloned data), and the SAME rank-then-lexical
/// wildcard tie-break — `term_rank` assigns each kind a UNIQUE rank except for
/// `Other`, so two keys only ever reach the tie-break when BOTH are `Other`
/// (whatever concrete non-Blank/Named/Literal `Term` variant they came from),
/// exactly the one case `cmp_term`'s own wildcard allocates for.
fn cmp_sort_key(a: &TermSortKey, b: &TermSortKey) -> Ordering {
    fn rank(k: &TermSortKey) -> u8 {
        match k {
            TermSortKey::BlankNode(_) => 0,
            TermSortKey::NamedNode(_) => 1,
            TermSortKey::Literal(_) => 2,
            TermSortKey::Other(_) => 3,
        }
    }
    match (a, b) {
        (TermSortKey::BlankNode(x), TermSortKey::BlankNode(y)) => x.cmp(y),
        (TermSortKey::NamedNode(x), TermSortKey::NamedNode(y)) => x.cmp(y),
        (TermSortKey::Literal(x), TermSortKey::Literal(y)) => cmp_literal(x, y),
        _ => rank(a).cmp(&rank(b)).then_with(|| match (a, b) {
            (TermSortKey::Other(x), TermSortKey::Other(y)) => x.cmp(y),
            // Same rank implies the same variant among Blank/Named/Literal/Other
            // (each of the first three has a UNIQUE rank, handled above), so a
            // tie here is only ever reached by two `Other`s.
            _ => unreachable!("equal TermSortKey rank implies both are Other"),
        }),
    }
}

/// Precompute one row's ORDER BY sort keys — one [`TermSortKey`] per [`OrderKey`]
/// in `order`, `None` for an unbound one — see [`order_cmp_precomputed`].
fn precompute_order_keys<'a>(
    order: &[OrderKey],
    bindings: &'a Bindings,
) -> Vec<Option<TermSortKey<'a>>> {
    order
        .iter()
        .map(|key| bindings.get(&key.var).map(term_sort_key))
        .collect()
}

/// Compare two solutions' PRECOMPUTED ORDER BY keys ([`precompute_order_keys`],
/// SPARQL §15.1), honoring each key's direction with explicit UNBOUND placement:
/// an unbound key sorts FIRST for ASC and LAST for DESC — matching the SQL `NULLS
/// FIRST/LAST` the single-branch path emits, so single- and multi-branch orderings
/// agree. Bound terms order blank-node < IRI < literal; numeric-typed literals
/// compare by value (so xsd:integer 2 < 10, not lexical "10" < "2") — see
/// [`cmp_sort_key`]. Used by the buffered ORDER BY sorts
/// ([`run_branches`]/[`rust_group_result_rows`]) with keys precomputed once per
/// row (a Schwartzian transform, ADR-0024/M4 perf), so an `n`-row sort computes
/// each term's (possibly allocating) fallback string O(n) times, not O(n log n).
fn order_cmp_precomputed(
    order: &[OrderKey],
    a: &[Option<TermSortKey>],
    b: &[Option<TermSortKey>],
) -> Ordering {
    for ((key, ka), kb) in order.iter().zip(a.iter()).zip(b.iter()) {
        let ord = match (ka, kb) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => {
                if key.descending {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (Some(_), None) => {
                if key.descending {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (Some(x), Some(y)) => {
                let c = cmp_sort_key(x, y);
                if key.descending {
                    c.reverse()
                } else {
                    c
                }
            }
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// Compare two literals: numerically when both carry a numeric XSD datatype, else
/// by lexical value, then datatype IRI, then language tag.
fn cmp_literal(x: &Literal, y: &Literal) -> Ordering {
    if let (Some(nx), Some(ny)) = (numeric_value(x), numeric_value(y)) {
        return nx.partial_cmp(&ny).unwrap_or(Ordering::Equal);
    }
    x.value()
        .cmp(y.value())
        .then_with(|| x.datatype().as_str().cmp(y.datatype().as_str()))
        .then_with(|| x.language().unwrap_or("").cmp(y.language().unwrap_or("")))
}

/// The `f64` value of a numeric-XSD-typed literal, else `None` (a non-numeric
/// datatype is ordered lexically, never coerced).
fn numeric_value(l: &Literal) -> Option<f64> {
    const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
    let local = l.datatype().as_str().strip_prefix(XSD)?;
    let numeric = matches!(
        local,
        "integer"
            | "decimal"
            | "double"
            | "float"
            | "long"
            | "int"
            | "short"
            | "byte"
            | "nonNegativeInteger"
            | "nonPositiveInteger"
            | "negativeInteger"
            | "positiveInteger"
            | "unsignedLong"
            | "unsignedInt"
            | "unsignedShort"
            | "unsignedByte"
    );
    if numeric {
        l.value().parse::<f64>().ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// SPARQL expression evaluator for ORDER BY expression keys
// ---------------------------------------------------------------------------
// Evaluates a SPARQL expression against a solution binding map, returning the
// result as an RDF term, or `None` on type error / unbound input. Covers the
// subset needed for ORDER BY expression keys: arithmetic, string built-ins,
// IF, BOUND, comparisons, COALESCE, boolean connectives. On unsupported
// sub-expressions returns None (unbound), which ORDER BY treats as sorting
// first/last per direction — sound but never silently wrong.

/// Evaluate a SPARQL expression to an RDF term, or `None` if indeterminate.
pub(crate) fn eval_expr(expr: &Expression, b: &Bindings) -> Option<Term> {
    match expr {
        Expression::Variable(v) => b.get(v.as_str()).cloned(),
        Expression::NamedNode(n) => Some(Term::NamedNode(n.clone())),
        Expression::Literal(l) => Some(Term::Literal(l.clone())),
        Expression::Bound(v) => {
            let bound = b.contains_key(v.as_str());
            Some(Term::Literal(Literal::new_typed_literal(
                if bound { "true" } else { "false" },
                sf_core::NamedNode::new("http://www.w3.org/2001/XMLSchema#boolean").ok()?,
            )))
        }
        Expression::If(cond, then, els) => {
            if eval_bool(cond, b)? {
                eval_expr(then, b)
            } else {
                eval_expr(els, b)
            }
        }
        Expression::Coalesce(args) => args.iter().find_map(|a| eval_expr(a, b)),
        Expression::Not(a) => {
            let v = eval_bool(a, b)?;
            bool_literal(!v)
        }
        Expression::And(a, c) => {
            let av = eval_bool(a, b)?;
            let bv = eval_bool(c, b)?;
            bool_literal(av && bv)
        }
        Expression::Or(a, c) => {
            let av = eval_bool(a, b)?;
            let bv = eval_bool(c, b)?;
            bool_literal(av || bv)
        }
        Expression::Equal(a, c) => {
            let cmp = cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref());
            bool_literal(matches!(cmp, Some(Ordering::Equal)))
        }
        Expression::Less(a, c) => bool_literal(matches!(
            cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref()),
            Some(Ordering::Less)
        )),
        Expression::Greater(a, c) => bool_literal(matches!(
            cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref()),
            Some(Ordering::Greater)
        )),
        Expression::LessOrEqual(a, c) => bool_literal(matches!(
            cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref()),
            Some(Ordering::Less | Ordering::Equal)
        )),
        Expression::GreaterOrEqual(a, c) => bool_literal(matches!(
            cmp_option(eval_expr(a, b).as_ref(), eval_expr(c, b).as_ref()),
            Some(Ordering::Greater | Ordering::Equal)
        )),
        Expression::Add(a, c) => num_binop(eval_expr(a, b)?, eval_expr(c, b)?, |x, y| x + y),
        Expression::Subtract(a, c) => num_binop(eval_expr(a, b)?, eval_expr(c, b)?, |x, y| x - y),
        Expression::Multiply(a, c) => num_binop(eval_expr(a, b)?, eval_expr(c, b)?, |x, y| x * y),
        Expression::Divide(a, c) => {
            let bv = term_to_f64(&eval_expr(c, b)?)?;
            if bv == 0.0 {
                return None;
            }
            num_binop(
                eval_expr(a, b)?,
                Term::Literal(Literal::new_simple_literal("0")),
                |x, _| x / bv,
            )
        }
        Expression::UnaryMinus(a) => {
            let v = term_to_f64(&eval_expr(a, b)?)?;
            f64_to_term(-v)
        }
        Expression::FunctionCall(func, args) => eval_function(func, args, b),
        _ => None,
    }
}

fn eval_bool(expr: &Expression, b: &Bindings) -> Option<bool> {
    match eval_expr(expr, b)? {
        Term::Literal(l) => {
            const XSD_BOOL: &str = "http://www.w3.org/2001/XMLSchema#boolean";
            if l.datatype().as_str() == XSD_BOOL {
                Some(l.value() == "true")
            } else {
                // Effective boolean value per SPARQL §17.2.2
                let v = l.value();
                Some(!v.is_empty() && v != "0" && v != "0.0" && v != "false")
            }
        }
        _ => None,
    }
}

fn bool_literal(v: bool) -> Option<Term> {
    Some(Term::Literal(Literal::new_typed_literal(
        if v { "true" } else { "false" },
        sf_core::NamedNode::new("http://www.w3.org/2001/XMLSchema#boolean").ok()?,
    )))
}

fn term_to_f64(t: &Term) -> Option<f64> {
    match t {
        Term::Literal(l) => l.value().parse::<f64>().ok(),
        _ => None,
    }
}

fn f64_to_term(v: f64) -> Option<Term> {
    let code = if v.fract() == 0.0 && v.abs() < 1e15 {
        XsdTypeCode::Integer
    } else {
        XsdTypeCode::Double
    };
    natural_literal(&v.to_string(), code).ok()
}

fn num_binop(a: Term, b: Term, op: impl Fn(f64, f64) -> f64) -> Option<Term> {
    let av = term_to_f64(&a)?;
    let bv = term_to_f64(&b)?;
    f64_to_term(op(av, bv))
}

fn cmp_option(a: Option<&Term>, b: Option<&Term>) -> Option<Ordering> {
    Some(cmp_term(a?, b?))
}

fn eval_function(func: &Function, args: &[Expression], b: &Bindings) -> Option<Term> {
    fn str_val(t: &Term) -> Option<String> {
        match t {
            Term::Literal(l) => Some(l.value().to_owned()),
            _ => None,
        }
    }
    match func {
        Function::StrLen => {
            let t = eval_expr(args.first()?, b)?;
            let s = str_val(&t)?;
            natural_literal(&s.chars().count().to_string(), XsdTypeCode::Integer).ok()
        }
        Function::UCase => {
            let t = eval_expr(args.first()?, b)?;
            Some(Term::Literal(Literal::new_simple_literal(
                str_val(&t)?.to_uppercase(),
            )))
        }
        Function::LCase => {
            let t = eval_expr(args.first()?, b)?;
            Some(Term::Literal(Literal::new_simple_literal(
                str_val(&t)?.to_lowercase(),
            )))
        }
        Function::Str => {
            let t = eval_expr(args.first()?, b)?;
            let s = match &t {
                Term::Literal(l) => l.value().to_owned(),
                Term::NamedNode(n) => n.as_str().to_owned(),
                _ => return None,
            };
            Some(Term::Literal(Literal::new_simple_literal(s)))
        }
        Function::Concat => {
            let mut result = String::new();
            for arg in args {
                let t = eval_expr(arg, b)?;
                result.push_str(&str_val(&t)?);
            }
            Some(Term::Literal(Literal::new_simple_literal(result)))
        }
        Function::Lang => {
            let t = eval_expr(args.first()?, b)?;
            let lang = match &t {
                Term::Literal(l) => l.language().unwrap_or("").to_owned(),
                _ => String::new(),
            };
            Some(Term::Literal(Literal::new_simple_literal(lang)))
        }
        Function::Datatype => {
            let t = eval_expr(args.first()?, b)?;
            match &t {
                Term::Literal(l) => Some(Term::NamedNode(
                    sf_core::NamedNode::new(l.datatype().as_str()).ok()?,
                )),
                _ => None,
            }
        }
        Function::Abs => {
            let t = eval_expr(args.first()?, b)?;
            let v = term_to_f64(&t)?;
            f64_to_term(v.abs())
        }
        Function::Floor => {
            let t = eval_expr(args.first()?, b)?;
            let v = term_to_f64(&t)?;
            f64_to_term(v.floor())
        }
        Function::Ceil => {
            let t = eval_expr(args.first()?, b)?;
            let v = term_to_f64(&t)?;
            f64_to_term(v.ceil())
        }
        Function::Round => {
            let t = eval_expr(args.first()?, b)?;
            let v = term_to_f64(&t)?;
            f64_to_term(v.round())
        }
        // ADR-0032 D3 item 3 — the five triple-term functions, operating on
        // an already-materialized `Term::Triple` value. `star::rewrite_expr`
        // resolves every statically-known case (an env-composed variable, or
        // a literal `TRIPLE(...)`/`<<(...)>>` operand) BEFORE this evaluator
        // ever runs, so these arms exist as a defense-in-depth fallback for
        // whatever reaches ORDER BY / post-GROUP-BY expression evaluation
        // unresolved. None-discipline audit (this wave): EVERY existing arm
        // above already uses `eval_expr(...)?` / `str_val(...)?` — an
        // unbound/wrong-shape operand makes the WHOLE function call `None`,
        // uniformly, for every function in this evaluator (not row-
        // eliminating or silently-skipped — the CALLER decides what `None`
        // means: `inject_order_expr_keys` leaves the ORDER BY key absent,
        // which `order_cmp_precomputed` sorts first/last per direction — a
        // documented, sound simplification, see this evaluator's module doc
        // — and `rust_group_result_rows`'s post-agg-expression path leaves
        // the target variable genuinely UNBOUND, the exact §10 ASSIGN
        // "expression error ⇒ unbound" behavior). These new arms follow the
        // SAME convention (R5: never silently produce a WRONG bound value —
        // §17.4.6's error is `None` here, exactly like every other function's
        // type error already is).
        Function::Subject => match eval_expr(args.first()?, b)? {
            Term::Triple(t) => Some(t.subject.into()),
            _ => None,
        },
        Function::Predicate => match eval_expr(args.first()?, b)? {
            Term::Triple(t) => Some(Term::NamedNode(t.predicate)),
            _ => None,
        },
        Function::Object => match eval_expr(args.first()?, b)? {
            Term::Triple(t) => Some(t.object),
            _ => None,
        },
        // §17.4.6 asymmetry: isTRIPLE never errors on a WRONG-KIND argument —
        // but (like every other function here) an argument that fails to
        // EVALUATE at all (`eval_expr` returns `None`, e.g. references an
        // unbound variable) still makes the whole call `None`, consistent
        // with this evaluator's uniform convention above; `star::rewrite_expr`
        // is what gives isTRIPLE its full always-a-value spec semantics
        // (resolved statically, never reaching here for the common case).
        Function::IsTriple => {
            let t = eval_expr(args.first()?, b)?;
            bool_literal(matches!(t, Term::Triple(_)))
        }
        Function::Triple => {
            let [s, p, o] = args else { return None };
            let s = eval_expr(s, b)?;
            let p = eval_expr(p, b)?;
            let o = eval_expr(o, b)?;
            Triple::from_terms(s, p, o).ok().map(Term::from)
        }
        _ => None,
    }
}

/// A SELECT solution: the projected variables (plan order) paired with each
/// row's bound terms (`None` = unbound).
pub struct Solutions {
    pub vars: Vec<String>,
    pub rows: Vec<Vec<Option<Term>>>,
}

/// Instantiate a CONSTRUCT-template triple against a solution; `None` if any
/// variable is unbound or the triple would be ill-formed (SPARQL §16.2: an
/// ill-formed instantiation is silently dropped, never an error). `pub(crate)`
/// so the PostgreSQL executor instantiates CONSTRUCT templates identically.
pub(crate) fn instantiate(
    tp: &spargebra::term::TriplePattern,
    bindings: &Bindings,
) -> Option<Triple> {
    use spargebra::term::NamedNodePattern;
    let subject = instantiate_term(&tp.subject, bindings)?;
    let predicate = match &tp.predicate {
        NamedNodePattern::NamedNode(n) => Term::NamedNode(n.clone()),
        NamedNodePattern::Variable(v) => bindings.get(v.as_str()).cloned()?,
    };
    let object = instantiate_term(&tp.object, bindings)?;
    Triple::from_terms(subject, predicate, object).ok()
}

/// A CONSTRUCT-template term slot → its bound `Term`, or `None` if unbound /
/// ill-formed. `TermPattern::Triple` (ADR-0032 D2) recurses — a nested quoted
/// triple in a template (`star::substitute_construct_template` is the ONLY
/// producer of this shape in a template today, but the arm is general) builds
/// its own s/p/o first, bottom-up, then composes via `Triple::from_terms`,
/// whose fallibility naturally enforces RDF 1.2 §3.1 position legality — an
/// illegal-position nested triple silently drops (§16.2), never errors. A
/// standalone (non-closure) function so it can recurse into itself.
fn instantiate_term(p: &spargebra::term::TermPattern, bindings: &Bindings) -> Option<Term> {
    use spargebra::term::TermPattern;
    match p {
        TermPattern::Variable(v) => bindings.get(v.as_str()).cloned(),
        TermPattern::NamedNode(n) => Some(Term::NamedNode(n.clone())),
        TermPattern::Literal(l) => Some(Term::Literal(l.clone())),
        TermPattern::BlankNode(b) => Some(Term::BlankNode(b.clone())),
        TermPattern::Triple(inner) => {
            let s = instantiate_term(&inner.subject, bindings)?;
            let p = match &inner.predicate {
                spargebra::term::NamedNodePattern::NamedNode(n) => Term::NamedNode(n.clone()),
                spargebra::term::NamedNodePattern::Variable(v) => {
                    bindings.get(v.as_str()).cloned()?
                }
            };
            let o = instantiate_term(&inner.object, bindings)?;
            Triple::from_terms(s, p, o).ok().map(Term::from)
        }
    }
}

// ---------------------------------------------------------------------------
// Rust-level GROUP BY (multi-branch inner, SPARQL §11)
// ---------------------------------------------------------------------------

/// Group a collected multiset of inner solutions by `rg.keys`, compute each
/// aggregate in `rg.aggs`, then apply the plan's ORDER BY + OFFSET/LIMIT to the
/// grouped rows (SPARQL §15: order, then slice). Returns the final result rows in
/// emit order.
///
/// Shared by the SQLite ([`rust_group_execute`]) and PostgreSQL
/// ([`crate::exec_pg`]) multi-branch GROUP BY paths (ADR-0007): the
/// grouping/aggregation semantics are backend-independent — only the collection of
/// the inner solutions (SQLite `Connection` vs live PostgreSQL cursor) differs.
pub(crate) fn rust_group_result_rows(
    plan: &Plan,
    rg: &RustGroup,
    inner_rows: Vec<Bindings>,
) -> Result<Vec<Bindings>> {
    // Group by the key variable values, preserving insertion order for stable output.
    // Use a Vec for ordering + a HashMap for O(1) group lookup.
    type GroupKey = Vec<Option<Term>>;
    type GroupRows = Vec<Bindings>;
    #[allow(clippy::type_complexity)]
    let mut groups: Vec<(GroupKey, GroupRows)> = Vec::new();
    let mut key_index: std::collections::HashMap<Vec<Option<Term>>, usize> =
        std::collections::HashMap::new();

    for row in inner_rows {
        let key: Vec<Option<Term>> = rg.keys.iter().map(|k| row.get(k).cloned()).collect();
        if let Some(&idx) = key_index.get(&key) {
            groups[idx].1.push(row);
        } else {
            let idx = groups.len();
            key_index.insert(key.clone(), idx);
            groups.push((key, vec![row]));
        }
    }

    // Implicit grouping (no key variables): always produce exactly one group,
    // even over an empty inner (COUNT(*) ⇒ 0, AVG/MIN/MAX ⇒ UNBOUND — §11).
    if rg.keys.is_empty() && groups.is_empty() {
        groups.push((vec![], vec![]));
    }

    // Every result row below binds the SAME `rg.keys`/agg `out_var`s/post-expr
    // `out_var`s, once per GROUP — interned ONCE here, not per group (Run 4
    // Wave C1, the same "once per row stream" idiom `intern_bindings` uses
    // for a branch's own vars): a shared `Arc<str>` clone beats a fresh
    // `String` allocation per group.
    let key_names: Vec<Arc<str>> = rg.keys.iter().map(|k| Arc::from(k.as_str())).collect();
    let agg_names: Vec<Arc<str>> = rg
        .aggs
        .iter()
        .map(|a| Arc::from(a.out_var.as_str()))
        .collect();
    let post_names: Vec<Arc<str>> = rg
        .post_exprs
        .iter()
        .map(|(v, _)| Arc::from(v.as_str()))
        .collect();

    // Materialise the result row (key vars + aggregates) for every group.
    let mut result_rows: Vec<Bindings> = Vec::with_capacity(groups.len());
    for (key_vals, group_rows) in &groups {
        let mut result = Bindings::new();
        for (name, val) in key_names.iter().zip(key_vals.iter()) {
            if let Some(t) = val {
                result.insert(name.clone(), t.clone());
            }
        }
        for (agg_spec, name) in rg.aggs.iter().zip(&agg_names) {
            if let Some(t) = rust_agg(agg_spec, group_rows)? {
                result.insert(name.clone(), t);
            }
        }
        // ADR-0025 Tier-2 gap 5: post-GROUP-BY expressions over the aggregate outputs
        // (e.g. `COUNT(?x) * 2`). Evaluate each over the row's now-materialised aggregate +
        // group-key bindings via the shared `eval_expr`; an unbound reference yields no
        // binding (SPARQL: the value is unbound), never a wrong answer.
        for ((_, expr), name) in rg.post_exprs.iter().zip(&post_names) {
            if let Some(t) = eval_expr(expr, &result) {
                result.insert(name.clone(), t);
            }
        }
        result_rows.push(result);
    }

    // ORDER BY over the grouped rows (if requested), then OFFSET/LIMIT. Schwartzian
    // transform (ADR-0024/M4 perf, see `order_cmp_precomputed`): precompute each
    // row's sort keys once, sort a permutation of INDICES by them (keeps the keys'
    // borrow of `result_rows` and the final move out of it both sound), then
    // reorder `result_rows` by that permutation — moving each row exactly once
    // (`Option::take`), never cloning it.
    if !plan.order.is_empty() {
        let keys: Vec<Vec<Option<TermSortKey>>> = result_rows
            .iter()
            .map(|r| precompute_order_keys(&plan.order, r))
            .collect();
        let mut idx: Vec<usize> = (0..result_rows.len()).collect();
        idx.sort_by(|&i, &j| order_cmp_precomputed(&plan.order, &keys[i], &keys[j]));
        drop(keys);
        let mut slots: Vec<Option<Bindings>> = result_rows.into_iter().map(Some).collect();
        result_rows = idx
            .into_iter()
            .map(|i| {
                slots[i]
                    .take()
                    .expect("permutation index used exactly once")
            })
            .collect();
    }
    let take = plan.limit.unwrap_or(usize::MAX);
    Ok(result_rows
        .into_iter()
        .skip(plan.offset)
        .take(take)
        .collect())
}

/// Compute one aggregate over a group of solutions. Returns `None` for
/// UNBOUND (AVG/MIN/MAX over an empty multiset — SPARQL §11).
fn rust_agg(agg: &RustAgg, rows: &[Bindings]) -> Result<Option<Term>> {
    match agg.kind {
        AggKind::Count => {
            let count = match &agg.arg_var {
                // COUNT(DISTINCT *) — count DISTINCT whole solutions in the group. A row's
                // canonical key is its (var, term) pairs via `canonical_pairs` (Run 4 Wave C1:
                // `Bindings` preserves insertion order, not the old `BTreeMap`'s sorted-key
                // order, so the key must be canonicalized explicitly — see its doc comment;
                // `BTreeMap` used to give this order-independence for free). `oxrdf::Term`
                // derives `Hash`/`Eq` (already relied on elsewhere in this file, e.g.
                // `seen_tuples` above), and N-Triples serialisation is injective, so a
                // `&Term`-keyed dedup set yields the IDENTICAL classes a `Term::to_string()`-
                // keyed one would — without the per-value allocation (ADR-0025 Tier-2 gap 3;
                // ADR-0024/M4 perf).
                None if agg.distinct => {
                    let mut seen: std::collections::HashSet<Vec<(&str, &Term)>> =
                        std::collections::HashSet::new();
                    rows.iter()
                        .filter(|r| seen.insert(canonical_pairs(r)))
                        .count()
                }
                None => rows.len(), // COUNT(*)
                Some(var) => {
                    if agg.distinct {
                        let mut seen: std::collections::HashSet<&Term> =
                            std::collections::HashSet::new();
                        rows.iter()
                            .filter_map(|r| r.get(var))
                            .filter(|t| seen.insert(*t))
                            .count()
                    } else {
                        rows.iter().filter(|r| r.contains_key(var.as_str())).count()
                    }
                }
            };
            Ok(Some(Term::Literal(Literal::new_typed_literal(
                count.to_string(),
                sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
            ))))
        }
        AggKind::Sum => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            // ADR-0025 C.5: SUM/AVG/MIN/MAX PROPAGATE an unbound operand — a NON-empty group
            // with ANY row whose operand var is unbound ⇒ the whole aggregate is UNBOUND
            // (SPARQL §11; spareval-confirmed). Only COUNT filters errors. This extends C.4,
            // which handled only the all-unbound case for AVG; the mixed bound+unbound group
            // (and SUM over all-unbound) was still wrongly computed over just the bound rows.
            if !rows.is_empty() && rows.iter().any(|r| r.get(var).is_none()) {
                return Ok(None);
            }
            let vals: Vec<&Term> = dedup_if_distinct(
                rows.iter().filter_map(|r| r.get(var)).collect(),
                agg.distinct,
            );
            if vals.is_empty() {
                // SUM over empty multiset ⇒ "0"^^xsd:integer (SPARQL §11).
                return Ok(Some(Term::Literal(Literal::new_typed_literal(
                    "0",
                    sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
                ))));
            }
            let nums: Vec<f64> = vals.iter().filter_map(|t| numeric_term(t)).collect();
            if nums.len() < vals.len() {
                return Ok(None); // non-numeric operand ⇒ UNBOUND (type error)
            }
            let sum: f64 = nums.iter().sum();
            // SPARQL §11.4 / XPath numeric type promotion: any xsd:double operand ⇒ double
            // result; else all-integer ⇒ integer; else decimal. (C.6b: rust_agg previously
            // always emitted integer-or-decimal, losing xsd:double — a datatype =_bag
            // divergence exposed once C.6 routed nullable double-operand SUMs here.)
            if vals.iter().any(|t| is_xsd_double(t)) {
                Ok(Some(double_term(sum)?))
            } else if vals.iter().all(|t| is_xsd_integer(t)) {
                Ok(Some(integer_term(sum as i64)))
            } else {
                Ok(Some(decimal_term(sum)?))
            }
        }
        AggKind::Avg => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            // ADR-0025 C.5 (see SUM): any unbound operand row in a non-empty group ⇒ UNBOUND.
            if !rows.is_empty() && rows.iter().any(|r| r.get(var).is_none()) {
                return Ok(None);
            }
            let vals: Vec<&Term> = dedup_if_distinct(
                rows.iter().filter_map(|r| r.get(var)).collect(),
                agg.distinct,
            );
            if vals.is_empty() {
                // ADR-0025 C.4: AVG over no bound values. If the GROUP is genuinely EMPTY
                // (0 rows — e.g. implicit grouping over an unmatched pattern), AVG ⇒
                // "0"^^xsd:integer (SPARQL §11, like SUM; spareval-confirmed). But if the
                // group HAS rows and the operand is simply UNBOUND in every one of them
                // (e.g. `AVG(?missing)` over a UNION arm that never binds it), there are no
                // numeric values to average ⇒ the result is UNBOUND, NOT 0. The old
                // `vals.is_empty()` conflated these two — discriminate on `rows`.
                return if rows.is_empty() {
                    Ok(Some(integer_term(0)))
                } else {
                    Ok(None)
                };
            }
            // SPARQL §11.4: AVG of xsd:double values stays xsd:double (else decimal). See the
            // SUM promotion note above (C.6b) — mirrors the SQL path's `avg_result_code`.
            //
            // ADR-0025 C.10: both arms below gate on `nums.len() < vals.len()`, NOT
            // `nums.is_empty()` — the same non-numeric-operand check `AggKind::Sum` already
            // uses above. The `is_empty()` form only caught an ALL-non-numeric group; a group
            // MIXING numeric and non-numeric operands (e.g. a UNION arm binding the same var
            // to a plain string) had `numeric_term`/`decimal_term_value`'s `filter_map` quietly
            // drop the non-numeric ones and average just the numeric-parseable SUBSET — a real
            // `=_bag` wrong answer per SPARQL §11 (Avg via Sum: ANY non-numeric operand errors
            // the whole aggregate, spareval-confirmed), previously tracked as a deliberate,
            // separate residue (ADR-0025 progress log, 2026-07-18 addendum). SUM never had this
            // gap; AVG's two branches independently repeat the mistake.
            if vals.iter().any(|t| is_xsd_double(t)) {
                let nums: Vec<f64> = vals.iter().filter_map(|t| numeric_term(t)).collect();
                if nums.len() < vals.len() {
                    return Ok(None); // non-numeric operand ⇒ UNBOUND (type error, §11)
                }
                let avg = nums.iter().sum::<f64>() / nums.len() as f64;
                Ok(Some(double_term(avg)?))
            } else {
                // M3 fix 1: every remaining operand is xsd:integer/xsd:decimal (the
                // xsd:double case already returned above), so accumulate with
                // `oxsdatatypes::Decimal` — exact i128 fixed-point, NEVER `f64` — instead of
                // the old `nums.iter().sum::<f64>() / len`. A non-terminating quotient (e.g.
                // 11/3) rendered as an f64 artifact ("3.6666666666666665") that diverged from
                // the spareval oracle's own exact decimal AVG ("3.666666666666666666"): same
                // `oxsdatatypes::Decimal` type on both sides ⇒ =_bag equality.
                let nums: Vec<Decimal> =
                    vals.iter().filter_map(|t| decimal_term_value(t)).collect();
                if nums.len() < vals.len() {
                    return Ok(None); // non-numeric operand ⇒ UNBOUND (type error, §11)
                }
                let Some(sum) = nums
                    .iter()
                    .try_fold(Decimal::from(0_i64), |acc, &d| acc.checked_add(d))
                else {
                    return Ok(None); // FOAR0002 overflow ⇒ UNBOUND (never a wrong answer)
                };
                match sum.checked_div(nums.len() as i64) {
                    Some(avg) => Ok(Some(decimal_term_exact(avg)?)),
                    None => Ok(None), // FOAR0001/FOAR0002 ⇒ UNBOUND
                }
            }
        }
        AggKind::Min | AggKind::Max => {
            let Some(var) = &agg.arg_var else {
                return Ok(None);
            };
            // ADR-0025 C.5 (see SUM): any unbound operand row in a non-empty group ⇒ UNBOUND.
            if !rows.is_empty() && rows.iter().any(|r| r.get(var).is_none()) {
                return Ok(None);
            }
            // NOTE (ADR-0025 C.8): `agg.distinct` is deliberately NOT applied here — deduping
            // the multiset before MIN/MAX cannot change the result (the minimum/maximum of a
            // set equals that of the multiset it came from), unlike SUM/AVG (see
            // `dedup_if_distinct` below), so `MIN(DISTINCT ?v)`/`MAX(DISTINCT ?v)` are already
            // correct without special-casing `distinct`.
            let vals: Vec<&Term> = rows.iter().filter_map(|r| r.get(var)).collect();
            if vals.is_empty() {
                return Ok(None); // UNBOUND for empty multiset (§11)
            }
            let result = if agg.kind == AggKind::Min {
                vals.iter().min_by(|a, b| cmp_term(a, b))
            } else {
                vals.iter().max_by(|a, b| cmp_term(a, b))
            };
            Ok(result.map(|t| (*t).clone()))
        }
    }
}

/// ADR-0025 C.8: dedup a `rust_agg` operand multiset when `SUM(DISTINCT ?v)`/`AVG(DISTINCT
/// ?v)` requires it (SPARQL §11 — DISTINCT reduces the aggregate's input multiset to a SET
/// before applying the set function). `RustAgg.distinct` was previously read only by `Count`;
/// `Sum`/`Avg` silently ignored it and double-counted duplicate rows, a real `=_bag` wrong
/// answer (the SQL-pushdown sibling, `emit.rs`'s `agg_expr_sql`, already renders `SUM(DISTINCT
/// col)` correctly — only this in-process path had the gap). Canonicalises on `Term`'s own
/// `Hash`/`Eq` (structural equality agrees with N-Triples lexical equality, so this is the
/// SAME dedup key the `COUNT(DISTINCT …)` branches above use, just without their `to_string()`
/// allocation — ADR-0024/M4 perf), so dedup is order-independent and consistent across every
/// aggregate. No-op (returns `vals` unchanged) when `distinct` is false.
fn dedup_if_distinct(vals: Vec<&Term>, distinct: bool) -> Vec<&Term> {
    if !distinct {
        return vals;
    }
    let mut seen: std::collections::HashSet<&Term> = std::collections::HashSet::new();
    vals.into_iter().filter(|t| seen.insert(*t)).collect()
}

/// Extract the `f64` numeric value of an RDF term (returns `None` for
/// non-numeric-typed literals and non-literals).
fn numeric_term(t: &Term) -> Option<f64> {
    match t {
        Term::Literal(l) => numeric_value(l),
        _ => None,
    }
}

/// Extract the EXACT `oxsdatatypes::Decimal` value of an RDF term (M3 fix 1):
/// the same "numeric literal" gate as [`numeric_term`] (so a non-numeric operand
/// is rejected identically), but parsed WITHOUT ever going through `f64` —
/// `Decimal::from_str` reads the literal's own lexical digits directly, so a
/// non-terminating AVG quotient stays exact end to end. Only reached once the
/// `xsd:double`/`xsd:float` case has already been handled elsewhere, so the
/// lexical form here is always plain-digit (never `E`-notation).
fn decimal_term_value(t: &Term) -> Option<Decimal> {
    match t {
        Term::Literal(l) if numeric_value(l).is_some() => l.value().parse().ok(),
        _ => None,
    }
}

/// Whether an RDF term is an `xsd:integer`-typed literal.
fn is_xsd_integer(t: &Term) -> bool {
    match t {
        Term::Literal(l) => l.datatype().as_str() == "http://www.w3.org/2001/XMLSchema#integer",
        _ => false,
    }
}

/// Whether an RDF term is an `xsd:double` (or `xsd:float`, which this codebase folds into
/// double) literal — the promotion signal for SUM/AVG result typing (SPARQL §11.4 / XPath
/// numeric type promotion: any `double` operand makes the aggregate result `double`).
fn is_xsd_double(t: &Term) -> bool {
    match t {
        Term::Literal(l) => matches!(
            l.datatype().as_str(),
            "http://www.w3.org/2001/XMLSchema#double" | "http://www.w3.org/2001/XMLSchema#float"
        ),
        _ => false,
    }
}

/// Build a canonical `xsd:double` literal from an `f64` (via the shared canonicaliser — the
/// oracle's `oxsdatatypes` library — so the lexical form matches, e.g. `1.0E1`).
fn double_term(n: f64) -> Result<Term> {
    natural_literal(&format!("{n}"), XsdTypeCode::Double)
}

/// Build an `xsd:integer` literal from an `i64`.
fn integer_term(n: i64) -> Term {
    Term::Literal(Literal::new_typed_literal(
        n.to_string(),
        sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"),
    ))
}

/// Build an `xsd:decimal` literal from an `f64`.
fn decimal_term(n: f64) -> Result<Term> {
    // Compact (non-scientific) representation, then run it through the shared XSD-decimal
    // canonicaliser (`oxsdatatypes::Decimal`, the SAME library the oxigraph oracle uses) so
    // the lexical form is canonical and oracle-matching: an integral value renders "30", not
    // "30.0" (RDF term equality is by lexical form, so "30.0"^^decimal ≠ "30"^^decimal would
    // be a =_bag divergence), and trailing zeros trim ("1.50" → "1.5"). Consistent with the
    // SUM / `natural_literal` reconstruction path.
    let raw = if n.fract() == 0.0 {
        format!("{n:.1}")
    } else {
        format!("{n}")
    };
    natural_literal(&raw, XsdTypeCode::Decimal)
}

/// Build an `xsd:decimal` literal from an EXACT `oxsdatatypes::Decimal` (M3 fix 1's
/// AVG accumulator — never route through `f64`, which cannot represent a non-terminating
/// quotient like 11/3 exactly). `Decimal`'s own `Display` is ALREADY XSD-canonical
/// (`sf-core/datatype.rs` module doc), so this round-trips through `natural_literal`
/// exactly like [`decimal_term`]/[`double_term`], for consistency, not because
/// canonicalisation is needed here.
fn decimal_term_exact(d: Decimal) -> Result<Term> {
    natural_literal(&d.to_string(), XsdTypeCode::Decimal)
}

// --- M2 Send-future / GAT monomorphization gate (design §1 line 103, §5 M2) ----

#[cfg(test)]
mod probe_backend {
    //! Fail-fast device: a throwaway `'static` backend that proves AFIT + GAT + the
    //! generic async sink monomorphize to a **`Send`** future — the one novel
    //! language-feature risk — before any live-DB adapter is exercised.
    use super::*;
    use sf_sql::{BranchStream, RawTuple, SqlBackend};

    struct MockBackend {
        rows: Vec<RawTuple>,
    }
    struct MockStream {
        iter: std::vec::IntoIter<RawTuple>,
    }
    impl BranchStream for MockStream {
        async fn next_row(&mut self) -> sf_sql::Result<Option<RawTuple>> {
            Ok(self.iter.next())
        }
    }
    impl SqlBackend for MockBackend {
        type Stream<'s>
            = MockStream
        where
            Self: 's;
        async fn column_names(&mut self, _probe: &str) -> sf_sql::Result<Vec<String>> {
            Ok(Vec::new())
        }
        async fn open_branch(
            &mut self,
            _sql: &str,
            _params: &[String],
        ) -> sf_sql::Result<MockStream> {
            Ok(MockStream {
                iter: std::mem::take(&mut self.rows).into_iter(),
            })
        }
    }

    fn assert_send<T: Send>(t: T) -> T {
        t
    }

    #[test]
    fn send_future_monomorphizes_and_spawns() {
        // Monomorphizes `run::<MockBackend>` (here: `ask`) and proves the future is
        // `Send + 'static` enough to `tokio::spawn` — the M2 exit gate half (ii).
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let plan = Plan {
            branches: Vec::new(),
            form: PlanForm::Ask,
            distinct: false,
            limit: None,
            offset: 0,
            order: Vec::new(),
            rust_group: None,
            dialect: Dialect::Sqlite,
            dedup_groups: std::collections::HashMap::new(),
            construct_drops_some_branch_var: false,
        };
        rt.block_on(async move {
            let backend = MockBackend { rows: Vec::new() };
            let fut = async move {
                let mut b = backend;
                ask(&plan, &mut b).await
            };
            let joined = tokio::spawn(assert_send(fut)).await.unwrap();
            assert!(!joined.unwrap());
        });
    }
}

// --- Schwartzian-transform ORDER BY sort order-identity gate (ADR-0024/M4 perf) ---

#[cfg(test)]
mod order_sort_key_tests {
    //! Locks the Schwartzian-transform ORDER BY refactor (`TermSortKey` /
    //! `cmp_sort_key` / `order_cmp_precomputed`): a mixed vector of every term
    //! kind — IRIs, literals, blank nodes, quoted triples (RDF-star; only same-kind
    //! pairs of THESE ever reach the allocating tie-break) — sorts IDENTICALLY
    //! through the reference [`cmp_term`] (which allocates a fallback string per
    //! comparison it needs one) and the new precomputed path (which allocates it
    //! once per term, ever).
    use super::*;
    use sf_core::{BlankNode, NamedNode, Triple};

    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new_unchecked(s))
    }
    fn lit(s: &str) -> Term {
        Term::Literal(Literal::new_simple_literal(s))
    }
    fn bnode(s: &str) -> Term {
        Term::BlankNode(BlankNode::new_unchecked(s))
    }
    fn triple(s: &str) -> Term {
        Term::Triple(Box::new(Triple::new(
            NamedNode::new_unchecked(s),
            NamedNode::new_unchecked("http://ex.org/p"),
            NamedNode::new_unchecked("http://ex.org/o"),
        )))
    }

    fn mixed_terms() -> Vec<Term> {
        vec![
            iri("http://b.example/2"),
            lit("zzz"),
            bnode("b2"),
            triple("http://s/2"),
            iri("http://a.example/1"),
            lit("aaa"),
            bnode("b1"),
            triple("http://s/1"),
            triple("http://s/1"), // duplicate — exercises stability
            lit("aaa"),           // duplicate literal
        ]
    }

    #[test]
    fn precomputed_sort_matches_cmp_term_reference() {
        let terms = mixed_terms();

        // Reference: the original per-comparison comparator, unchanged.
        let mut via_cmp_term = terms.clone();
        via_cmp_term.sort_by(cmp_term);

        // New: precompute each term's sort key ONCE, then sort via the keys.
        let keys: Vec<TermSortKey> = terms.iter().map(term_sort_key).collect();
        let mut idx: Vec<usize> = (0..terms.len()).collect();
        idx.sort_by(|&i, &j| cmp_sort_key(&keys[i], &keys[j]));
        let via_precomputed: Vec<Term> = idx.into_iter().map(|i| terms[i].clone()).collect();

        assert_eq!(via_cmp_term, via_precomputed);
    }

    /// The same equivalence one layer up, at [`order_cmp_precomputed`] — the
    /// actual multi-row, `OrderKey`-driven machinery `run_branches` /
    /// `rust_group_result_rows` call — including an UNBOUND row (no "v" binding),
    /// exercising the `None`-placement arms `cmp_sort_key` alone doesn't cover.
    /// The reference comparator is `order_cmp`'s original body, reimplemented
    /// here directly over the unchanged [`cmp_term`] — `order_cmp` itself was
    /// superseded (both its call sites now use the precomputed path) and removed,
    /// so this stands in for it per the fix's own "reimplement the old comparator
    /// in the test" instruction.
    #[test]
    fn order_cmp_precomputed_matches_reference_over_solutions() {
        let order = vec![OrderKey {
            var: "v".to_owned(),
            descending: false,
            expr: None,
        }];
        let mut rows: Vec<Bindings> = mixed_terms()
            .into_iter()
            .map(|t| {
                let mut m = Bindings::new();
                m.insert(Arc::from("v"), t);
                m
            })
            .collect();
        rows.push(Bindings::new()); // UNBOUND — no "v" key

        let reference_cmp = |a: &Bindings, b: &Bindings| {
            for key in &order {
                let ord = match (a.get(&key.var), b.get(&key.var)) {
                    (None, None) => Ordering::Equal,
                    (None, Some(_)) => Ordering::Less,
                    (Some(_), None) => Ordering::Greater,
                    (Some(x), Some(y)) => cmp_term(x, y),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            Ordering::Equal
        };
        let mut via_reference = rows.clone();
        via_reference.sort_by(reference_cmp);

        let keys: Vec<Vec<Option<TermSortKey>>> = rows
            .iter()
            .map(|r| precompute_order_keys(&order, r))
            .collect();
        let mut idx: Vec<usize> = (0..rows.len()).collect();
        idx.sort_by(|&i, &j| order_cmp_precomputed(&order, &keys[i], &keys[j]));
        let via_precomputed: Vec<Bindings> = idx.into_iter().map(|i| rows[i].clone()).collect();

        assert_eq!(via_reference, via_precomputed);
    }
}

// --- ADR-0032 D3 item 3: eval_function's triple-term runtime arms ----------

#[cfg(test)]
mod triple_function_tests {
    //! `eval_expr`/`eval_function`'s Subject/Predicate/Object/IsTriple/Triple
    //! arms, operating on an already-materialized `Term::Triple` — the
    //! defense-in-depth fallback for whatever `star::rewrite_expr` did not
    //! resolve statically (ORDER BY / post-GROUP-BY expression evaluation
    //! only; see those arms' own doc comment for the full None-discipline).
    use super::*;
    use sf_core::NamedNode;
    use spargebra::algebra::Function;
    use spargebra::term::Variable;

    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new_unchecked(s))
    }
    fn triple_term(s: &str, p: &str, o: Term) -> Term {
        Term::Triple(Box::new(Triple::new(
            NamedNode::new_unchecked(s),
            NamedNode::new_unchecked(p),
            o,
        )))
    }
    fn bindings_with(var: &str, t: Term) -> Bindings {
        let mut b = Bindings::new();
        b.insert(Arc::from(var), t);
        b
    }
    fn call(f: Function, var: &str) -> Expression {
        Expression::FunctionCall(f, vec![Expression::Variable(Variable::new(var).unwrap())])
    }

    #[test]
    fn subject_predicate_object_extract_components_from_a_materialized_triple() {
        let t = triple_term("http://ex/s", "http://ex/p", iri("http://ex/o"));
        let b = bindings_with("t", t);
        assert_eq!(
            eval_expr(&call(Function::Subject, "t"), &b),
            Some(iri("http://ex/s"))
        );
        assert_eq!(
            eval_expr(&call(Function::Predicate, "t"), &b),
            Some(iri("http://ex/p"))
        );
        assert_eq!(
            eval_expr(&call(Function::Object, "t"), &b),
            Some(iri("http://ex/o"))
        );
    }

    #[test]
    fn subject_predicate_object_error_on_a_non_triple_is_none() {
        let b = bindings_with("t", iri("http://ex/plain"));
        assert_eq!(eval_expr(&call(Function::Subject, "t"), &b), None);
        assert_eq!(eval_expr(&call(Function::Predicate, "t"), &b), None);
        assert_eq!(eval_expr(&call(Function::Object, "t"), &b), None);
    }

    #[test]
    fn is_triple_never_errors_true_and_false_cases() {
        let triple_b = bindings_with(
            "t",
            triple_term("http://ex/s", "http://ex/p", iri("http://ex/o")),
        );
        let plain_b = bindings_with("t", iri("http://ex/plain"));
        let true_lit = eval_expr(&call(Function::IsTriple, "t"), &triple_b);
        let false_lit = eval_expr(&call(Function::IsTriple, "t"), &plain_b);
        assert_eq!(true_lit, bool_literal(true));
        assert_eq!(false_lit, bool_literal(false));
    }

    #[test]
    fn triple_function_composes_legal_components_and_drops_illegal_ones() {
        let e = Expression::FunctionCall(
            Function::Triple,
            vec![
                Expression::NamedNode(NamedNode::new_unchecked("http://ex/s")),
                Expression::NamedNode(NamedNode::new_unchecked("http://ex/p")),
                Expression::NamedNode(NamedNode::new_unchecked("http://ex/o")),
            ],
        );
        let b = Bindings::new();
        assert_eq!(
            eval_expr(&e, &b),
            Some(triple_term(
                "http://ex/s",
                "http://ex/p",
                iri("http://ex/o")
            ))
        );

        // A literal in SUBJECT position is illegal (RDF 1.2 §3.1: subject
        // must be IRI/bnode) — `Triple::from_terms` rejects it, so the whole
        // call is `None` (never a malformed Term::Triple).
        let illegal = Expression::FunctionCall(
            Function::Triple,
            vec![
                Expression::Literal(Literal::new_simple_literal("not-a-subject")),
                Expression::NamedNode(NamedNode::new_unchecked("http://ex/p")),
                Expression::NamedNode(NamedNode::new_unchecked("http://ex/o")),
            ],
        );
        assert_eq!(eval_expr(&illegal, &b), None);
    }
}

// --- ADR-0006 M4 wave-2 batch restructure correctness gates ------------------

#[cfg(test)]
mod batch_reconstruct_tests {
    //! `reconstruct_batch` must reproduce the exact per-row sequential
    //! [`reconstruct`] output, in the exact original row order, whether the
    //! batch is small enough to stay sequential, large enough to fan out to
    //! rayon (`TERM_GEN_MIN_PARALLEL_ROWS`), or large but `parallel_allowed` is
    //! `false` (ledger F8's dump-path gate) — the "safest is strict order
    //! preservation via indexed chunks" design note on `reconstruct_batch`.
    use super::*;
    use sf_core::ir::TermSpec;

    /// A branch with ONE bound variable `?v`, read from column `"val"` of scan
    /// alias 0 as a plain literal — real per-row reconstruction work (unlike
    /// `TermDef::Const`, which never touches the row).
    fn branch_with_val_binding() -> Branch {
        let mut b = Branch::empty();
        b.bindings.insert(
            "v".to_owned(),
            TermDef::Derived {
                term_map: TermMap::Column("val".into(), TermSpec::plain_literal()),
                alias: 0,
            },
        );
        b
    }

    /// `n` raw rows, column `"val"` set to the row's index as text — each row
    /// must reconstruct to a distinct term, so a reordering or drop is visible.
    fn raw_rows(n: usize) -> Vec<RawTuple> {
        (0..n)
            .map(|i| RawTuple {
                values: vec![Some(i.to_string())],
                codes: vec![None],
            })
            .collect()
    }

    #[test]
    fn batched_reconstruction_matches_sequential_reference_in_order() {
        let branch = branch_with_val_binding();
        let schema = vec![ColRef::new(0, "val")];
        let col_index = build_col_index(&schema);
        let interned = intern_bindings(&branch);

        // Spans: below TERM_GEN_MIN_PARALLEL_ROWS (the whole-batch sequential
        // path), astride it (the smallest dispatch that goes parallel at all),
        // exactly one full steady-state batch, and several batches' worth — what
        // `run_branches` actually issues as consecutive `reconstruct_batch` calls
        // for one long branch stream (mirrored below via
        // `.chunks(TERM_GEN_BATCH_SIZE)`).
        for n in [
            1,
            50,
            TERM_GEN_MIN_CHUNK_ROWS,
            TERM_GEN_MIN_PARALLEL_ROWS - 1,
            TERM_GEN_MIN_PARALLEL_ROWS,
            TERM_GEN_BATCH_SIZE,
            2 * TERM_GEN_BATCH_SIZE + 137,
        ] {
            let rows = raw_rows(n);
            let sequential: Vec<Option<Term>> = rows
                .iter()
                .map(|t| {
                    let raw = RawRow {
                        values: &t.values,
                        codes: &t.codes,
                        index: &col_index,
                    };
                    reconstruct(&interned, &raw)
                        .expect("reference reconstruct")
                        .get("v")
                        .cloned()
                })
                .collect();

            // Both gate states must match the reference — `parallel_allowed`
            // only decides WHETHER a big batch may fan out, never the result.
            for parallel_allowed in [true, false] {
                let mut batched: Vec<Option<Term>> = Vec::with_capacity(n);
                for chunk in rows.chunks(TERM_GEN_BATCH_SIZE) {
                    for bindings in
                        reconstruct_batch(&interned, chunk, &col_index, parallel_allowed)
                    {
                        batched.push(bindings.expect("batch reconstruct").get("v").cloned());
                    }
                }
                assert_eq!(
                    sequential, batched,
                    "reconstruct_batch must match sequential reconstruct, in order, \
                     at n={n}, parallel_allowed={parallel_allowed}"
                );
            }
        }
    }
}

#[cfg(test)]
mod batch_loop_tests {
    //! `run_branches`' buffer -> parallel-map -> emit-in-order loop (batch-fill,
    //! the `first_batch` ramp, batch-exhaustion detection) end to end through
    //! [`select`], over a stream spanning many batches — a plan-level mock
    //! backend, not a real SQL source, so this is fast and deterministic.
    use super::*;
    use crate::iq::Scan;
    use sf_core::ir::{LogicalSource, TermSpec};

    struct MockBackend {
        rows: Vec<RawTuple>,
    }
    struct MockStream {
        iter: std::vec::IntoIter<RawTuple>,
    }
    impl BranchStream for MockStream {
        async fn next_row(&mut self) -> sf_sql::Result<Option<RawTuple>> {
            Ok(self.iter.next())
        }
    }
    impl SqlBackend for MockBackend {
        type Stream<'s>
            = MockStream
        where
            Self: 's;
        async fn column_names(&mut self, _probe: &str) -> sf_sql::Result<Vec<String>> {
            Ok(Vec::new())
        }
        async fn open_branch(
            &mut self,
            _sql: &str,
            _params: &[String],
        ) -> sf_sql::Result<MockStream> {
            Ok(MockStream {
                iter: std::mem::take(&mut self.rows).into_iter(),
            })
        }
    }

    /// ORDER BY over a stream spanning several `TERM_GEN_BATCH_SIZE` batches
    /// (plus the small first-batch ramp): rows arrive in STRICTLY REVERSED value
    /// order, so a correct result requires the plan-wide sort buffer
    /// (`run_branches`' `buffer`) to have accumulated EVERY row across EVERY
    /// batch-fill iteration — a bug that reset or truncated it per batch would
    /// fail this, where a single-batch-sized fixture could not catch it.
    #[test]
    fn order_by_spans_multiple_batches_correctly() {
        let n = 2 * TERM_GEN_BATCH_SIZE + 137;
        let mut branch = Branch::single(Scan {
            alias: 0,
            source: LogicalSource::Table("t".to_owned()),
        });
        branch.bindings.insert(
            "v".to_owned(),
            TermDef::Derived {
                term_map: TermMap::Column("val".into(), TermSpec::plain_literal()),
                alias: 0,
            },
        );
        let plan = Plan {
            branches: vec![branch],
            form: PlanForm::Select {
                vars: vec!["v".to_owned()],
            },
            distinct: false,
            limit: None,
            offset: 0,
            order: vec![OrderKey {
                var: "v".to_owned(),
                descending: false,
                expr: None,
            }],
            rust_group: None,
            dialect: Dialect::Sqlite,
            dedup_groups: std::collections::HashMap::new(),
            construct_drops_some_branch_var: false,
        };
        // Reversed, zero-padded so lexical order (plain-literal comparison)
        // matches numeric order: row k carries the value belonging at sorted
        // position n-1-k.
        let rows: Vec<RawTuple> = (0..n)
            .map(|k| RawTuple {
                values: vec![Some(format!("{:07}", n - 1 - k))],
                codes: vec![None],
            })
            .collect();
        let mut backend = MockBackend { rows };

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let sol = rt.block_on(select(&plan, &mut backend)).unwrap();

        assert_eq!(
            sol.rows.len(),
            n,
            "no row dropped/duplicated across batches"
        );
        let expected: Vec<String> = (0..n).map(|i| format!("{i:07}")).collect();
        let actual: Vec<String> = sol
            .rows
            .iter()
            .map(|row| match &row[0] {
                Some(Term::Literal(l)) => l.value().to_owned(),
                other => panic!("expected a literal, got {other:?}"),
            })
            .collect();
        assert_eq!(
            actual, expected,
            "ORDER BY must span every batch, not just within one"
        );
    }
}
