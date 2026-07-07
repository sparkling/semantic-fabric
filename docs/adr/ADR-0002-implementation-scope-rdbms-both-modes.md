---
status: accepted
date: 2026-06-27
tags: [scope, virtualization, obda, rdbms, r2rml, direct-mapping, sparql-1.2, rdf-1.2, conformance]
depends-on:
  - ADR-0001
implements:
  - ADR-0001
---

# Scope — virtualisation-only OBDA over relational databases via R2RML

## Context and Problem Statement

semantic-fabric answers **SPARQL over the ontology by rewriting it to SQL against live relational databases** — Ontology-Based Data Access (OBDA). It is a query engine, not an ETL tool: source rows are never copied into RDF. The mappings it executes are **R2RML** (with W3C **Direct Mapping** as an auto-generated R2RML path), authored upstream as the source-mapping layer in the upstream modelling project; semantic-fabric is their executor, not their author.

Two small RDF graphs are loaded at startup and held in memory — the OBDA `⟨T, M⟩`:

- **T — the ontology / T-Box** (classes, properties, hierarchy, annotations).
- **M — the mapping graph** (R2RML as RDF).

The unbounded **instance data (A-Box)** is never materialised: it is produced on demand by the rewriter, streamed to the client, and discarded.

## Considered Options

- **Virtualisation-only OBDA over relational databases via R2RML** (chosen) — SPARQL rewritten to SQL at query time; no instance triple store, no batch materialisation.
- **Materialisation / batch ETL into a persistent instance triple store** — excluded by design.
- **Non-relational / heterogeneous sources** (CSV/JSON/Parquet/XML, Façade-X / code-walked / streaming) via a file-reader layer — excluded by design.
- **The RML mapping family** (RML, FNML, RML-CC, YARRRML) as the mapping language — excluded; the IR models exactly what R2RML needs.
- **SPARQL `SERVICE` federation to external endpoints** — excluded (cross-*RDBMS* federation via semi-join reduction is in scope per ADR-0006; remote-endpoint federation is not).

## Decision Outcome

### In scope (v1)

| Area | Decision |
|---|---|
| Mode | **Virtualisation / OBDA only** — SPARQL → SQL at query time. No instance triple store, no batch materialisation. |
| Mappings | **R2RML** + **Direct Mapping** (treated as auto-generated R2RML). Nothing else. |
| Sources | **Relational databases only** — PostgreSQL (primary), SQLite (CI/embedded), MySQL (follows). |
| Query | **Full SPARQL 1.2** — BGPs, OPTIONAL, UNION, MINUS, FILTER + (NOT) EXISTS, BIND, VALUES, subqueries, aggregation (GROUP BY/HAVING), solution modifiers, **recursive property paths (`*`/`+`)**, and all four query forms (SELECT/ASK/CONSTRUCT/DESCRIBE). |
| Data model | **RDF 1.2** — triple terms, reifiers, directional language-tagged strings. |
| Results | SELECT/ASK → **SPARQL 1.2 Results, JSON**; CONSTRUCT/DESCRIBE → **streamed JSON-LD** (expanded/flattened, emitted incrementally — never framed/compacted). |
| Reasoning | Entailment folded into the rewrite (ADR-0008). No materialised closure. |

### Out of scope (excluded by design, not "deferred")

- Materialisation of instance data, and any persistent instance triple store.
- Non-relational sources (CSV/JSON/Parquet/XML), Façade-X / code-walked / streaming sources, and any file-reader layer.
- RML, FNML, RML-CC, YARRRML parsing, and any "RML-ready" extensibility in the mapping IR — the IR models exactly what R2RML needs.
- SPARQL `SERVICE` to external endpoints. (Cross-*RDBMS* federation via semi-join reduction **is** in scope — ADR-0006; remote-endpoint federation is not.)

A one-off RDF dump, where ever needed, is `CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }` streamed through the same rewriter — a query, not a second mode.

### Targets & correctness gates

- **Correctness** — the **W3C RDB2RDF test cases** (Direct Mapping + R2RML), executed as CONSTRUCT through the rewriter and compared to the expected output; the **Ontop** offline differential oracle on a shared R2RML set (ADR-0005); NoREC internal-differential + MR1 metamorphic (ADR-0012).
- **Performance** — the **GTFS-Madrid-Bench OBDA track**: query latency, first-result latency, and **constant engine memory under growing source data** (ADR-0005). Materialisation benchmarks (KROWN) do not apply.
- The engine is exercised only against these standard suites/benchmarks and Ontop — **never against production mappings**, which remain runtime build output (the upstream source-mapping decision).

### Consequences

- Good, because one engine, one evaluation strategy. The source database does all set-work and spills natively; engine memory is bounded by `⟨T, M⟩` plus a fixed streaming budget, independent of source size (ADR-0006, ADR-0010).
- Good, because R2RML + relational only removes the heterogeneous source layer and all speculative RML-readiness.
- Good, because RDF 1.2 / SPARQL 1.2 is a current-standard differentiator over the SPARQL-1.1 incumbents.
- Bad, because a full SPARQL 1.2 rewriter is the up-front cost; there is no smaller materialiser baseline to ship first. This is the engine, built whole.

### Confirmation

- `cargo build --workspace` succeeds; `sf-cli` exposes `serve` (+ `conformance`/`bench`) and **no `materialize` verb**.
- `sf-conformance` drives the W3C RDB2RDF suite via CONSTRUCT and the Ontop oracle (red until engine logic lands).
- No crate depends on a triple store for instance data; Oxigraph holds only `⟨T, M⟩` (ADR-0004).

## More Information

- **Charter:** ADR-0001. **Architecture:** ADR-0003. **Execution/crates:** ADR-0006. **Rewriting:** ADR-0007. **Reasoning:** ADR-0008. **Conformance/bench:** ADR-0005.
- **Cross-project:** the R2RML mappings executed here are authored in the upstream modelling project (source mapping; the upstream mapping-conformance requirements / source-mapping decision). semantic-fabric is their executor.
