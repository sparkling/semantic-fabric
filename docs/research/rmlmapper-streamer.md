# RMLMapper and RMLStreamer: Conformance Baseline and Streaming Architecture

**Research key**: `rmlmapper-streamer`
**Date**: 2026-06-26
**Scope**: RMLMapper as correctness reference for R2RML/RML test cases; RMLStreamer streaming architecture on Apache Flink and its implications for scalable Rust materialization in semantic-fabric.

---

## 1. Background: the R2RML/RML conformance landscape

The [W3C RDB2RDF Working Group](https://www.w3.org/2009/08/rdb2rdf-charter) published [R2RML: RDB to RDF Mapping Language](https://www.w3.org/TR/r2rml/) in 2012 together with an official [R2RML and Direct Mapping Test Suite](https://www.w3.org/TR/rdb2rdf-test-cases/). That suite contains 60+ individual test cases (D000–D025, with lettered variants) covering:

- Empty tables, single- and multi-row scenarios
- SQL datatypes: integer, float, double, date, timestamp, boolean, binary, string
- NULL value handling
- Primary/foreign key joins, many-to-many relations
- URI template generation, blank nodes, constant values, language tags
- Named graphs (specific and default), multiple graph assignments
- Referencing object maps and inverse expressions

Conformance is defined by exact N-Quads output match against the published expected output.

[RML](https://rml.io/specs/rml/) extends R2RML to non-relational sources (CSV, JSON, XML). The W3C Knowledge Graph Construction Community Group (KGC-CG) has since developed [RMLkgc](https://kg-construct.github.io/rml-resources/), modularising the spec into **RML-Core**, **RML-IO**, and **RML-LV** (Logical Views). Legacy [RML test cases](https://rml.io/test-cases/) (297 cases, RMLTC0000–RMLTC0020 with lettered variants) are now deprecated in favour of per-module test cases.

---

## 2. RMLMapper: the reference implementation

### 2.1 Project status

[RMLMapper](https://github.com/RMLio/rmlmapper-java) (Java, Maven, ~98% Java) is maintained by IDLab / rml.io. The latest release at research time is **v8.1.0** (December 2025), with 50 releases total. It is available via [Maven Central](https://mvnrepository.com/artifact/be.ugent.rml/rmlmapper). A [JavaScript port](https://github.com/comake/rmlmapper-js) exists but is third-party maintained.

### 2.2 Supported output formats

`nquads` (default), `turtle`, `trig`, `trix`, `jsonld`, `hdt`, `jelly`

The Jelly format was added in v8.0.0 (September 2025). N-Quads is the canonical choice for conformance testing.

### 2.3 RDBMS support

RMLMapper connects to **MySQL, PostgreSQL, Oracle, and SQL Server** via JDBC. The CLI exposes `-u` (username), `-p` (password), and `-dsn` (JDBC connection string). R2RML mappings are automatically converted to RML internally at execution time.

### 2.4 Conformance coverage

**W3C R2RML test cases (2012)**: RMLMapper passes 100% — the historical exception of automatic datatyping of literals has been resolved in recent versions. R2RML mappings are fully supported via auto-conversion to RML.

**RMLio (legacy spec)**: 100% pass rate.

**RMLkgc (W3C KGC-CG, introduced v7.0.0)** as reported in [KGCW Challenge 2025](https://ceur-ws.org/Vol-3999/short2.pdf):

| Module | Pass | Total | Coverage |
|--------|------|-------|----------|
| RML-Core | 53 | 59 | 89.83% |
| RML-IO | 41 | 73 | 56.16% |
| RML-LV (with view-to-CSV) | ~26 | 32 | 81.25% |
| All RMLkgc (overall) | — | — | 73.70% |

The backwards-compatibility paper ([KGCW 2024](https://openreview.net/forum?id=m6yJtJvu6y)) confirms that any R2RML or old-RMLio mapping can be executed by RMLMapper without modification.

### 2.5 Architecture and key limitations

RMLMapper follows a **load-everything-in-memory, then map** ETL model:

1. Parse mapping document (Turtle).
2. For each `TriplesMap`: execute the logical source query (JDBC `SELECT *` or custom SQL), fetch the full result set into memory.
3. Apply `SubjectMap`, `PredicateObjectMap`, join conditions, etc., generating triples.
4. Deduplicate the output triple set in memory using a set data structure.
5. Serialize to the chosen format.

This makes RMLMapper straightforward to implement correctly but creates well-documented scaling problems:

- **Memory**: All data loaded in RAM. XML parsing (DOM-based for full XPath) consumes up to 10x the file size.
- **Deduplication**: In-memory set. As the output KG grows, deduplication becomes the dominant cost.
- **Joins**: Nested-loop style. The [KROWN benchmark](https://kg-construct.github.io/KROWN/) shows timeout (>6 hours) for 5 or more join conditions.
- **Named graphs**: Times out on 15 named graph mappings; complex dynamic cases fail.
- **Large cell values**: Out-of-memory at increasing cell sizes (in KROWN, shared with Morph-KGC and SDM-RDFizer).

The [KROWN benchmark](https://kg-construct.github.io/KROWN/) ([results on Zenodo](https://zenodo.org/records/10973892)) benchmarks RMLMapper, RMLStreamer, Morph-KGC, SDM-RDFizer, and OntopM across axes: row count (10K–10M), column count (1–30), cell size (500B–10KB), duplicate ratio (0–100%), triple map count (1–30), predicate-object map count (1–10), named graph count (1–15), and join condition count (1–15).

### 2.6 Additional features

- **FnO functions**: dynamic function loading via JAR or `.java` files; enables data transformation within mappings.
- **PROV-O metadata**: provenance output via `-e`/`-l` flags.
- **CSVW support**: from v4.4.0.
- **W3C Web of Things**: from v4.13.0.
- **Remote sources**: SPARQL endpoints, HTTP APIs, JDBC.

---

## 3. RMLStreamer: Flink-based parallel materialization

### 3.1 Project status

[RMLStreamer](https://github.com/RMLio/RMLStreamer) (Scala 95%, Java 5%) runs on **Apache Flink 1.14.5** with Scala 2.11. The latest release is **v2.5.0** (June 2023). The project appears to be in low-activity maintenance — no releases since June 2023 as of the research date. Build requires JDK 11–13.

### 3.2 Architecture: Flink operator pipeline

RMLMapper's flat-serial approach is replaced by a **Flink job graph** where each `TriplesMap` becomes a chain of stream processing operators:

```
[Source connector] → [Record parser] → [Reference resolver] → [Map generator] → [Sink]
         ↑
   (parallelism across task slots)
```

Key design choices:

- **Data parallelism**: Input records are spread across available Flink task slots by default. The `-p N` flag controls parallelism degree (number of task slots used).
- **Operator chaining**: Adjacent single-input operators with the same concurrency are fused into one task, eliminating network serialization between them.
- **Ordering mode**: `--disable-local-parallel` preserves record order at the cost of parallelism.
- **Checkpointing**: Flink's built-in checkpointing and watermarking provide fault tolerance for streaming workloads.
- **No deduplication**: RMLStreamer deliberately omits duplicate removal, which gives it **constant memory usage** regardless of dataset size — at the cost of potentially non-conformant output for mappings that generate duplicates.

### 3.3 Supported sources and sinks

**Sources**: TCP socket streams, Kafka topics, MQTT, file-based JSON/CSV, JDBC (relational databases).

**Sinks**: File (`toFile` via StreamingFileSink), Kafka (`toKafka`), TCP socket (`toTCPSocket`), MQTT, or `noOutput` for benchmarking.

### 3.4 Output formats

N-Quads (default, faster) and JSON-LD (slower, object-grouped). No Turtle/Trig/HDT/Jelly.

### 3.5 RMLStreamer-SISO: streaming extension

[RMLStreamer-SISO (ISWC 2022)](https://iswc2022.semanticweb.org/wp-content/uploads/2022/11/978-3-031-19433-7_40.pdf) extends the base for single-input-single-output stream scenarios with a **dynamic windowing** approach for joining streaming data. It targets sensor/IoT use cases where RDF must be generated continuously with millisecond latency.

Reported performance from ISWC 2022: **~70,000 records/s** versus ~10,000 records/s for state-of-the-art tools at the time, with millisecond latency and constant memory across all workloads.

### 3.6 Conformance gaps

RMLStreamer's known conformance gaps matter for semantic-fabric:

- **No multiple join conditions**: Fails or produces incorrect output when a `ReferencingObjectMap` specifies more than one `JoinCondition`.
- **No multiple named graphs**: Does not support multiple graph map assignments per triple map.
- **No duplicate removal**: Output may contain duplicate triples; not conformant with R2RML semantics (which requires a set of triples).
- **Limited output formats**: Only N-Quads and JSON-LD.

From [KROWN](https://kg-construct.github.io/KROWN/): RMLStreamer uses constant time for any row-count scaling (because it skips deduplication) but times out on large cell values and fails on multi-join and multi-graph scenarios.

### 3.7 Benchmark infrastructure

The [rmlstreamer-benchmark-rust](https://github.com/s-minoo/rmlstreamer-benchmark-rust) repository (used for ISWC 2022) implements the data streamer component in **Rust**, with a containerized framework (data streamer + SUT + monitoring unit) for reproducible evaluation. This demonstrates that Rust is already used at the ingestion/benchmarking layer of the RML ecosystem.

---

## 4. Implications for semantic-fabric (Rust/Oxigraph)

### 4.1 RMLMapper as correctness oracle

RMLMapper passing all W3C R2RML test cases makes it the definitive correctness oracle for semantic-fabric's materialization mode. The test suite at [W3C](https://www.w3.org/TR/rdb2rdf-test-cases/) provides SQL scripts, mapping documents (`.ttl`), and expected N-Quads output — a natural test harness for a Rust implementation.

For semantic-fabric's RDBMS-only scope, the RML-IO and RML-LV modules are not immediately relevant; RML-Core and the original R2RML suite are the primary targets.

### 4.2 Architectural lessons from RMLMapper

The central lesson is what **not** to do at scale:

- Do **not** materialise full JDBC result sets in memory. Use cursor-based / streaming result set iteration (PostgreSQL `fetchSize`, MySQL streaming result set) to bound memory.
- Do **not** use in-memory set deduplication over the full output KG. Instead, use sort-merge or hash-join with bounded windows, or make deduplication optional (with explicit cost).
- For join conditions: pre-compute join indexes or use hash-join rather than nested loop. The KROWN timeout at 5+ conditions reveals O(n×m) behaviour.
- Named graph support must be correct from the start — both reference implementations fail or timeout on complex named graph scenarios.

### 4.3 Architectural lessons from RMLStreamer

RMLStreamer's Flink model maps naturally onto a Rust async/parallel execution model:

- **TriplesMap-level parallelism**: Each `TriplesMap` is an independent unit of work; semantic-fabric can process them in parallel using Tokio tasks or Rayon.
- **Streaming JDBC reads**: Issue one JDBC query per TriplesMap, stream rows through the mapping pipeline, emit triples to a shared sink.
- **Operator chain**: `query_executor → row_mapper → subject_builder → predicate_object_builder → triple_emitter`.
- **Configurable deduplication**: Implement as an optional post-processing stage (hash set with memory limit + spill to disk), not always-on.
- RMLStreamer's benchmark harness (Rust data streamer) confirms the ecosystem already expects Rust-level tooling at the data ingestion layer.

### 4.4 What to reimplement vs. reuse

| Component | Recommendation |
|-----------|----------------|
| W3C R2RML test harness | Reuse directly — SQL scripts + expected N-Quads are format-agnostic |
| KROWN benchmark suite | Reuse as scaling benchmark; the Docker framework is language-agnostic |
| GTFS-Madrid-Bench | Reuse for virtualization/SPARQL→SQL benchmarking |
| RMLMapper conformance logic | Reimplement in Rust; use as oracle only (run RMLMapper on same inputs, diff output) |
| Flink operator pattern | Reimplement as Tokio tasks / async pipeline; no JVM dependency needed |
| FnO function evaluation | Out of scope initially; design IR to be FnO-extensible |
| Jelly serialization | Consider via `jelly-ttl` Rust crate if available; N-Quads first |

---

## 5. SOTA benchmark numbers to target

From published results:

- **R2RML conformance**: 100% pass on W3C test suite (target: match RMLMapper)
- **RML-Core**: 89.83% (53/59) as of KGCW 2025 — target: 100% for RDBMS-relevant subset
- **RMLStreamer throughput**: ~70,000 records/s sustained at ISWC 2022 (SISO streaming mode)
- **KROWN**: No published per-tool throughput numbers in triples/s; key axes are execution time, memory, and timeout/OOM thresholds — all of which the current JVM tools fail before 10M rows

For semantic-fabric, the goal is to process 10M+ rows without OOM, support all join conditions without timeout, handle named graphs correctly, and produce conformant (deduplicated) output — a combination no current open-source JVM tool achieves.

---

## 6. Open questions

1. Is RMLStreamer v2.5.0 (June 2023) the last planned release, or is active development paused? The benchmark mismatch with current Flink versions (>1.14) may be blocking updates.
2. RMLMapper v7+ translates R2RML to RMLkgc Core internally. Is this translation lossless for all 60+ W3C test cases, or are edge cases silently dropped?
3. KROWN reports OOM for RMLMapper on large cell sizes — is this in the JDBC result set fetch or the deduplication phase? The answer determines which optimisation dominates for semantic-fabric.
4. The rmlstreamer-benchmark-rust harness is not results-published. Is there a newer KROWN-based Rust benchmark that covers materialization throughput in triples/s?

---

## Sources

- [RMLMapper GitHub (rmlmapper-java)](https://github.com/RMLio/rmlmapper-java) — v8.1.0, December 2025
- [RMLStreamer GitHub](https://github.com/RMLio/RMLStreamer) — v2.5.0, June 2023
- [RMLStreamer benchmark (Rust data streamer)](https://github.com/s-minoo/rmlstreamer-benchmark-rust)
- [W3C R2RML and Direct Mapping Test Cases](https://www.w3.org/TR/rdb2rdf-test-cases/)
- [W3C R2RML specification](https://www.w3.org/TR/r2rml/)
- [RML test cases (rml.io)](https://rml.io/test-cases/)
- [KGCW Challenge 2025 — RMLMapper + RML-view-to-CSV](https://ceur-ws.org/Vol-3999/short2.pdf)
- [Backwards or Forwards? R2RML Backwards Compatibility in RMLMapper (KGCW 2024)](https://openreview.net/forum?id=m6yJtJvu6y)
- [KROWN benchmark (GitHub)](https://github.com/kg-construct/KROWN)
- [KROWN results on Zenodo](https://zenodo.org/records/10973892)
- [KROWN: A Benchmark for RDF Graph Materialisation (Springer)](https://link.springer.com/chapter/10.1007/978-3-031-77847-6_2)
- [RMLStreamer-SISO: An RDF Stream Generator (ISWC 2022)](https://iswc2022.semanticweb.org/wp-content/uploads/2022/11/978-3-031-19433-7_40.pdf)
- [RMLStreamer-SISO arXiv preprint](https://arxiv.org/pdf/2210.14599)
- [RMLStreamer with Reference Conditions in the KGCW Challenge 2023](https://ceur-ws.org/Vol-3471/paper11.pdf)
- [Scaling Up Knowledge Graph Creation to Large and Heterogeneous Data Sources (arXiv 2022)](https://arxiv.org/abs/2201.09694)
- [Knowledge Graph Construction with R2RML and RML: An ETL System-based Overview (KGCW 2021)](https://ceur-ws.org/Vol-2873/paper11.pdf)
