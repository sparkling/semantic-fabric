---
status: accepted
date: 2026-06-27
tags: [crate-layout, cargo-workspace, execution, performance, push-down, semi-join, semi-join-cost, term-generation, cost-driven, streaming, bounded-memory, rayon, tokio]
supersedes: []
depends-on:
  - ADR-0002
  - ADR-0003
  - ADR-0004
implements:
  - ADR-0001
---

# Crate/workspace layout and the execution & performance model

## Context and Problem Statement

ADR-0003 fixed the virtualiser architecture; this ADR realises it as a Cargo workspace and fixes the execution & performance model that makes ADR-0001's "SOTA, fast, scalable" charter concrete over relational sources (ADR-0002).

The load-bearing execution decision: semantic-fabric serves an **OLTP-shaped runtime path** — the live virtualizer (ADR-0007) — over **live relational source databases**. Relational data is never routed through a columnar/OLAP intermediary. **The source database does the set-work (scan, join, DISTINCT, sort, spill); the engine generates SQL, generates RDF terms, and streams.** Engine memory is bounded by `⟨T, M⟩` + a fixed streaming budget, **independent of source size**.

## Considered Options

* **Push down to the source DB + bounded semi-join reduction (chosen)** — the rewriter emits SQL run via native drivers; the source does scan/join/DISTINCT/aggregation/sort/spill/parallelism, the engine streams rows and generates terms, with cross-source joins handled by bounded semi-join reduction + streaming k-way merge. Memory bounded by `⟨T, M⟩` independent of source size.
* **Columnar/OLAP intermediary (DataFusion / `connector_arrow` / DuckDB)** — rejected: an in-process columnar engine buffers instance data and breaks the bounded-memory invariant; only the source DB does blocking set-work (it spills natively).
* **Pulled-in in-process join engine for cross-source tables** — rejected in favor of bounded semi-join reduction (fixed-size Bloom filter / bounded `IN`-list / temp-table batch) plus streaming k-way merge, kept inside the fixed memory budget.

## Decision Outcome

### Workspace crates

| Crate | Responsibility | Key deps |
|---|---|---|
| `sf-core` | Mapping IR; term generation; R2RML §10 datatype canonicalization (ADR-0015); `oxrdf` re-exports. No I/O. | `oxrdf`, `oxsdatatypes` |
| `sf-sql` | Source/SQL layer: native connectors, dialect SQL emission, schema introspection, cursor-streamed result iteration, cross-source semi-join planning | `tokio-postgres`, `deadpool-postgres`, `rusqlite`, `sqlparser` |
| `sf-mapping` | R2RML/Direct-Mapping parser (Turtle → IR) | `oxttl`, `sf-core` |
| `sf-sparql` | The virtualizer: SPARQL 1.2 → SQL rewriting + cascade (ADR-0007); streaming result serialization | `spargebra`, `sparopt`, `sparesults`, `oxjsonld`, `sqlparser`, `sf-*` |
| `sf-conformance` | W3C RDB2RDF harness (via CONSTRUCT), EARL, graph-iso, in-memory oracle, `M ⋈ T` hook (ADR-0005) | `oxrdf`, `oxttl`, `shacl`, `sf-*` |
| `sf-bench` | GTFS-Madrid OBDA-track driver | `criterion`, `sf-*` |
| `sf-cli` | Single binary: `serve · conformance · bench` | all `sf-*` |

Dependencies flow **core → virtualizer → cli**; `sf-core`/`sf-sql`/`sf-mapping` never depend on the virtualizer frontend (checkable via `cargo tree`).

### Relational execution — push down to the source, stream back

* **Single-source (the common case): push the work into the source SQL.** The rewriter (ADR-0007) emits one `SELECT … FROM … [JOIN …] [WHERE …] [GROUP BY …] [ORDER BY …]` and runs it via the source's **native driver** (`tokio-postgres` + `deadpool`; `rusqlite`). The source does scan + join + DISTINCT + aggregation + sort + spill + parallelism; the engine streams rows, generates terms (`sf-core`), and serialises. This dissolves the multi-join / N:M cliff (the source has indexes and a real optimizer).
* **Cross-source (rare; tables in *different* relational databases): bounded semi-join reduction, never a pulled-in join engine.** Ship the smaller side's join keys to the larger source as a **fixed-size Bloom filter or a bounded `IN`-list / temp-table batch** (never the full key set), then combine the reduced inputs in-process via a **streaming k-way merge**. The reduction is **cost-driven** — side selection, reducer form/sizing, and a skip-if-unselective gate (see *Cross-source semi-join cost* below) — kept **bounded-memory** (ADR-0010); this is the Teiid dependent-join / Trino dynamic-filtering family.
* **No columnar/OLAP engine on the relational path.** DataFusion, `connector_arrow`, and DuckDB are **not** used to mediate between the rewriter and relational sources: a columnar engine in-process would buffer instance data and break the bounded-memory invariant; only the source DB does blocking set-work (it spills natively). Relational execution = native drivers + push-down + bounded semi-join reduction.

### Cross-source semi-join cost

The cross-source semi-join is the engine's one genuinely in-process join decision, so its planner is **cost-driven from the start** — a foundational, baked-in decision, because retrofitting cost once the planner has callers is expensive:

* **Side selection** — ship the *smaller* side's keys to the larger source, where "smaller" is chosen by **distinct-key cardinality** from source catalogs (`pg_class.reltuples`, `information_schema`, `sqlite_stat1`), not by raw row count.
* **Reducer form & sizing** — choose `IN`-list vs temp-table vs Bloom filter by the estimated distinct-key count (small → `IN`-list; larger → temp-table / Bloom), so the reducer itself stays inside the fixed memory budget.
* **Skip-if-unselective gate** — if the estimated reduction ratio is ≈ 1 (the reducer would eliminate almost nothing), skip the semi-join and stream-merge the inputs directly; the reducer round-trip only earns its cost when it is selective.
* **Estimation inputs** — catalog stats, plus HLL/Bloom distinct-count sketches where catalogs are thin, plus at most one cached `EXPLAIN (FORMAT JSON)` row-count probe of a *leaf* sub-pattern (reusing the source's own estimator) — never a probe that compares two whole equivalent translations. This needs only the catalog read this ADR already mandates.

### Streaming & bounded memory (the invariant)

Results stream end to end: a **server-side cursor** (`tokio-postgres` `query_raw()` → `RowStream`; never the buffer-all `query()`), per-row term generation, and a streamed serializer — **SPARQL 1.2 Results** (SELECT/ASK) or **JSON-LD** (CONSTRUCT/DESCRIBE; expanded/incremental, never framed — ADR-0019). No operator buffers instance data unbounded; blocking operators (sort/group/distinct) are pushed to the source. Governance (timeouts, caps, backpressure, cancel-on-drop) is ADR-0010.

### Parallelism & dialects

* `tokio` owns all async I/O (drivers, result streaming, the OBDA endpoint); `rayon` parallelises any CPU-bound term generation. **The pools stay separate** (mixing causes latency spikes); CPU work invoked from async goes through `spawn_blocking`.
  > **Measured correction (2026-07-18, M4 wave-2).** The rayon term-gen pool was
  > built but never wired to a caller, and measurement settled it: per-row
  > dispatch of a ~10ns unit of work is **~2× slower** than inline (~1.9–2.4ms
  > vs ~0.93–1.0ms per 100k rows) — scheduling overhead dominates — so
  > `pool.rs` and the `rayon` dependency are **removed**; term generation runs
  > inline on the calling thread/task everywhere. A **chunked** dispatch
  > (1000 rows/task) measured a genuine **~6× win** (~155µs), but it requires
  > restructuring `exec_core`'s per-row solution loop into a batch shape —
  > real, unscheduled follow-up work, recorded here rather than half-shipped.
  >
  > **Chunked dispatch implemented, correction to the correction (2026-07-19,
  > M4 wave-2 continued).** `exec_core`'s `run_branches` loop is restructured
  > into buffer → parallel-map → emit-in-order: a bounded batch of raw rows is
  > pulled off the cursor (a small first batch, then a fixed steady-state
  > size, so first-result latency stays bounded — the streaming invariant
  > above), `rayon::par_chunks` reconstructs it (chunks sized off
  > `current_num_threads()`, never one task per row — the shape that measured
  > slower), and the batch is emitted downstream in original order (`par_chunks`
  > is index-preserving, so this needs no extra bookkeeping). `rayon` returns to
  > `sf-sparql/Cargo.toml`, using its own lazily-initialized global pool
  > directly rather than a hand-rolled `ThreadPool` — still structurally
  > separate from `tokio` (a wholly different set of OS threads), satisfying
  > the pool-separation rule above without reintroducing `pool.rs`.
  >
  > The **~6× / 1000-rows-per-task** figure above does not hold at this
  > restructure's actual granularity and was superseded by re-measurement, not
  > assumed to transfer: that number came from a ONE-SHOT `par_chunks` call
  > over a whole synthetic dataset at once, but a streaming cursor cannot be
  > buffered whole (would break the invariant this section opens with), so
  > `run_branches` issues one FRESH `par_chunks` call PER BATCH. Re-measured at
  > that granularity (`sf-bench`'s `micro_term_gen_batch`, ~100k synthetic
  > `rr:template` rows), 1000-row batches (100 dispatch calls) came out **~1.8×
  > SLOWER** than plain inline — the fixed per-call cost (thread wake/join)
  > dominates a batch that small the same way it dominated a single row. A
  > sweep found the throughput break-even between 2000–5000 rows, and
  > 10 000 measured a genuine, comfortable **~1.6–1.7× faster**.
  >
  > A **second, independent constraint** then capped the batch size far below
  > that throughput optimum: `sf-bench`'s own `constant_memory` peak-heap
  > invariant test (which this restructure must keep passing, not just the
  > throughput bench) measured `mem_ratio` — its bounded-memory tolerance,
  > `4.0` — blown well past at both candidate sizes (9.05 at 10 000 rows, 5.44
  > at 5000), because a buffered, reconstructed row costs far more than its
  > term data: `BTreeMap<String, Term>`'s per-node allocator overhead
  > dominates for the small (1–3-entry) per-row binding maps a typical branch
  > produces, multiplied by up to `TERM_GEN_BATCH_SIZE` of them alive at once.
  > Memory *does* stay strictly O(batch), never O(result) — a dedicated
  > single-branch test (`engine_memory_is_batch_bounded_past_the_batch_size_threshold`)
  > proves the peak is byte-near-identical at 20k rows and at 80k rows once
  > both exceed the batch size — but the size of that fixed O(batch) budget is
  > itself large enough, at throughput-optimal batch sizes, to fail the
  > existing GTFS-workload test's tolerance at ITS 1×/4×/16× scale factors
  > (whose branches don't uniformly cross the batch-size threshold together).
  > **`TERM_GEN_BATCH_SIZE = 3000`** is therefore the memory-constrained
  > final value (mem_ratio ≈ 3.4–3.5, a real margin under the `4.0` gate), not
  > the throughput-optimal one — it measures a modest but genuine **~1.10×
  > faster** than inline, not the ~1.6–1.7× a bigger batch would give. Raising
  > this ceiling needs a leaner per-row binding representation than
  > `BTreeMap` (out of scope here; a real follow-up wave, not a footnote to
  > half-ship) — this section's structural claims (pool separation, chunked
  > dispatch, order preservation, streaming-bounded first batch) all hold at
  > any batch size; only the specific constant is memory-bound today.
  >
  > **Dump-path regression + call-site gate (2026-07-19, ledger F8).** The
  > chunked dispatch above measurably REGRESSED the streamed CONSTRUCT dump
  > (`constant_memory_dump`: +31–35% at 10×/100× scale) while still winning on
  > `micro_distinct_agg`/`micro_group_avg_rust`. Toggle-isolated on a quiet
  > machine: forcing every batch sequential while leaving the buffer-then-
  > reconstruct shape exactly as-is reproduced the pre-batch, zero-buffer
  > baseline to within ~2% — the buffering indirection itself costs nothing
  > measurable; the regression is 100% the `par_chunks` dispatch. The
  > differentiator is per-row cost, not row count: the dump's rows are plain
  > column/template copies (cheap `Literal::new_simple_literal`, no numeric
  > formatting), so dispatch's fixed thread wake/join cost exceeds the compute
  > saved even at 80k-row batches; `rust_group`'s aggregate inner collection
  > (`AVG`/`SUM(DISTINCT)`/`COUNT(DISTINCT)` over `canonical_lexical`-formatted
  > numeric literals — always fully materialized before grouping can start
  > regardless) is the shape the constant was tuned against and still wins
  > there, by a more modest ~5–8% toggle-isolated (not the full ~29%/~4% the
  > original F6 landing measured against a stale pre-F6 baseline under
  > different machine load). Fix: `reconstruct_batch` takes a `parallel_allowed`
  > flag threaded through `PlanCtx`, `true` only for `rust_group_execute`'s
  > inner collection — the plain streaming SELECT/CONSTRUCT/ASK path
  > (`for_each_solution`'s direct `run_branches` call) always reconstructs
  > sequentially now. `TERM_GEN_BATCH_SIZE`/`TERM_GEN_MIN_PARALLEL_ROWS` and the
  > memory-bound reasoning above are unchanged — only WHO may cross the
  > parallel gate changed, not the batch shape or its constants.
* First-class source dialects: **PostgreSQL** (primary production), **SQLite** (embedded / W3C-suite CI); **MySQL** follows. DuckDB may appear only as a *SQL source you push down to* like any other relational source — never a columnar intermediary, never a file reader; heterogeneous/file sources are out of scope (ADR-0002).
* Crate pins + 1.2 feature flags: ADR-0004 / ADR-0019. Toolchain pinned via `rust-toolchain.toml`.

### Term generation — allocation discipline

Term generation runs once per result row, and its dominant cost is **small-object allocation, not byte-level work** — so the discipline is fixed now, before the term API has callers (costly to retrofit afterwards):

* **Constants built once.** Predicate, `rdf:type`, and datatype IRIs, plus the literal segments of every `rr:template`, are interned at mapping-load time and emitted by reference (`oxrdf::NamedNodeRef`, zero-copy). Template-constructed IRIs use `NamedNode::new_unchecked` — the R2RML template already fixes the form, so per-row RFC-3987 re-validation is waste.
* **Write-through, not allocate-through.** Terms are written into a reusable buffer via a `generate_into(&mut String)` / visitor API rather than returning an owned `Term`/`String` per call (predicated on `sparesults` accepting borrowed terms; if it forces an owned term on the SELECT path, that one alloc stays and CONSTRUCT still wins). `rr:template` is precompiled to a segment list, so there is no per-row placeholder scan.
* **Bounded by `⟨T, M⟩`, never by data.** A symbol table (`lasso`) interns *mapping-IR* symbols at parse time only; it is **never** used for per-row data values (append-only → unbounded → breaks the bounded-memory invariant). At most a small fixed-size LRU for a column proven low-cardinality.
* **Datatype formatting stays on `oxsdatatypes`** (hand-written XSD-canonical), **not** `ryu`/shortest-round-trip — which is not XSD-canonical and would be a conformance bug (ADR-0015). *(Reconciliation, 2026-06-28, impl-verified: the rule fixes the **output** as XSD-canonical and bans the non-canonical `ryu` crate — it is not a ban on `std` formatting as such. `oxsdatatypes` `Display` is itself canonical for every type the engine emits **except `xsd:double` / `xsd:float`**, whose `Display` delegates to `f64`/`f32` and is non-canonical; for those two the chokepoint validates through `oxsdatatypes` and then emits canonical `E`-notation via `std` exponential formatting — canonical output, not `ryu`. See the ADR-0015 reconciliation note.)*
* **SIMD is profile-gated, not baked in.** `portable_simd` is nightly and we pin stable, so any SIMD (`simdutf8` over raw column bytes, nibble-table percent-encoding) is added only if profiling shows term-gen bound there — typical OBDA keys are short, clean PKs. A fast global allocator (mimalloc/jemalloc) is a measure-first drop-in, not a correctness dependency.

### Consequences

* Good, because memory-bounded by construction (the source spills); minimal data movement; a light dependency set with no columnar engine and no triplestore on the data plane; coherent with the OLTP runtime path and the single-binary ethos.
* Good, because crate boundaries enforce the architecture; perf decisions grounded in measured prior art.
* Bad, because cross-source joins need an in-engine bounded semi-join / k-way-merge planner; cross-source cardinality estimation remains the hardest input (catalogs can be stale or thin), now mitigated by the cost model above — sketch-based distinct counts, a cached leaf `EXPLAIN` probe, and the skip-if-unselective gate — rather than left open.
* Bad, because the rayon/tokio pool separation is a standing latency/correctness discipline.

### Confirmation

* `cargo build --workspace` succeeds; `cargo tree` shows native drivers and **no `datafusion` / `connector_arrow` / `duckdb` / `librocksdb-sys`** on the relational crates.
* `sf-cli --help` lists `serve · conformance · bench` (no `materialize`).
* GTFS-Madrid OBDA-track scenarios complete with **constant engine memory** under growing scale factor, measured via `sf-bench` (ADR-0005).
* Term generation emits constants by reference and writes via `generate_into` — an allocation-count test over a fixed result size shows no per-row owned `Term` on the CONSTRUCT path.
* The cross-source semi-join planner selects side, reducer form, and skip-vs-reduce from catalog/sketch estimates — unit-tested against synthetic cardinalities (small, large, and ≈ 1-reduction).

## More Information
* **Architecture:** ADR-0003. **Substrate:** ADR-0004. **Rewriting + cascade:** ADR-0007. **Datatype/dialect:** ADR-0015. **Reasoning:** ADR-0008. **Conformance/bench/oracle:** ADR-0005. **Governance + streaming:** ADR-0010. **Test strategy:** ADR-0012.
* **Research:** `docs/research/` — `external-memory-join`, `federation`, `rust-substrate`.
* **Cost-driven design (baked in here):** the term-gen allocation discipline and the cross-source semi-join cost model; the rewriter-side term-construction lifting + plan cache are in ADR-0007. Both promoted from the ADR-0020 research register.
