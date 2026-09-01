//! Bounded raw-row batching and ordered parallel term reconstruction.

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
pub(super) const TERM_GEN_BATCH_SIZE: usize = 4_000;

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
pub(super) const TERM_GEN_FIRST_BATCH_SIZE: usize = 64;

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
/// SELECT) deserved cleaner verification before its default flipped.
///
/// **FINAL VERDICT (2026-07-20, Run 5 W1) — un-gated.** The idle-machine
/// re-run happened under a noise-floor-first protocol (load 3.92 on 18
/// cores, zero competing cargo/rustc/criterion processes; two prior
/// attempts correctly ABORTED under swarm load rather than repeat the
/// contamination). Measured floor from back-to-back unchanged runs:
/// ~0.47% at 1x / ~2.42% at 10x — the first post-build pair was discarded
/// for a one-time cold-start swing (`first_result_µs` 1616 → 190), the
/// same first-result-vs-steady-state distinction drawn elsewhere in this
/// file. Against a warm baseline (medians): `full_dump_10x` **−9.15% /
/// −9.95%** across two flip replicates (both p < 0.05, agreeing within
/// 0.8pp), clearing the decision bar of max(5%, 2×floor) = 5%;
/// `full_dump_1x` stayed within noise in both replicates (+0.60% n.s. /
/// −1.73% criterion-noise) — 1x can still cross this gate (5200 triples ⇒
/// one 4000-row batch), so in-noise was the requirement, not untouched.
/// Both constant-memory invariant tests pass in the flipped
/// configuration. `for_each_solution` therefore now sets
/// `parallel_term_gen: true` for the plain streaming path; this constant
/// and `TERM_GEN_BATCH_SIZE` are unchanged — only WHO may cross the gate
/// changed, completing the F6→F8→C1 arc (the C1-era loaded-box "7-9%
/// faster" signal was real).
pub(super) const TERM_GEN_MIN_PARALLEL_ROWS: usize = 2_000;

/// The floor on `par_chunks`' chunk size WITHIN one already-dispatched batch
/// (see [`TERM_GEN_MIN_PARALLEL_ROWS`] for the separate whole-batch gate). Once a
/// batch is worth dispatching at all, a single `par_chunks` call's per-call
/// overhead is already paid — so this floor only needs to keep individual
/// chunks well above the measured-slower per-row granularity, not repeat
/// [`TERM_GEN_MIN_PARALLEL_ROWS`]'s much larger bar.
pub(super) const TERM_GEN_MIN_CHUNK_ROWS: usize = 128;

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
pub(super) fn reconstruct_batch(
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
use sf_sql::RawTuple;

use crate::Result;

use super::row::{reconstruct, Bindings, ColIndex, InternedBindings, RawRow};
