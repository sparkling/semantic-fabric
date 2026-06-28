---
status: accepted
date: 2026-06-27
tags: [substrate, oxigraph, oxrdf, spargebra, oxjsonld, rust, rdf-1.2, sparql-1.2, dependencies, licensing, intensional-graphs]
supersedes: []
depends-on:
  - ADR-0003
implements:
  - ADR-0001
---

# Oxigraph crates as the RDF/SPARQL substrate — reuse the plumbing, own the rewriter, hold ⟨T, M⟩ in memory

## Context and Problem Statement

The virtualiser (ADR-0003) needs an RDF 1.2 term system, Turtle/JSON-LD I/O, a SPARQL 1.2 parser + algebra, and a place to hold the small intensional graphs `⟨T, M⟩` (ontology + mapping graph) in memory. Reimplementing these against the mature, dual-licensed (MIT OR Apache-2.0) Oxigraph crate family is wasted effort. The questions: which crates, and how much.

`spargebra` (parser → algebra) and `sparopt` (optimizer) carry zero dependency on any evaluator or store, so we consume their AST and feed our own SQL-rewriting engine (ADR-0007). Instance data is never stored (ADR-0002), so we take no persistent triplestore.

## Decision

**Reuse the Oxigraph sub-crates as substrate; own the SQL-rewriting evaluator; hold `⟨T, M⟩` as small in-memory graphs.**

### Depend on (pinned to the oxigraph 0.5.x train; 1.2 feature flags per ADR-0019)

| Crate | Pin | Features | Role |
|---|---|---|---|
| `oxrdf` | 0.3 | `rdf-12`, `rdfc-10` | RDF 1.2 terms/triples/quads — the type currency (ADR-0003 R2) |
| `oxttl` | 0.2 | `rdf-12` (+`async-tokio`) | Turtle/N-Triples/TriG/N-Quads 1.2 parse + serialize — mapping/ontology ingest + N-Triples output |
| `oxjsonld` | 0.2 (≥ 0.2.5) | `rdf-12` | Streaming JSON-LD serialize — CONSTRUCT/DESCRIBE output (ADR-0019) |
| `spargebra` | 0.4 | `sparql-12`, `sep-0002`, `sep-0006` | SPARQL 1.2 parser → algebra (rewriter entry; `ADJUST` needs `sep-0002`; `sep-0006` enables the LATERAL extension) |
| `sparopt` | 0.3 | — | SPARQL algebra optimizer — opt-in pre-rewrite stage |
| `sparesults` | 0.3 | `sparql-12` (+`async-tokio`) | SPARQL 1.2 Results (JSON/XML/CSV/TSV) — SELECT/ASK responses |
| `oxsdatatypes` | 0.2 | — | XSD datatype canonicalization (ADR-0015) + residual FILTER arithmetic |

Enable `standard-unicode-escaping` (strict 1.2 `\u`) where wanted. **`sep-0006` (LATERAL) is enabled** as a documented opt-in extension, kept out of the 1.2 conformance surface (ADR-0007 / ADR-0019).

> **Reconciliation note (2026-06-28 — corrected; supersedes an earlier same-day claim).** An earlier version of this note said `sparopt` 0.3.6 "does not compile against `spargebra` with `sparql-12`/`sep-0006`." That is **empirically false** (verified via `cargo build -p sparopt` + `cargo tree`): `sparopt` 0.3.6 compiles with `spargebra` 0.4.6 + `sparql-12`/`sep-0002`/`sep-0006` and is a live transitive dependency here (via the `spareval` oracle → `oxigraph`/`rudof` → `sf-conformance`). Corrected status: `sparopt` is **not wired into the engine optimizer by choice** — the ADR-0007 cascade is the sole optimiser (no loss; the pre-rewrite stage is opt-in). The dead `[workspace.dependencies]` `sparopt` line was removed as hygiene (it was unreferenced; `sparopt` still resolves transitively). Companion notes: ADR-0007 §pipeline step 2, ADR-0019 §config matrix.

### Hold `⟨T, M⟩` in memory (this absorbs the former serving-store decision)

- **T (ontology / T-Box)** and **M (mapping graph)** load once into small in-memory `oxrdf` graphs. The T-Box class/property hierarchy is indexed in-process for query saturation (ADR-0008); the mapping graph parses into the mapping IR (ADR-0003).
- **No persistent triplestore and no `librocksdb-sys` on the engine path.** Instance data is virtualised, never stored — so the `oxigraph` store crate (RocksDB) is not a dependency. This replaces the host platform's Jena Fuseki/TDB2 store for serving the ontology: there is no materialised instance graph to serve; the intensional graphs live in process.
- If SPARQL over the intensional graphs is needed (source-mapping governance / lineage / impact queries over M, or ad-hoc T queries), serve it with an **in-memory evaluator** (`spareval` over an in-memory dataset) — the intensional graphs are tiny. This is the *only* place `spareval` may appear, and never on the OBDA hot path.

### Explicitly do NOT depend on

- **`spareval` on the OBDA path** — its triple-at-a-time pull model is the wrong execution model for SQL-rewriting OBDA (we push the whole query into the DB as one SQL statement; ADR-0007). Permitted only for the in-memory `⟨T, M⟩` queries above.
- **`oxigraph`** (the store crate) — bundles RocksDB / `librocksdb-sys`, for which the virtualiser has no use.

**Versioning:** pin exact patch versions (the 1.2 specs are pre-final — ADR-0019); the sub-crates follow the oxigraph release train in lockstep; track the changelog before bumping.

### Consequences

* Good — first-class RDF 1.2 / SPARQL 1.2 plumbing for free; we build only the novel part (the SQL rewriter); permissive licensing; no JVM, no RocksDB, no `librocksdb-sys` on the engine path.
* Good — `⟨T, M⟩` in memory keeps the engine store-free while still allowing SPARQL over the intensional graphs via an in-memory evaluator.
* Bad — 0.x semver: breaking changes between minors, mitigated by exact pins + lockstep releases.

### Confirmation

* `Cargo.toml` depends on the sub-crates above with the 1.2 feature flags set, and **not** on the `oxigraph` store crate; `cargo tree` shows **no `librocksdb-sys`**.
* A smoke test parses a SPARQL 1.2 query (triple-term pattern) via `spargebra` and round-trips an RDF 1.2 Turtle document (triple terms, dir-lang strings) through `oxttl`.

## More Information

* **Architecture:** ADR-0003 (R2 — `oxrdf` terms end-to-end). **Rewriter:** ADR-0007. **Crate layout:** ADR-0006. **1.2 feature flags / Jena replacement:** ADR-0019. **Datatypes:** ADR-0015.
* **Survey:** `docs/research/oxigraph.md`, `docs/research/rust-substrate.md`.
</content>
