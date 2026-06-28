# semantic-fabric

**A uniform query layer over every system of record — at OLTP speed. The data foundation you build your agents on.**

[![CI](https://github.com/sparkling/semantic-fabric/actions/workflows/ci.yml/badge.svg)](https://github.com/sparkling/semantic-fabric/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#13-contributing--license)
[![W3C RDB2RDF](https://img.shields.io/badge/W3C%20RDB2RDF-81%2F82%20SQLite%20%C2%B7%2080%2F81%20PostgreSQL-success.svg)](#9-status--limitations)
[![Rust 1.96](https://img.shields.io/badge/rust-1.96.0-orange.svg)](rust-toolchain.toml)

semantic-fabric is a **Rust-native, virtualisation-only OBDA engine**: it answers
**SPARQL 1.2** by rewriting each query to **SQL** that runs directly against your
live relational database through **R2RML** mappings ([ADR-0001](docs/adr/ADR-0001-semantic-fabric-rust-data-fabric.md)).
There is **no JVM** and **no copy of the data** — instance data (the A-Box) is
generated on demand at query time, streamed, and discarded. Only the ontology
hierarchy `T` and the mappings `M` ever live in the engine.

> _Scope chip:_ Rust-native, virtualisation-only OBDA — **SPARQL 1.2 → SQL over
> R2RML**, executed live against **SQLite & PostgreSQL** ([ADR-0002](docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md)).

```sparql
# "Which routes belong to which agency?" — answered live over a database,
# never materialised. (SPARQL in crates/sf-bench/src/workload.rs::queries)
PREFIX gtfs: <http://vocab.gtfs.org/terms#>
SELECT ?route ?agency WHERE { ?route a gtfs:Route ; gtfs:agency ?agency . }
```

```bash
# Today: the in-process OBDA driver rewrites these to SQL and streams results
# from a live SQLite source (the `serve` HTTP endpoint is on the roadmap — §9).
cargo run -p sf-cli -- bench
```

---

## 1. The problem

Every organisation's data lives in many **systems of record** — OLTP databases,
internal services, one per team. Every consumer needs uniform access to it, and in
2026 the most demanding new consumer is the **AI agent**.

An agent reasoning over your business needs a single, typed, ontology-shaped view
of **live** operational data — not a pile of bespoke per-table tools, not a
nightly export, not a vector copy that went stale an hour ago. Today, giving it
that means brittle ETL pipelines, warehouse copies, and one-off integration glue
for every source. The same pain hits analytics, BI, and compliance: data is
siloed across systems that each speak their own schema, and unifying them means
moving and duplicating it ([ADR-0001](docs/adr/ADR-0001-semantic-fabric-rust-data-fabric.md)).

The standard fix — copy everything into a warehouse or lake — trades freshness
and cost for convenience. semantic-fabric takes the other path: **leave the data
where it is and query it in place** ([ADR-0002](docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md)).

## 2. The solution — what semantic-fabric is

semantic-fabric exposes your relational sources as **one virtual RDF knowledge
graph** and answers SPARQL 1.2 by **rewriting each query to SQL that runs directly
against the live database** through R2RML mappings ([ADR-0003](docs/adr/ADR-0003-shared-core-two-frontend-architecture.md),
[ADR-0007](docs/adr/ADR-0007-sparql-to-sql-rewriting-strategy.md)).

The data is **never copied, never materialised**. Instance data (the A-Box) is
generated on demand at query time, streamed out, and discarded. Only the ontology
hierarchy `T` and the mappings `M` — the in-memory ⟨T, M⟩ pair — live in the
engine ([ADR-0004](docs/adr/ADR-0004-oxigraph-rdf-sparql-substrate.md),
[ADR-0002](docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md)).

|  | ETL / warehouse | semantic-fabric |
|---|---|---|
| Setup | Build + schedule a pipeline; stand up a warehouse | Point at a DB + one R2RML file |
| Freshness | As stale as the last batch | Always live (read in place) |
| Storage | A full second copy of the data | None — nothing is copied |
| Query | SQL over the copy | SPARQL over the live source |

## 3. Why "semantic fabric" (the name)

**Semantic** — an ontology/RDF layer gives *meaning* and a uniform vocabulary
across heterogeneous schemas, so a `Route` is a `Route` whatever the underlying
table calls it ([ADR-0001](docs/adr/ADR-0001-semantic-fabric-rust-data-fabric.md)).

**Fabric** — a thin weave *over* your systems of record that unifies them
**without moving the data**, in contrast to a warehouse or lake that copies it.
The weave is literal: there is no OLAP engine or staging layer in the middle —
the source does the set-work and the engine just rewrites and streams
([ADR-0006](docs/adr/ADR-0006-crate-layout-and-performance-model.md)).

The name encodes the architecture: **meaning on top, live sources underneath, no
copy in between.**

## 4. Why it is amazing

- **Constant engine memory under growing source data** — the source DB does the
  set-work and spills natively; engine memory is bounded by `|T| + |M| + a fixed
  batch budget`, independent of dataset size. **Proven byte-for-byte** (see
  [§7](#7-benchmarks--comparison)) — [ADR-0006](docs/adr/ADR-0006-crate-layout-and-performance-model.md),
  [ADR-0010](docs/adr/ADR-0010-security-and-resource-governance.md).
- **No JVM anywhere on the runtime path** — a single native Rust binary, in place
  of a Jena/Fuseki-class stack ([ADR-0008](docs/adr/ADR-0008-reasoning-strategy.md),
  [ADR-0019](docs/adr/ADR-0019-rdf-sparql-shacl-12-readiness.md)).
- **No copy / no ETL** — point it at a DB + an R2RML file and query live
  ([ADR-0002](docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md),
  [ADR-0003](docs/adr/ADR-0003-shared-core-two-frontend-architecture.md)).
- **Standards-grounded correctness** — W3C RDB2RDF conformance (81/82 SQLite,
  80/81 PostgreSQL; one documented deviation) over an ISWC-2018-based,
  provably-correct rewrite ([ADR-0005](docs/adr/ADR-0005-conformance-and-benchmark-harness.md),
  [ADR-0007](docs/adr/ADR-0007-sparql-to-sql-rewriting-strategy.md)).
- **Two real backends today** — SQLite + PostgreSQL via a dialect-correct SQL
  layer ([ADR-0015](docs/adr/ADR-0015-datatype-dialect-correctness.md)). _(SQLite
  is the wired executor today; see [§9](#9-status--limitations).)_
- **Built-in query-time provenance/lineage and a security edge** — named-graph
  lineage plus source RLS / rewriter ABAC / data-sensitivity authZ
  ([ADR-0017](docs/adr/ADR-0017-provenance-lineage.md),
  [ADR-0018](docs/adr/ADR-0018-security-edge.md)).

## 5. How it works — and why the HOW is part of why it's amazing

```
SPARQL 1.2
   │  parse (Oxigraph / spargebra)
   ▼
algebra ── unfold against M  +  T-saturation (tier-1 entailment)
   │  ISWC-2018 base translation
   ▼
relational plan ── 6-rule cascade  +  term-construction lifting  +  plan cache
   │  dialect SQL (SQLite / PostgreSQL)
   ▼
live source ── executes the set-work, spills natively
   │  RowStream (bounded batches)
   ▼
RDF terms streamed out  (A-Box never materialised)
```

- **5a. No-JVM Rust + Oxigraph crates** (parser/terms, *not* the store) — fast
  startup, a tiny footprint, a single embeddable binary
  ([ADR-0004](docs/adr/ADR-0004-oxigraph-rdf-sparql-substrate.md),
  [ADR-0019](docs/adr/ADR-0019-rdf-sparql-shacl-12-readiness.md)).
- **5b. The ⟨T, M⟩ pair held in memory, A-Box never materialised** — this is
  *why* memory is constant: the engine never holds the instance data, only the
  schema and mappings ([ADR-0003](docs/adr/ADR-0003-shared-core-two-frontend-architecture.md),
  [ADR-0004](docs/adr/ADR-0004-oxigraph-rdf-sparql-substrate.md)).
- **5c. SPARQL→SQL via the ISWC-2018 cascade + term-construction lifting + plan
  cache** — correctness and speed come from a *published, verified* translation,
  not ad-hoc string SQL ([ADR-0007](docs/adr/ADR-0007-sparql-to-sql-rewriting-strategy.md)).
- **5d. Constant-memory streaming** (`RowStream`, bounded `O(|T| + |M| + batch)`)
  — the headline property: the engine streams fixed-size batches while the source
  does the heavy lifting ([ADR-0006](docs/adr/ADR-0006-crate-layout-and-performance-model.md),
  [ADR-0010](docs/adr/ADR-0010-security-and-resource-governance.md)).
- **5e. Reasoning folded into the rewrite** — tier-1 hierarchy (subclass,
  subproperty, inverse, symmetric) is unfolded into the query; transitive
  properties are served live via single-predicate `P+` / `P*` recursive CTEs —
  entailment with **no separate reasoner and no JVM**
  ([ADR-0008](docs/adr/ADR-0008-reasoning-strategy.md)).
- **5f. W3C conformance + dialect/datatype canonicalisation (R2RML §10)** — why
  results are byte-correct across SQLite & PostgreSQL despite their different type
  systems ([ADR-0005](docs/adr/ADR-0005-conformance-and-benchmark-harness.md),
  [ADR-0015](docs/adr/ADR-0015-datatype-dialect-correctness.md)).

## 6. Use cases

**Agent data-access layer.** Give an LLM agent **one ontology-typed SPARQL
interface** over your live operational DBs instead of N bespoke per-table tools.
The agent queries meaning, not schema; query-time lineage tells it (and you) where
every answer came from ([ADR-0001](docs/adr/ADR-0001-semantic-fabric-rust-data-fabric.md),
[ADR-0017](docs/adr/ADR-0017-provenance-lineage.md)).

**Federated read over OLTP systems of record.** Query live transactional
databases *as a graph* without standing up a warehouse; cross-source joins run as
bounded semi-joins, not a full copy ([ADR-0006](docs/adr/ADR-0006-crate-layout-and-performance-model.md)).

**Ontology-driven access without an ETL pipeline.** Ship an R2RML mapping and
query immediately — no nightly batch, no staleness window
([ADR-0002](docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md),
[ADR-0003](docs/adr/ADR-0003-shared-core-two-frontend-architecture.md)).

**Query a live database as a knowledge graph.** Expose existing relational schemas
through a shared vocabulary for BI and SPARQL consumers, without re-platforming the
data ([ADR-0007](docs/adr/ADR-0007-sparql-to-sql-rewriting-strategy.md)).

## 7. Benchmarks & comparison

> **Honesty contract.** semantic-fabric's load-bearing result is the **constant
> engine memory under growing source data** invariant — a property of the
> streaming architecture, demonstrated byte-for-byte below. The sf-vs-Ontop
> latency tables are **not** a clean apples-to-apples race (different process
> model and backend). Full methodology, caveats, and reproduction live in
> **[BENCHMARKS.md](BENCHMARKS.md)**.

Workload: a deterministic, cross-reference-consistent subset of the
**[GTFS-Madrid-Bench](https://github.com/oeg-upm/gtfs-bench)**, vendored verbatim
under `crates/sf-bench/vendor/gtfs-madrid-bench/` (see its `PROVENANCE.md`).
Hardware: Apple M5 Max, macOS 26.4, rustc 1.96.0; sf over embedded SQLite
(in-process, `--release` via criterion) ([ADR-0005](docs/adr/ADR-0005-conformance-and-benchmark-harness.md),
[ADR-0006](docs/adr/ADR-0006-crate-layout-and-performance-model.md)).

### 7a. Constant engine memory — the differentiator

```bash
cargo bench -p sf-bench --bench constant_memory
```

Peak engine heap during the streamed CONSTRUCT dump (source data lives in a
file-backed SQLite DB, off the engine heap):

| Scale | Triples | Peak engine heap | Bytes / triple |
|---|---|---|---|
| 1x | 5 200 | **129 358 B** | 24.877 |
| 10x | 51 880 | **129 358 B** | 2.493 |
| 100x | 518 680 | **129 358 B** | 0.249 |

The engine peak heap is **byte-identical (129 358 B) across a 100× growth in
source data and result size**, while bytes/triple collapses toward zero — the
`O(|T| + |M| + batch)` invariant, demonstrated. The same property is asserted as a
fast unit test (peak-heap growth exactly 1× against a 16× row growth):

```bash
cargo test -p sf-bench --test constant_memory -- --nocapture
# 1x→5 200, 4x→20 760, 16x→83 000 triples — all 129 358 B; test PASSED
```

### 7b. Per-query OBDA latency

```bash
cargo bench -p sf-bench --bench obda_latency
```

criterion medians @1x, in-process (SQLite):

| Query | Shape | Median @1x |
|---|---|---|
| Q1 routes BGP | single-table BGP | 29.35 µs |
| Q2 route → agency join | 2-way join | 37.83 µs |
| Q3 stop_time → trip → route join | 3-way join | 738.17 µs |
| Q4 route FILTER | pushed-down FILTER | 26.69 µs |
| Q5 trip OPTIONAL headsign | NULL-safe left join | 96.35 µs |

Streamed CONSTRUCT dump — first-result latency stays bounded (~65 µs) while total
grows linearly with the result (the non-materialising path):

| Scale | Triples | First result | Total |
|---|---|---|---|
| 1x | 5 200 | 67.0 µs | 3.284 ms |
| 10x | 51 880 | 64.2 µs | 28.431 ms |
| 100x | 518 680 | 64.4 µs | 283.017 ms |

### 7c. Ontop comparison (real local run)

Real **Ontop 5.5.0** numbers, captured on the same machine over the **same logical
GTFS dataset** loaded into PostgreSQL 17.7 (Java 23, PG JDBC 42.7.4), warm HTTP
endpoint, median of 15 round-trips (`scripts/run_ontop_bench.sh`). **Answer
cardinalities matched semantic-fabric exactly at every scale** (parity confirmed):

| Query | Ontop @1x | Ontop @10x |
|---|---|---|
| Q1 routes BGP | 1.96 ms | 3.07 ms |
| Q2 route → agency join | 2.02 ms | 2.42 ms |
| Q3 stop_time → trip → route join | 13.44 ms | 56.74 ms |
| Q4 route FILTER | 1.46 ms | 1.40 ms |
| Q5 trip OPTIONAL headsign | 1.75 ms | 3.20 ms |
| CONSTRUCT dump (Turtle) | 110.7 ms | 1.272 s |

> **Read before comparing.** This is **not** a clean head-to-head. sf is an
> in-process Rust library over embedded SQLite (no HTTP, no serialization); Ontop
> is a warm JVM HTTP endpoint over PostgreSQL (includes HTTP + result
> serialization, ~1 ms floor). The backends differ (sf has no PostgreSQL executor
> yet). **Both are virtualisers** — neither materialises — so "no materialisation"
> is *common ground*, not an sf advantage over Ontop. The defensible, load-bearing
> result is the **constant-memory invariant**. See
> [BENCHMARKS.md › Measurement asymmetry](BENCHMARKS.md#measurement-asymmetry-read-this-before-comparing).

### 7d. OSS OBDA / RDB-to-RDF landscape

Qualitative positioning (all competitor facts cited; no head-to-head speed number
is claimed — the canonical GTFS-Madrid-Bench paper publishes *completeness*
tables, not timings, and explicitly declines to rank engines):

| Engine | Runtime | Virtual? | Mapping→SQL approach | Backends | Maturity |
|---|---|---|---|---|---|
| **semantic-fabric** | **Rust (no JVM, single binary)** | **Yes (virtualisation-only, never materialises)** | SPARQL 1.2 → SQL over R2RML | SQLite, PostgreSQL | early/public; serve not built; paths `P+`/`P*` single-predicate; W3C 81/82 SQLite, 80/81 PG (1 deviation) |
| [Ontop](https://github.com/ontop/ontop) | Java/JVM | Yes (core) | SPARQL→datalog→optimised SQL, R2RML/.obda | Many RDBMS | Mature, maintained, reference |
| [Morph-RDB](https://github.com/oeg-upm/morph-rdb) | JVM (Scala/Java) | Yes (+materialise) | SPARQL→SQL, R2RML | JDBC RDBMS | Unmaintained since 2019 (v3.12.5) |
| [Squerall](https://github.com/EIS-Bonn/Squerall) | JVM (Scala/Spark+Presto) | Yes | Data-lake OBDA, distributed | CSV/Parquet/Mongo/Cassandra/JDBC | Research (SANSA) |
| [Morph-KGC](https://github.com/morph-kgc/morph-kgc) | Python | No (materialises) | n/a — builds RDF | RDB/CSV/JSON/XML via RML | Maintained, active |
| [RMLMapper](https://github.com/RMLio/rmlmapper-java) | Java | No (materialises) | n/a | RML sources | Reference RML tool |
| [SDM-RDFizer](https://github.com/SDM-TIB/SDM-RDFizer) | Python | No (materialises) | n/a | RML sources | Active, fast |

Context (cited): on GTFS-Madrid-Bench, Ontop has historically answered only ~half
the 18 queries (failing on OPTIONAL-with-NULLs and arithmetic FILTER/date
expressions, and needing its memory cap raised from 512 MB to 8 GB) — Morph-CSV
paper §5.2, [arXiv 2001.09052](https://arxiv.org/pdf/2001.09052). The canonical
benchmark ([J. Web Semantics 65 (2020) 100596](https://www.sciencedirect.com/science/article/pii/S1570826820300354))
publishes result-completeness tables, **not** a speed ranking. No precise
published per-query GTFS-Madrid-Bench *time* is citable for any virtualisation
engine — hence semantic-fabric positions on **Rust / no-JVM / single-binary,
virtualisation-only, and constant engine memory**, not a claimed speed win.

## 8. Quick start / how to use

**Prerequisites:** the pinned Rust toolchain (`rust-toolchain.toml`, channel
1.96.0); PostgreSQL is optional (only needed for the Ontop comparison harness — sf
itself runs over embedded SQLite).

```bash
# Build the workspace and see the CLI
cargo build --workspace
cargo run -p sf-cli -- --help          # the `semantic-fabric` binary (ADR-0006)
```

The single binary exposes three subcommands:

| Subcommand | What it does | Status |
|---|---|---|
| `conformance` | Runs the **real W3C RDB2RDF suite** (SQLite) with EARL reporting ([ADR-0005](docs/adr/ADR-0005-conformance-and-benchmark-harness.md)) | Working |
| `bench` | Runs the **GTFS-Madrid OBDA driver** — live SPARQL→SQL over SQLite ([ADR-0005](docs/adr/ADR-0005-conformance-and-benchmark-harness.md)/[0006](docs/adr/ADR-0006-crate-layout-and-performance-model.md)) | Working |
| `serve` | SPARQL 1.2 Protocol HTTP endpoint | **Roadmap — not built; prints not-implemented & exits non-zero** ([ADR-0003](docs/adr/ADR-0003-shared-core-two-frontend-architecture.md)/[0007](docs/adr/ADR-0007-sparql-to-sql-rewriting-strategy.md)) |

```bash
cargo run -p sf-cli -- conformance     # W3C RDB2RDF over SQLite (prints pass/deviation summary)
cargo run -p sf-cli -- bench           # GTFS-Madrid OBDA: rewrites SPARQL → SQL, streams from SQLite
cargo run -p sf-cli -- serve           # NOT YET BUILT — reports not-implemented
```

**Minimal walkthrough.** Give the engine (a) an R2RML mapping `M` describing how a
SQLite table maps to RDF and (b) a SPARQL query. The vendored GTFS example
(`crates/sf-bench/vendor/gtfs-madrid-bench/gtfs-rdb.r2rml.ttl` + the queries in
`crates/sf-bench/src/workload.rs`) is the end-to-end reference exercised by `bench`
today; the rewriter unfolds the SPARQL against `M`, emits dialect SQL, runs it
against the live SQLite source, and streams RDF terms back out — no copy, no
materialisation.

## 9. Status & limitations

**Works today.** SPARQL 1.2 → SQL over R2RML + Direct Mapping; SQLite +
PostgreSQL dialects; conformance and bench end-to-end; named-graph output; R2RML
§10 datatype canonicalisation; constant-memory streaming.

**Limitations — stated plainly so the numbers are not over-read:**

| Area | Honest status |
|---|---|
| `serve` HTTP endpoint | **Not built.** Scaffold returns not-implemented; all numbers come from the in-process bench/test harness ([ADR-0003](docs/adr/ADR-0003-shared-core-two-frontend-architecture.md)/[0007](docs/adr/ADR-0007-sparql-to-sql-rewriting-strategy.md)) |
| Backend executor | **SQLite-only** today. PostgreSQL SQL is *emitted* (`Dialect::Postgres`) but not yet *executed* ([ADR-0002](docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md)) |
| Property paths | **Single-predicate `P+` / `P*` only** — no arbitrary path expressions ([ADR-0008](docs/adr/ADR-0008-reasoning-strategy.md)) |
| W3C RDB2RDF conformance | **81/82 (SQLite), 80/81 (PostgreSQL)** — **not 100%** — with one documented standards deviation, `R2RMLTC0002f` ([ADR-0005](docs/adr/ADR-0005-conformance-and-benchmark-harness.md), [ADR-0015](docs/adr/ADR-0015-datatype-dialect-correctness.md)) |
| Out of scope | Heterogeneous (CSV/JSON/XML) sources, `SERVICE` federation, FNML ([ADR-0002](docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md), [ADR-0014](docs/adr/ADR-0014-production-hardening-backlog.md)) |

Features outside the v1 surface return 501 / are skipped — they are not silently
wrong, but they are not done.

## 10. Architecture / workspace

Seven crates ([ADR-0006](docs/adr/ADR-0006-crate-layout-and-performance-model.md)
crate layout, [ADR-0003](docs/adr/ADR-0003-shared-core-two-frontend-architecture.md)
shared-core/frontend split, [ADR-0004](docs/adr/ADR-0004-oxigraph-rdf-sparql-substrate.md)
substrate):

| Crate | Role |
|---|---|
| `sf-core` | Shared core: R2RML mapping IR, RDF term generation, R2RML §10 datatypes |
| `sf-sql` | Source/SQL layer: connectors, dialects (SQLite/PostgreSQL), schema introspection |
| `sf-mapping` | R2RML / Direct-Mapping parser → core IR |
| `sf-sparql` | The virtualiser: SPARQL 1.2 → SQL rewriter (instance data never materialised) |
| `sf-conformance` | W3C RDB2RDF harness + EARL + `M ⋈ T` SHACL gate |
| `sf-bench` | GTFS-Madrid OBDA benchmark driver |
| `sf-cli` | The single binary: `serve · conformance · bench` |

## 11. Decision records

The full ADR corpus — 18 records (0001–0008, 0010–0015, 0017–0020; **0009 folded
into 0004, 0016 deleted**). Each prior section cites its ADRs inline; this is the
canonical index.

**Charter & scope**

| ADR | Decision |
|---|---|
| [ADR-0001](docs/adr/ADR-0001-semantic-fabric-rust-data-fabric.md) | A custom Rust OBDA data fabric — virtualisation over relational sources (charter) |
| [ADR-0002](docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md) | Scope: virtualisation-only OBDA over relational databases via R2RML |

**Architecture**

| ADR | Decision |
|---|---|
| [ADR-0003](docs/adr/ADR-0003-shared-core-two-frontend-architecture.md) | The virtualiser pipeline (SPARQL 1.2 → SQL over R2RML) |
| [ADR-0004](docs/adr/ADR-0004-oxigraph-rdf-sparql-substrate.md) | Oxigraph crates as the RDF/SPARQL substrate; own the rewriter, hold ⟨T, M⟩ in memory |

**Correctness & performance**

| ADR | Decision |
|---|---|
| [ADR-0005](docs/adr/ADR-0005-conformance-and-benchmark-harness.md) | W3C RDB2RDF + GTFS/KROWN harness — correctness gate and fitness function |
| [ADR-0006](docs/adr/ADR-0006-crate-layout-and-performance-model.md) | Crate layout, execution & performance model (push-down + semi-join; no OLAP intermediary) |
| [ADR-0007](docs/adr/ADR-0007-sparql-to-sql-rewriting-strategy.md) | SPARQL→SQL rewriting + cascade correctness (ISWC-2018 base translation) |
| [ADR-0015](docs/adr/ADR-0015-datatype-dialect-correctness.md) | Datatype & dialect correctness — R2RML §10 canonicalization (SQLite affinity) |

**Reasoning**

| ADR | Decision |
|---|---|
| [ADR-0008](docs/adr/ADR-0008-reasoning-strategy.md) | Entailment folded into the rewrite, native Rust, no runtime JVM |

**Ops & security**

| ADR | Decision |
|---|---|
| [ADR-0010](docs/adr/ADR-0010-security-and-resource-governance.md) | Security & resource governance (injection-safety, `P+` DoS limits) |
| [ADR-0011](docs/adr/ADR-0011-observability-and-configuration.md) | Observability (`tracing` + `metrics`/Prometheus) + configuration model |
| [ADR-0018](docs/adr/ADR-0018-security-edge.md) | Security edge — source RLS + rewriter ABAC + data-sensitivity authZ |

**Quality & process**

| ADR | Decision |
|---|---|
| [ADR-0012](docs/adr/ADR-0012-test-strategy.md) | Test strategy (unit/integration/property/fuzz + snapshot) |
| [ADR-0013](docs/adr/ADR-0013-meta-harness-dev-loop.md) | Meta-harness dev loop (readiness drift + perf tuning) |

**Data & provenance**

| ADR | Decision |
|---|---|
| [ADR-0017](docs/adr/ADR-0017-provenance-lineage.md) | Provenance & lineage — query-time named graphs + PROV-O |

**Readiness & roadmap**

| ADR | Decision |
|---|---|
| [ADR-0014](docs/adr/ADR-0014-production-hardening-backlog.md) | Production-hardening backlog (acknowledged-deferred) |
| [ADR-0019](docs/adr/ADR-0019-rdf-sparql-shacl-12-readiness.md) | RDF 1.2 / SPARQL 1.2 / SHACL readiness — Rust stack in place of a JVM |
| [ADR-0020](docs/adr/ADR-0020-outstanding-sota-optimisations.md) | Outstanding SOTA optimisations — research register & dispositions |

## 12. Research grounding / prior art

These decisions stand on peer-reviewed work, not invention
([ADR-0001](docs/adr/ADR-0001-semantic-fabric-rust-data-fabric.md) "stand on
proven designs"; [ADR-0020](docs/adr/ADR-0020-outstanding-sota-optimisations.md)
SOTA register). The full literature review lives in
[`docs/research/`](docs/research/): Ontop, Morph-KGC, SDM-RDFizer,
RMLMapper/Streamer, Oxigraph, R2RML + W3C tests, RML/YARRRML, the SPARQL→SQL
cascade-correctness result, OBDA resource governance, external-memory join,
dialect correctness, virtualization streaming, and the Rust substrate survey.

## 13. Contributing & license

Dual-licensed under **[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE)** — your
choice. Before opening a PR, run the same gates CI runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
cargo test --workspace
cargo run -p sf-cli -- conformance     # no UNEXPECTED W3C RDB2RDF regressions
```

Architectural changes follow the ADR process under [`docs/adr/`](docs/adr/) — open
or amend an ADR alongside the code so every claim stays backed by a record.
