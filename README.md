# semantic-fabric

A **Rust-native, virtualisation-only OBDA engine**: it answers `SPARQL` 1.2 over a
virtual RDF graph by rewriting each query to `SQL` over a live relational source
through `R2RML` mappings. There is **no JVM** and **no copy of the data** — instance
data is **never materialised**; the engine reads it in place at query time.

- **Virtualisation / OBDA:** `SPARQL 1.2` over the ontology is rewritten to
  `SQL` over the source and executed directly against the database (ADR-0003,
  ADR-0007).
- **No materialisation:** the ⟨T, M⟩ pair (the ontology hierarchy `T` and the
  R2RML mappings `M`) is held in memory; only those, never the A-Box, ever live in
  the engine (ADR-0004).
- **Native Rust reasoning:** tier-1 entailment (class/property hierarchies,
  inverses, symmetry) is folded into the rewrite; transitive properties are served
  live as recursive SQL CTEs (ADR-0008).

## Status

The engine is **built**. The W3C RDB2RDF conformance suite and the GTFS-Madrid
OBDA benchmark run end-to-end over SQLite and PostgreSQL. Property paths `P+`/`P*`
(recursive CTEs), named-graph output, and the R2RML §10 datatype mapping are
implemented. The standalone SPARQL Protocol `serve` endpoint is the remaining
later-wave increment (ADR-0003/0007).

## Capabilities

- **SPARQL 1.2 → SQL rewriting** over R2RML and Direct Mapping (ADR-0007).
- **Two execution backends:** SQLite and PostgreSQL (`sf-sql`).
- **Property paths** `P+` / `P*` via recursive CTEs, with DoS limits (ADR-0010).
- **Named-graph output** and query-time provenance/lineage (ADR-0017).
- **W3C RDB2RDF conformance** via the real harness with EARL reporting (ADR-0005).
- **GTFS-Madrid-Bench** OBDA driver for query-rewriting performance (ADR-0005/0006).

## Scope (ADR-0002)

This engine delivers **virtualisation over relational databases** (R2RML + Direct
Mapping), developed and conformance-tested against the **W3C RDB2RDF** suite and the
**GTFS-Madrid-Bench / KROWN** benchmarks. Deferred: heterogeneous (CSV/JSON/XML)
execution, the standalone SPARQL Protocol endpoint, `SERVICE` federation, and FNML.

## Workspace (ADR-0006)

| Crate | Role |
|---|---|
| `sf-core` | Shared core: R2RML mapping IR, RDF term generation, R2RML §10 datatypes |
| `sf-sql` | Source/SQL layer: connectors, dialects (SQLite/PostgreSQL), schema introspection |
| `sf-mapping` | R2RML / Direct-Mapping parser → core IR |
| `sf-sparql` | The virtualiser: SPARQL 1.2 → SQL rewriter (instance data never materialised) |
| `sf-conformance` | W3C RDB2RDF harness + EARL + `M ⋈ T` SHACL gate |
| `sf-bench` | GTFS-Madrid OBDA benchmark driver |
| `sf-cli` | The single binary: `serve · conformance · bench` |

## Build

```bash
cargo build --workspace        # toolchain pinned via rust-toolchain.toml
cargo run -p sf-cli -- --help  # the `semantic-fabric` binary
cargo run -p sf-cli -- conformance   # run the W3C RDB2RDF suite (SQLite)
cargo run -p sf-cli -- bench         # run the GTFS-Madrid OBDA benchmark
```

## Decision records

| ADR | Decision |
|---|---|
| [ADR-0001](docs/adr/ADR-0001-semantic-fabric-rust-data-fabric.md) | A custom Rust OBDA data fabric — virtualisation over relational sources (charter) |
| [ADR-0002](docs/adr/ADR-0002-implementation-scope-rdbms-both-modes.md) | Scope: virtualisation-only OBDA over relational databases via R2RML |
| [ADR-0003](docs/adr/ADR-0003-shared-core-two-frontend-architecture.md) | Architecture: the virtualiser pipeline (SPARQL 1.2 → SQL over R2RML) |
| [ADR-0004](docs/adr/ADR-0004-oxigraph-rdf-sparql-substrate.md) | Oxigraph crates as the RDF/SPARQL substrate; own the rewriter, hold ⟨T, M⟩ in memory |
| [ADR-0005](docs/adr/ADR-0005-conformance-and-benchmark-harness.md) | W3C RDB2RDF + GTFS/KROWN harness — correctness gate and fitness function |
| [ADR-0006](docs/adr/ADR-0006-crate-layout-and-performance-model.md) | Crate layout, execution & performance model (source push-down + cross-source semi-join; no OLAP intermediary) |
| [ADR-0007](docs/adr/ADR-0007-sparql-to-sql-rewriting-strategy.md) | SPARQL→SQL rewriting + cascade correctness (ISWC-2018 base translation) |
| [ADR-0008](docs/adr/ADR-0008-reasoning-strategy.md) | Reasoning: entailment folded into the rewrite, native Rust, no runtime JVM |
| [ADR-0010](docs/adr/ADR-0010-security-and-resource-governance.md) | Security & resource governance (injection-safety, `P+` DoS limits) |
| [ADR-0011](docs/adr/ADR-0011-observability-and-configuration.md) | Observability (`tracing` + `metrics`/Prometheus) + configuration model |
| [ADR-0012](docs/adr/ADR-0012-test-strategy.md) | Test strategy (unit/integration/property/fuzz + snapshot) |
| [ADR-0013](docs/adr/ADR-0013-meta-harness-dev-loop.md) | Meta-harness dev loop (readiness drift + perf tuning) |
| [ADR-0014](docs/adr/ADR-0014-production-hardening-backlog.md) | Production-hardening backlog (acknowledged-deferred) |
| [ADR-0015](docs/adr/ADR-0015-datatype-dialect-correctness.md) | Datatype & dialect correctness — R2RML §10 canonicalization (SQLite affinity) |
| [ADR-0017](docs/adr/ADR-0017-provenance-lineage.md) | Provenance & lineage — query-time named graphs + PROV-O |
| [ADR-0018](docs/adr/ADR-0018-security-edge.md) | Security edge — source RLS + rewriter ABAC + data-sensitivity authZ |
| [ADR-0019](docs/adr/ADR-0019-rdf-sparql-shacl-12-readiness.md) | RDF 1.2 / SPARQL 1.2 / SHACL readiness — Rust stack in place of a JVM |
| [ADR-0020](docs/adr/ADR-0020-outstanding-sota-optimisations.md) | Outstanding SOTA optimisations — research register & dispositions |

Prior-art research grounding these decisions: [`docs/research/`](docs/research/) —
Ontop, Morph-KGC, SDM-RDFizer, RMLMapper/Streamer, Oxigraph, R2RML + W3C tests,
RML/YARRRML, foundations + benchmarks, Rust substrate; SHACL-engine selection,
virtualization streaming, OBDA governance, external-memory join, dialect
correctness, SPARQL→SQL cascade correctness, DuckDB-as-heterogeneous-reader,
OWL-QL tier-2, provenance + security.

## SOTA targets (ADR-0006)

100% W3C RDB2RDF · match/beat Ontop query latency at GTFS SF≥100 while holding
constant engine memory · complete the KROWN dimensions without OOM/timeout.
