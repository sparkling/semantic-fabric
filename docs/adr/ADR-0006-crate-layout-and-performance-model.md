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
