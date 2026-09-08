---
status: accepted
date: 2026-06-27
updated: 2026-09-08
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

The load-bearing execution decision: semantic-fabric serves an **OLTP-shaped runtime path** — the live virtualizer (ADR-0007) — over **live relational source databases**. Relational data is never routed through a columnar/OLAP intermediary. **For admitted, fully pushed-down single-source shapes, the source database does the set-work (scan, join, DISTINCT, sort, spill); the engine generates SQL, generates RDF terms, and streams.** Engine memory is bounded by `⟨T, M⟩` plus a fixed streaming budget, **independent of source size**. Unsupported composite shapes reject before I/O unless a separately accepted bounded physical operator proves the same invariant.

## Considered Options

* **Push down to the source DB + bounded semi-join reduction (chosen baseline)** — the rewriter emits SQL run via native drivers; the source does scan/join/DISTINCT/aggregation/sort/spill/parallelism and the engine streams rows and generates terms. Bounded semi-join reduction plus streaming merge admits only shapes for which exactness and source-independent memory are proved; other composite shapes reject. Accepting ADR-0040 would explicitly amend this cross-source clause with quota-bounded external operators.
* **Columnar/OLAP intermediary (DataFusion / `connector_arrow` / DuckDB)** — rejected: an in-process columnar engine buffers instance data and breaks the bounded-memory invariant; only the source DB does blocking set-work (it spills natively).
* **Pulled-in in-process join engine for cross-source tables** — rejected in favor of bounded semi-join reduction (fixed-size Bloom filter / bounded `IN`-list / temp-table batch) plus streaming k-way merge, kept inside the fixed memory budget.

## Decision Outcome

### Workspace crates

| Crate | Responsibility | Key deps |
|---|---|---|
| `sf-core` | Mapping/source-affinity IR; neutral relational schema DTOs; term generation; R2RML §10 datatype canonicalization (ADR-0015); `oxrdf` re-exports. No I/O. | `oxrdf`, `oxsdatatypes` |
| `sf-sql` | Source/SQL layer: native connectors, dialect SQL emission, schema introspection, cursor-streamed result iteration, cross-source semi-join planning | `tokio-postgres`, `deadpool-postgres`, `rusqlite`, `mysql_async`, `sqlparser` |
| `sf-mapping` | R2RML/Direct-Mapping parser (Turtle → IR), including Direct Mapping from neutral core schema DTOs | `oxttl`, `sf-core` |
| `sf-sparql` | The virtualizer: SPARQL 1.2 → SQL rewriting + cascade (ADR-0007); streaming result serialization | `spargebra`, `sparesults`, `oxjsonld`, `sqlparser`, `quick_cache`, `sf-*` |
| `sf-serve` | HTTP/SPARQL Protocol boundary, source selection, backend pools and streamed responses | `axum`, `tokio`, native database drivers, `sf-*` |
| `sf-validation` | Product-owned bounded sealed `M ⋈ T` gate; three Core shapes via rudof Native, one parsed sealed datatype query executed globally, policy-v2 receipt identity, and count-only redacted outcome | `oxrdf`, `oxttl`, `shacl`, `rudof_rdf`, `sparql_service` |
| `sf-conformance` | W3C RDB2RDF harness (via CONSTRUCT), EARL, graph-iso, in-memory oracle, and consumer of the product `M ⋈ T` gate (ADR-0005) | `oxrdf`, `oxttl`, `sf-validation`, `sf-*` |
| `sf-bench` | GTFS-Madrid OBDA-track driver | `criterion`, `sf-*` |
| `sf-cli` | Single binary: `serve · conformance · bench` | all `sf-*` |

The product flow is **core → {mapping, SQL/source} → virtualizer → validation/serve → cli**; conformance and bench are sibling development consumers. The neutral schema DTO now lives in `sf-core`, `sf-sql` re-exports it for compatibility, and `sf-mapping` no longer depends on `sf-sql`. `sf-core`/`sf-sql`/`sf-mapping` never depend on the virtualizer or serving frontends (checkable via `cargo tree`).

### Relational execution — push down to the source, stream back

* **Single-source (the common case): push the work into the source SQL.** For admitted, fully pushed-down shapes, the rewriter (ADR-0007) emits one `SELECT … FROM … [JOIN …] [WHERE …] [GROUP BY …] [ORDER BY …]` and runs it via the source's **native driver** (`rusqlite`, `tokio-postgres` + `deadpool`, or `mysql_async`). The source does scan + join + DISTINCT + aggregation + sort + spill + parallelism; the engine streams rows, generates terms (`sf-core`), and serialises. This dissolves the multi-join / N:M cliff (the source has indexes and a real optimizer).
* **Current binding boundary.** One immutable `CompilerBinding` owns its `SourceMapping`, dialect, T-box, compiler-safe schema and private plan cache; `sf-serve` pairs it with one backend and verifies the bound plan before backend selection or I/O. Commit `24a0e20` makes the serving constructor convert the raw observation to `CompilerSchema` with `ConstraintAuthority::Unverified`: it retains table/column names, SQL types and estimates, but removes PK, UNIQUE, FK, functional-dependency and NOT-NULL claims. The authority is part of `CompileScope`, so cache hits and misses share the same policy. A `SourceRegistry` validates every source with the sealed split `M ⋈ T` evaluator before assembling those bindings into one deterministic, digest-addressed immutable `RuntimeSnapshot`; admission/cache identity includes validation policy v2 and its count-only warning policy. `RuntimeManager` atomically replaces complete generations, exposes readiness, and pins the selected generation through response-body lifetime. Constructing a new runtime binding still creates a fresh cache namespace and there is no in-place schema mutation. `ServeConfig` and the CLI support single-source plus two sealed source-affine profiles: exactly two one-triple `SELECT UNION` arms, or the bounded two-pattern join below. Public opt-in authored reload validates and atomically publishes complete generations and fences observed drift; backend DDL protection, policy/configuration hot reload, live Direct Mapping promotion and backend admission remain open.
* **PostgreSQL relation scope.** Introspection and unqualified `rr:tableName` resolution are explicitly `public`-only. Startup observes one read-only repeatable-read catalogue snapshot; pool creation/recycle pins `search_path=pg_catalog,public,pg_temp`, every request verifies it, and cross-schema foreign keys or `public` relation names shadowed by the earlier `pg_catalog` scope fail closed because the neutral DTO cannot represent qualified identity. Trusted raw `rr:sqlQuery` text is emitted verbatim and can name a qualified relation; neither `search_path` nor this guard confines it. This is coherent startup identity for catalogued base tables, not later-DDL detection, an SQL-query sandbox, or arbitrary-schema support. Captured integrity facts remain useful as observation evidence, but never enter the current serving compiler as proof.
* **Direct Mapping lifecycle.** Current `sf-serve` consumes authored R2RML and does not generate Direct Mapping. The Direct Mapping utility and conformance runners may generate mappings from an explicit frozen fixture schema and may pass those same facts to raw translation APIs. That is test/development authority, not serving authority. A future live Direct-Mapping path must bind its PK/FK-dependent generated mapping to a verified source generation through the complete streamed execution; quarantining optimiser facts after generating the mapping would not protect its semantics.
* **Cross-source (rare; tables in *different* relational databases).** Non-blocking `UnionAll` is now implemented for the sealed exactly-two-source shape above: both source-local plans retain their immutable bindings, both source leases are acquired before success is committed, one request budget and serializer span both fragments, and the bags stream sequentially without global buffering. The separately implemented two-pattern join below follows the original baseline: an admitted reducible join ships a bounded representation of one side's keys as a fixed-size Bloom filter or bounded `IN`-list/temp-table batch and use a proven bounded merge. That reducer is not a general N:M join answer. Shapes needing an unimplemented blocking global operator reject before I/O; accepting proposed ADR-0040 would replace only this semi-join/merge-only clause with its quota-bounded external layer. The streaming `UnionAll` slice neither depends on nor accepts ADR-0040.
* **No columnar/OLAP engine on the relational path.** DataFusion, `connector_arrow`, and DuckDB are **not** used to mediate between the rewriter and relational sources: a columnar engine in-process would buffer instance data and break the bounded-memory invariant; only the source DB does blocking set-work (it spills natively). Relational execution = native drivers + push-down + bounded semi-join reduction.

**UNION lineage update (2026-09-08):** the opt-in bounded actual-origin profile in
ADR-0017 composes two capped source-local witness producers with the same public
UNION owners, snapshot, security and cumulative budget. It adds no global join,
spill, persistent data or new architecture. Ordinary and lineage UNION now scope
blank nodes, including nested triple terms, by their actual SourceId before shared
serialization; scratch growth is charged first. TBox-aware source affinity in both formats
rejects ambiguity before I/O. Required HTTP and pinned native CLI checks cover the
precise profile; federated lineage joins and broader release admission remain open.

### Cross-source semi-join cost

The cross-source semi-join is the engine's one genuinely in-process join decision, so its planner is **cost-driven from the start** — a foundational, baked-in decision, because retrofitting cost once the planner has callers is expensive:

* **Side selection** — ship the *smaller* side's keys to the larger source, where "smaller" is chosen by **distinct-key cardinality** from source catalogs (`pg_class.reltuples`, `information_schema`, `sqlite_stat1`), not by raw row count.
* **Reducer form & sizing** — choose `IN`-list vs temp-table vs Bloom filter by the estimated distinct-key count (small → `IN`-list; larger → temp-table / Bloom), so the reducer itself stays inside the fixed memory budget.
* **Skip-if-unselective gate** — if the estimated reduction ratio is ≈ 1 (the reducer would eliminate almost nothing), skip the semi-join and stream-merge the inputs directly; the reducer round-trip only earns its cost when it is selective.
* **Estimation inputs** — catalog stats, plus HLL/Bloom distinct-count sketches where catalogs are thin, plus at most one cached `EXPLAIN (FORMAT JSON)` row-count probe of a *leaf* sub-pattern (reusing the source's own estimator) — never a probe that compares two whole equivalent translations. This needs only the catalog read this ADR already mandates.

### Implemented bounded join profile (2026-09-07)

The public two-source mode now accepts `SELECT ?left ?right WHERE { ?left <urn:left> ?key . ?right <urn:right> ?key }` when each predicate belongs to exactly one distinct source. Each arm requires distinct subject/object variables, one constant matching predicate emitter, a direct object map, one authored default-graph base-table mapping and one scan. At least one shared key must be a reversible single-slot IRI template or explicitly typed/language-tagged literal column. Constants/repeated variables within an arm, dynamic predicates, authored SQL views, global modifiers and wider algebra reject before source I/O. This is a versioned bounded profile, not full join-algebra support.

The coordinator retains at most **128 distinct complete driving triples** and **4,096 distinct complete probe triples**, with separate retained-payload, source-work, result, serialized-byte and deadline controls. It replaces only a strictly validated compiler-generated raw-column single-table projection with its known authored table: database-collation `DISTINCT` cannot establish RDF graph identity. Complete source triples are normalized exactly before the merge, including language case and source-scoped blank nodes; projected solutions and N×M result multiplicity are never deduplicated. Caps never reset or evict during a query. This fixed-cap operator is the accepted bounded-merge exception, not source-sized DISTINCT, general global operators, spill, or acceptance of proposed ADR-0040.

A parameter-bound conservative reducer is added to the already authorized probe plan; native SQL equality is only a superset filter, followed by exact RDF term comparison, including datatype, language and direction. Percent-encoded keys are inverted and re-expanded. Current catalog row estimates, when both exist, are **cost proxies only**, not distinct-count observations or semantic authority: `plan_semijoin` chooses the driving side and skip/reduce advice. The physical implementation always uses one capped batch, never silently allocates the planner's suggested Bloom/temp-table strategy. Missing estimates use deterministic input order; skip advice retains the same exact bounded merge. Catalog NDV/sketch/EXPLAIN enhancements remain unimplemented, not correctness prerequisites.

Both source leases are acquired before execution. One pinned application generation/security context/budget covers both statements; each database supplies its own source-local statement view, **not a distributed data snapshot or a backend DDL lease**. The complete serialized response is capacity-checked before HTTP 200, so source failure, overflow and observed deadline/cancellation cannot expose a success prefix. Fragments now share ADR-0010's owned native PostgreSQL/MySQL stop/discard guards; required pinned PostgreSQL 16.15/MySQL 8.4.11 TLS CLI tests now cover UNION and both join pattern orders under deadline, disconnect and forced SIGTERM, observe target native stop with server-side granted-lock witnesses, preserve a distinct same-credential CLI sibling, and recover the full exact bag through both cap-one pools. Join timeout remains pre-200; failed streaming UNION has no clean chunked completion. Wider backend/profile combinations, protected source generations and exact-release admission still require ADR-0055 qualification. No total heap bound or database scan-cost proof is inferred from coordinator payload accounting. Required public tests include an independent materialized graph oracle, collation/padding/NULL/bag boundaries, authorization, 128/129 and 4096/4097 limits, failure recovery, real SQLite CLI reload, and encrypted mixed PostgreSQL/MySQL CLI joins.

### Streaming & bounded memory (the invariant)

Except for the explicitly capped pre-200 join response above, admitted results stream end to end: a **server-side cursor** (`tokio-postgres` `query_raw()` → `RowStream`; never the buffer-all `query()`), per-row term generation, and a streamed serializer — **SPARQL 1.2 Results** (SELECT/ASK) or **JSON-LD** (CONSTRUCT/DESCRIBE; expanded/incremental, never framed — ADR-0019). No admitted operator buffers instance data unbounded; blocking operators are pushed to the source or require a separately accepted bounded implementation. Governance (timeouts, caps, backpressure, cancel-on-drop) is ADR-0010.

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
  >
  > **The leaner representation landed (2026-07-20, Run 4 C1).** The
  > "follow-up wave" the batch-size paragraph ledgers is done: the per-row
  > `BTreeMap<String, Term>` is now `Bindings(Vec<(Arc<str>, Term)>)` —
  > linear-scan lookups (branches bind 1–3 vars), var names interned once per
  > branch and `Arc`-cloned per row (the `String`-key clone is gone). Two
  > whole-row consumers relied on `BTreeMap`'s alphabetical iteration and now
  > canonicalize explicitly (`canonical_pairs`, sort-by-name): the ADR-0034
  > term-dedup key and `COUNT(DISTINCT *)`. Re-measured: the memory ceiling
  > moved — **`TERM_GEN_BATCH_SIZE = 4000`** (mem_ratio 3.69 under the 4.0
  > gate; 4500 fails at 4.01; 3000 under the new representation is 3.0 vs the
  > old ~3.4) — and the representation alone is worth **−15–19% on the dump**
  > and **−28–33% on the `rust_group` paths** (criterion medians, shared box,
  > noise floor −2.6%). The F8 call-site gate is KEPT: forcing streaming-path
  > dispatch on still regressed the allocator-instrumented dump bench
  > (+17–19%), but that signal is an artifact of the tracking allocator's
  > atomics contending under `par_chunks` — the UNINSTRUMENTED dump bench
  > showed dispatch now WINNING ~7–9% at 10× (p<0.05, two replicates) on a
  > loaded machine. Not shipped from that measurement session; the follow-up
  > (re-run `obda_construct_dump` on an idle machine, flip the gate if it
  > holds) was recorded in `TERM_GEN_MIN_PARALLEL_ROWS`'s doc comment.
  >
  > **Un-gated (2026-07-20, Run 5 W1) — the arc closes.** The idle-machine
  > re-run (noise-floor-first protocol; floor 0.47%/2.42%; two prior
  > correctly-aborted attempts under swarm load) confirmed the C1 signal:
  > `full_dump_10x` **−9.15%/−9.95%** across two replicates (p<0.05,
  > clearing max(5%, 2×floor)), `full_dump_1x` within noise, constant-memory
  > invariants green flipped. The plain streaming path now dispatches
  > (`parallel_term_gen: true`); constants unchanged. Full verdict in the
  > gate's own doc comment.
* First-class source dialects: **PostgreSQL**, **SQLite**, and **MySQL** have reachable development serving paths; zero are production-admitted under ADR-0038 R3. PostgreSQL is the primary target and SQLite is the embedded/W3C-suite CI source. DuckDB may appear only as a *SQL source you push down to* like any other relational source — never a columnar intermediary, never a file reader; heterogeneous/file sources are out of scope (ADR-0002).
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
* Bad, because cross-source cardinality estimates can be stale or absent. The implemented capped join uses optional row-count cost proxies and skip advice; distinct-count sketches and a cached leaf `EXPLAIN` probe remain design follow-ups, not implemented mitigation. Irreducible shapes reject; the general global-operator layer in ADR-0040 remains proposed.
* Bad, because the rayon/tokio pool separation is a standing latency/correctness discipline.

### Confirmation

* `cargo build --workspace` succeeds; `cargo tree` shows native drivers and **no `datafusion` / `connector_arrow` / `duckdb` / `librocksdb-sys`** on the relational crates.
* `sf-cli --help` lists `serve · conformance · bench` (no `materialize`).
* Required compiler, serving and real-process HTTP tests prove the sealed two-source `UnionAll` shape over two file-backed SQLite sources, including bag duplicates, UNBOUND domains, pre-I/O rejection, shared `0/N/N+1` budgets, second-source acquisition failure/recovery and request-lifetime snapshot pinning.
* GTFS-Madrid OBDA-track scenarios complete with **constant engine memory** under growing scale factor, measured via `sf-bench` (ADR-0005).
* Term generation emits constants by reference and writes via `generate_into` — an allocation-count test over a fixed result size shows no per-row owned `Term` on the CONSTRUCT path.
* For admitted reducible shapes, the cross-source semi-join planner selects side, reducer form, and skip-vs-reduce from catalog/sketch estimates — unit-tested against synthetic cardinalities (small, large, and ≈ 1-reduction).

> **Historical reconciliation (2026-09-05; join status superseded above on 2026-09-07).** At that date the planner was unit-tested design with no production caller and only bounded `UnionAll` was served. The universal bounded-memory
> wording is also ahead of the implementation for Rust grouping, multi-branch
> solution dedup, and some CONSTRUCT dedup paths, which retain source-sized
> collections in `exec_core.rs`. Finite root `ORDER BY … OFFSET … LIMIT` windows
> now use a bounded stable heap and have semantic-oracle plus fresh-process RSS
> qualification; wider ORDER shapes still reject. Accepted ADR-0055 preserves this
> architecture but requires composite SQL or a bounded coordinator operator—and
> an explicit unsupported result until each remaining shape has that proof.

## More Information
* **Architecture:** ADR-0003. **Substrate:** ADR-0004. **Rewriting + cascade:** ADR-0007. **Datatype/dialect:** ADR-0015. **Reasoning:** ADR-0008. **Conformance/bench/oracle:** ADR-0005. **Governance + streaming:** ADR-0010. **Test strategy:** ADR-0012.
* **Research:** `docs/research/` — `external-memory-join`, `federation`, `rust-substrate`.
* **Cost-driven design (baked in here):** the term-gen allocation discipline and the cross-source semi-join cost model; the rewriter-side term-construction lifting + plan cache are in ADR-0007. Both promoted from the ADR-0020 research register.
