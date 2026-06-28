# Rust Substrate for semantic-fabric: SOTA Performance in Both Modes

> **SUPERSEDED (execution stack), 2026-06-27.** This round-1 report recommends **DataFusion + Arrow + `connector_arrow` + Polars** as the execution core. That recommendation is **dropped** — see **ADR-0015** + `external-memory-dedup.md` / `external-memory-join.md`: the materializer **pushes scan / join / DISTINCT into the source DB** (which spills natively) and uses **DuckDB** for the cross-source residual; DataFusion's hash-join + hash-aggregate/DISTINCT do **not** spill (June 2026, `apache/datafusion#17267`). The crate-survey *facts* below remain useful; the *architecture recommendation* (DataFusion as the engine) does not.

*Research date: 2026-06-26. Topic: crate stack to achieve SOTA throughput and scalability for
(1) R2RML materialization and (2) SPARQL-to-SQL virtualization/OBDA — sharing one Rust core.*

---

## 1. Problem Framing

semantic-fabric must run two modes from a single mapping IR:

| Mode | Data flow | Bottleneck |
|------|-----------|------------|
| **Materialization** | RDBMS → Arrow batches → RDF terms → N-Triples on disk | CPU (IRI construction, dedup) + DB bulk scan throughput |
| **OBDA / virtualization** | SPARQL query → SQL rewrite → RDBMS → streaming result | SQL planning, DB round-trip latency, async concurrency |

Both modes share: mapping IR parsing, IRI template evaluation, term dictionary, triple encoding, and the SPARQL algebra → SQL mapping step. The substrate choices must serve both without duplication.

---

## 2. Vectorized Columnar Execution: Apache DataFusion + Arrow

[Apache DataFusion](https://github.com/apache/datafusion) is the natural core for both modes. It is a fully embeddable, extensible, multi-threaded, vectorized SQL/dataflow engine that uses [Apache Arrow](https://arrow.apache.org/) as its in-memory format. A 2024 SIGMOD paper — [Apache Arrow DataFusion: A Fast, Embeddable, Modular Analytic Query Engine](https://dl.acm.org/doi/10.1145/3626246.3653368) — positions it as the foundation for production systems (InfluxDB 3.0, Apple Comet/Spark accelerator).

**For the materializer:** DataFusion's `TableProvider` trait wraps an RDBMS connection; its query planner generates a partitioned `ExecutionPlan` that yields `RecordBatch` streams. IRI template expansion runs as a vectorized Arrow compute kernel over string columns. The result is a columnar triple stream fed directly to a streaming N-Triples serializer — no heap-allocated `Vec<Triple>` intermediary.

**For OBDA:** DataFusion's optimizer + the [`datafusion-federation`](https://crates.io/crates/datafusion-federation) crate perform automatic SQL predicate/projection pushdown. A SPARQL query decomposes into a logical plan; the federation optimizer identifies sub-plans fully expressible as SQL, cuts them out, and ships them to the RDBMS — avoiding any columnar materialization in the Rust process.

**Critical proof-of-concept:** [rdf-fusion](https://github.com/tobixdev/rdf-fusion) (v0.1.0, IEEE Access 2025 — [paper](https://www.researchgate.net/publication/396724140_RDF_Fusion_An_Extensible_SPARQL_Engine_for_Hybrid_Data_Models)) is an experimental columnar SPARQL engine built on DataFusion, started as a fork of Oxigraph. It proves:
- Arrow columns can encode heterogeneous RDF terms via **dual encoding** — one column layer for lexical values (join equality), a second for typed/parsed values (arithmetic) — sidestepping the heterogeneity problem without expensive runtime type-checking.
- DataFusion's optimizer and execution engine handle SPARQL algebra well (joins, aggregates, optionals).
- rdf-fusion outperforms Oxigraph's row-based engine on queries that process large result sets.

semantic-fabric should study rdf-fusion's `rdf-fusion-encoding` crate design when laying out its own Arrow schema for intermediate triples.

---

## 3. RDBMS Extraction

Two distinct patterns are needed; the crates differ.

### 3a. Bulk extraction (materialization)

[`connector_arrow`](https://github.com/aljazerzen/connector_arrow) (crate: `connector_arrow`) is the right choice: a pure-Rust, safe, Cargo-compiled Arrow database client modeled after ADBC. It converts result sets directly to `RecordBatch` without a Python or C FFI layer. Supports PostgreSQL, DuckDB, SQLite via feature flags; MySQL is tracked. Its mandate: minimal deps, Arrow-only destination, no built-in pooling (composable).

The [original ConnectorX](https://github.com/sfu-db/connector-x) benchmarks (against Pandas + Turbodbc, TPC-H SF10, 4-partition parallel) show 3x less memory and 13x less time for PostgreSQL, and up to 14x faster for MSSQL — evidence that the zero-copy, partition-parallel approach is sound. `connector_arrow` carries the same architecture into a library-first Rust API.

The [`datafusion-table-providers`](https://github.com/datafusion-contrib/datafusion-table-providers) crate provides ready-made `TableProvider` wrappers for PostgreSQL, MySQL, SQLite, ClickHouse, and DuckDB, integrated with `datafusion-federation`. For a project starting from scratch, this is the fastest path to a working materialization pipeline with automatic partition planning.

### 3b. Live query execution (OBDA)

For OBDA, each incoming SPARQL request generates one or more SQL queries that execute against the live RDBMS and return a small-to-medium result set. Here, latency matters more than bulk throughput.

[`tokio-postgres`](https://crates.io/crates/tokio-postgres) is the lowest-overhead async PostgreSQL driver. A [benchmarked issue in the sqlx repo](https://github.com/launchbadge/sqlx/issues/2436) documents a roughly 150x throughput gap between tokio-postgres and sqlx for simple queries — sqlx's compile-time macro machinery adds substantial overhead per query. For OBDA hot paths, use `tokio-postgres` v0.7.x directly, with [`deadpool-postgres`](https://crates.io/crates/deadpool-postgres) or [`bb8-postgres`](https://crates.io/crates/bb8) for connection pooling.

For MySQL targets, `mysql_async` v0.34+ fills the equivalent role.

**Decision matrix:**

| Scenario | Crate | Reason |
|----------|-------|--------|
| Bulk table scan (materialization) | `connector_arrow` | Arrow-native, zero-copy, partition-parallel |
| Federated DataFusion scan | `datafusion-table-providers` | TableProvider integration, federation pushdown |
| Live SPARQL→SQL (OBDA) | `tokio-postgres` + `deadpool-postgres` | Lowest latency, no abstraction overhead |

---

## 4. SQL Dialect Emission: sqlparser-rs

[`sqlparser`](https://crates.io/crates/sqlparser) (now maintained as [`apache/datafusion-sqlparser-rs`](https://github.com/apache/datafusion-sqlparser-rs)) is the standard Rust SQL AST library. It is DataFusion's own SQL frontend.

For OBDA, semantic-fabric needs to emit target-dialect SQL from the SPARQL-to-SQL rewrite. The flow: SPARQL → DataFusion logical plan → SQL AST (via sqlparser types) → `ast.to_string()` in the target dialect. The crate's `Display` impl normalizes to compact SQL; the `{:#}` formatter pretty-prints. Dialects cover PostgreSQL, MySQL, MSSQL, SQLite, Snowflake, BigQuery, DuckDB, Hive, Oracle, Teradata, Redshift, ClickHouse, Spark, and Databricks — selectable at runtime via `dialect_from_str("postgres")`.

The `Visitor` trait enables recursive AST rewriting, which is needed for R2RML template expansion (e.g., rewriting `CONCAT` fragments into native dialect equivalents like `||` on PostgreSQL vs `CONCAT()` on MySQL).

The Ontop system ([github.com/ontop/ontop](https://github.com/ontop/ontop)) is the reference for SPARQL-to-SQL correctness. Its approach — translate SPARQL + R2RML mappings to Datalog, apply query containment-based optimizations to eliminate redundant self-joins, then emit SQL — has been validated against [GTFS-Madrid-Bench](https://github.com/oeg-upm/gtfs-bench) and shown to outperform other SPARQL-to-SQL systems by orders of magnitude. semantic-fabric's OBDA mode should replicate the join-elimination step; sqlparser-rs is the correct tool to emit the resulting SQL.

---

## 5. Polars for Partition Processing

[Polars](https://github.com/pola-rs/polars) v0.46.x brings a fully Rust-native, Rayon-parallel DataFrame engine that can process R2RML mapping partitions independently. Key capabilities:

- **Partition-aware groupby**: threshold-controlled via `POLARS_PARTITION_UNIQUE_COUNT`; for high-cardinality join columns (common in R2RML subject maps), partitioned execution distributes load across cores.
- **Streaming mode**: processes queries in fixed-memory chunks for tables that exceed RAM — important for the target scale (GTFS-Madrid-Bench scale factors 1–10).
- **Arrow interop**: Polars DataFrames export to `arrow::RecordBatch` directly, fitting into the DataFusion pipeline.

**Caution:** Polars uses its own Rayon thread pool. Mixing Polars inside a Tokio async context causes thread-pool contention and latency spikes (see [PostHog writeup](https://posthog.com/blog/untangling-rayon-and-tokio)). The recommended pattern: confine Polars partition processing to a `tokio::task::spawn_blocking` block, keeping the Tokio executor's I/O threads uncontested.

Polars is best suited as an optional high-level join/aggregate step *within* the materializer pipeline, not as the primary execution layer. DataFusion's own partitioned scan + sort-merge join is sufficient for the core; Polars adds value for complex mapping-level transformations.

---

## 6. Parallelism: rayon + tokio

The canonical Rust data-pipeline pattern:

```
tokio runtime (I/O: DB connections, async result streaming)
    ↓  channel (flume or tokio::sync::mpsc)
rayon thread pool (CPU: IRI template expansion, dictionary insert, triple encoding)
    ↓  channel
tokio runtime (I/O: async write N-Triples to disk / network)
```

- **Tokio** v1.x handles all async I/O: DB connection lifecycle, result streaming, HTTP for OBDA responses.
- **Rayon** v1.10.x handles CPU-bound batch work: vectorized string operations on Arrow column buffers, IRI deduplication, dictionary lookups.
- Keep the two pools separate. As the [PostHog production post-mortem](https://posthog.com/blog/untangling-rayon-and-tokio) shows, mixing them (e.g., blocking rayon inside a tokio task) creates latency spikes of 20x+.
- For the materializer, consider `rayon::ThreadPoolBuilder::new().num_threads(N).build_global()` to leave headroom for tokio workers. For the OBDA mode (I/O-bound), rayon is barely needed; tokio's multi-thread scheduler suffices.

---

## 7. Term Dictionary Encoding / String Interning

Dictionary encoding maps long RDF term strings (IRIs, literals) to compact integer IDs at ingestion time. This is the primary mechanism for deduplication in both modes.

[`lasso`](https://github.com/Kixiron/lasso) v0.7.x provides:
- `Rodeo` (single-threaded): O(1) intern + resolve, arena-backed, no allocation per lookup after warmup.
- `ThreadedRodeo` (concurrent): lock-striped for parallel materialization pipelines. Suitable when multiple Rayon workers build the term dictionary concurrently.
- Keys are typed (`Spur` = u32 by default, or custom) — directly embeddable in Arrow `UInt32Array` columns to represent triple components.

The pattern for semantic-fabric:
1. Extract raw strings from Arrow `StringArray` columns (subject/predicate/object after IRI template expansion).
2. Batch-intern into a `ThreadedRodeo` — returns `Spur` IDs.
3. Build triple arrays as `UInt32Array` triplets — highly cache-friendly, sortable, dedup-able via Arrow sort + dedup kernels.
4. On output, resolve keys back to strings only once per unique IRI — write count is O(distinct terms), not O(triples).

For the OBDA mode, a simpler `Rodeo` per-request (or per-mapping-partition) is sufficient; the RDBMS does the heavy join work.

---

## 8. Streaming N-Triples Serialization

[`oxttl`](https://crates.io/crates/oxttl) (part of the Oxigraph project) provides a conformant, streaming N-Triples serializer and parser. It is the successor to the deprecated `rio_turtle`. The `oxrdfio` crate wraps oxttl behind a unified format-agnostic API (Turtle, TriG, N-Quads, N-Triples, RDF/XML).

For bulk materialization the pattern is:
- Instantiate an `NTriplesSerializer` from oxttl.
- Drive it with a `tokio::io::AsyncBufWriter` over a file or network socket.
- Feed Arrow batches → resolve dictionary keys → emit one triple at a time into the serializer's write call.
- Because the serializer writes line-by-line, memory stays bounded regardless of output volume.

A newer format worth tracking: [Jelly](https://arxiv.org/html/2506.11298v1) (2025) is a binary streaming RDF format with significantly better throughput than text N-Triples for large-scale materialization. Not yet in the Oxigraph family, but the architecture supports swappable serializers.

---

## 9. Oxigraph as the RDF/SPARQL Substrate

[`oxigraph`](https://github.com/oxigraph/oxigraph) v0.4.x provides:
- SPARQL 1.1 (Query + Update + Federated Query), with SPARQL 1.2 behind `rdf-12` feature.
- Parsers/serializers for Turtle, TriG, N-Triples, N-Quads, RDF/XML (via oxttl + oxrdfio sub-crates).
- `spargebra` sub-crate: a SPARQL algebra AST — the correct starting point for the SPARQL → SQL rewrite step.
- Storage: in-memory (no deps) or RocksDB-backed for persistent indexes.
- Performance (2026 benchmarks): warm point queries at 0.8 µs, complex SPARQL at 0.5 ms, 2.1 GB RAM for 100M triples vs ~24 GB for a naive triple store.

For semantic-fabric, Oxigraph serves three roles:
1. **SPARQL parser** (`spargebra`): parse the incoming SPARQL query into an algebra tree before the SQL rewrite.
2. **Ontology/mapping metadata store**: hold the parsed R2RML mapping graph in an in-memory Oxigraph store for fast lookup during rewriting.
3. **Result collector (OBDA)**: optionally reconstruct RDF result graphs from DB result rows to return via SPARQL protocol.

---

## 10. Recommended Crate Stack

| Role | Crate | Version (approximate) |
|------|-------|----------------------|
| SQL/dataflow engine | `datafusion` | 47.x |
| Arrow in-memory format | `arrow` (via datafusion) | 54.x |
| SPARQL algebra | `spargebra` (via oxigraph) | 0.4.x |
| RDF storage + SPARQL | `oxigraph` | 0.4.x |
| N-Triples streaming I/O | `oxttl` / `oxrdfio` | 0.4.x |
| Bulk DB extraction (Arrow) | `connector_arrow` | 0.4.x |
| DB TableProvider (DataFusion) | `datafusion-table-providers` | 0.x |
| SQL federation pushdown | `datafusion-federation` | 0.x |
| Live SQL (OBDA, PostgreSQL) | `tokio-postgres` | 0.7.x |
| Live SQL (OBDA, MySQL) | `mysql_async` | 0.34.x |
| Connection pooling | `deadpool-postgres` | 0.12.x |
| SQL AST + dialect emit | `sqlparser` (datafusion-sqlparser-rs) | 0.52.x |
| CPU parallelism | `rayon` | 1.10.x |
| Async runtime | `tokio` (full features) | 1.x |
| Term dictionary | `lasso` | 0.7.x |
| DataFrame partition processing | `polars` (streaming feature) | 0.46.x |

---

## 11. SOTA Targets and Benchmark Context

- **KROWN** ([kg-construct/KROWN](https://github.com/kg-construct/KROWN)): benchmarks RDF materialization systems (RMLMapper, RMLStreamer, Morph-KGC, SDM-RDFizer, Ontop). As of 2024, [Morph-KGC](https://github.com/morph-kgc/morph-kgc) (Python, pandas-based, mapping partitions) is the fastest Python engine; RMLMapper times out at high scale. A Rust engine with DataFusion + Arrow should beat all current KROWN leaders on throughput — the Python overhead alone is a 10–50x handicap.
- **GTFS-Madrid-Bench** ([oeg-upm/gtfs-bench](https://github.com/oeg-upm/gtfs-bench)): the standard OBDA benchmark. Ontop (Java) is the reference implementation and current leader for SPARQL-to-SQL correctness and speed. The Rust target is to match Ontop's SQL generation quality (join elimination, pushdown) while adding lower JVM startup latency and better memory efficiency.
- **W3C R2RML test cases**: 100% pass rate is a hard requirement before declaring SOTA. Run via the official [W3C RDB2RDF test suite](https://www.w3.org/TR/rdb2rdf-test-cases/).

---

## 12. Architecture Notes for Implementation

1. **Shared IR**: R2RML mappings should be parsed into an internal IR that is dialect-agnostic. Use `spargebra` term types for RDF terms in the IR; use sqlparser AST node types for SQL template fragments. This keeps both modes on the same code path.
2. **Two TableProvider implementations**: one wraps `connector_arrow` for bulk materialization scans; the other wraps `tokio-postgres` + DataFusion-compatible execution for OBDA. Both implement the same DataFusion `TableProvider` trait, so the planner is shared.
3. **Avoid Polars in the hot path**: use DataFusion's native sort-merge join for the main triple-generation pipeline. Add Polars as an optional transformation stage (via `spawn_blocking`) for complex mapping rules that benefit from DataFrame semantics.
4. **Dictionary encoding is core, not an add-on**: allocate the `lasso::ThreadedRodeo` at startup, share it (via `Arc`) across all materializer partitions, flush to disk after a run so warm restarts skip re-interning.
5. **rdf-fusion as prior art, not a dependency**: do not depend on `rdf-fusion` directly (it is 0.1.0 and experimental), but study its `rdf-fusion-encoding` crate's dual-column Arrow layout when designing the internal triple column schema.

---

## Sources

- [Apache DataFusion repo](https://github.com/apache/datafusion)
- [DataFusion SIGMOD 2024 paper](https://dl.acm.org/doi/10.1145/3626246.3653368)
- [rdf-fusion repo](https://github.com/tobixdev/rdf-fusion)
- [rdf-fusion IEEE Access 2025 paper](https://www.researchgate.net/publication/396724140_RDF_Fusion_An_Extensible_SPARQL_Engine_for_Hybrid_Data_Models)
- [connector_arrow (aljazerzen)](https://github.com/aljazerzen/connector_arrow)
- [ConnectorX repo + benchmarks](https://github.com/sfu-db/connector-x/blob/main/Benchmark.md)
- [sqlx vs tokio-postgres perf issue #2436](https://github.com/launchbadge/sqlx/issues/2436)
- [datafusion-table-providers](https://github.com/datafusion-contrib/datafusion-table-providers)
- [datafusion-sqlparser-rs (Apache)](https://github.com/apache/datafusion-sqlparser-rs)
- [sqlparser crate docs](https://docs.rs/sqlparser/latest/sqlparser/)
- [Polars repo](https://github.com/pola-rs/polars)
- [PostHog: Untangling Rayon and Tokio](https://posthog.com/blog/untangling-rayon-and-tokio)
- [lasso crate](https://github.com/Kixiron/lasso)
- [Oxigraph repo](https://github.com/oxigraph/oxigraph)
- [oxttl (via Oxigraph)](https://crates.io/crates/oxigraph)
- [Jelly RDF format paper](https://arxiv.org/html/2506.11298v1)
- [KROWN benchmark](https://github.com/kg-construct/KROWN)
- [KROWN results on Zenodo](https://zenodo.org/records/10973892)
- [GTFS-Madrid-Bench](https://github.com/oeg-upm/gtfs-bench)
- [Ontop repo](https://github.com/ontop/ontop)
- [Ontop SPARQL-to-SQL evaluation](https://ceur-ws.org/Vol-1015/paper_16.pdf)
- [Morph-KGC repo](https://github.com/morph-kgc/morph-kgc)
- [W3C R2RML test cases](https://www.w3.org/TR/rdb2rdf-test-cases/)
