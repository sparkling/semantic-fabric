---
status: accepted
date: 2026-06-27
tags: [data-fabric, obda, virtualization, r2rml, rdbms, rust, no-jvm, sparql-1.2, rdf-1.2, source-mapping, sota, charter]
supersedes: []
depends-on: []
implements: []
---

# semantic-fabric — a custom Rust OBDA data fabric (virtualisation over relational sources)

## Context and Problem Statement

The ontology is populated from relational source systems via **source mapping** (R2RML) — the integration layer authored in the upstream modelling project, governed by the upstream mapping-conformance requirements and the upstream source-mapping decision. Serving the ontology over live data is **runtime-critical**: queries answer SPARQL over the ontology by rewriting to SQL against the live source at query time — the source-mapping `M ⋈ T` gate (the upstream mapping-conformance requirements) means a mapping-vs-ontology drift serves a wrong answer live.

The host platform serves this today on **Apache Jena (Fuseki + jena-shacl)** plus off-the-shelf OBDA tooling (Ontop) — a **JVM** stack, owned by several upstreams with independent roadmaps, sitting on the runtime-critical answer-serving path. The question: keep assembling off-the-shelf JVM tools, or build **one Rust engine we own** — native to RDF 1.2 / SPARQL 1.2, with no JVM on the runtime path.

## Decision

Build **semantic-fabric**: a custom **Rust Ontology-Based Data Access (OBDA) engine** that answers **SPARQL 1.2 over the ontology by rewriting to SQL** against live **relational** sources, executing **R2RML** mappings. It is a **virtualiser** — source data is never materialised into RDF (ADR-0002). The ontology and the mappings (`⟨T, M⟩`) are loaded as the engine's intensional inputs; instance data stays in the source and is produced on demand, streamed, and discarded.

### Why build it

* **No JVM on the runtime-critical path** — a single static Rust binary; native speed, memory safety, nothing to operate but the binary. Replaces Jena (Fuseki + jena-shacl) and Ontop end-to-end with Rust (Oxigraph crates + rudof; ADR-0004, ADR-0005, ADR-0019).
* **RDF 1.2 / SPARQL 1.2 native** — the ontology is built on RDF 1.2 (triple terms, `rdf:reifies`, directional strings) and SPARQL 1.2; we target these directly (ADR-0019), ahead of the SPARQL-1.1 incumbents.
* **Own the runtime-critical path** — the fabric is load-bearing (the upstream mapping-conformance requirements); owning it removes a multi-upstream dependency from the answer-serving path and lets the source-mapping `M ⋈ T` gate run natively.
* **Enterprise-grade, SOTA, secure, performant** — streaming, bounded-memory execution over live sources (ADR-0006, ADR-0010); the source database does the set-work and spills natively.
* **Stand on proven designs** — the Xiao/Kontchakov (ISWC-2018) SPARQL→SQL translation, Ontop's OBDA shape (as offline oracle, ADR-0005), and the W3C RDB2RDF conformance suite.

### Boundary — design-phase / data

The engine is developed and conformance-tested against **standard suites and benchmarks** (W3C RDB2RDF via CONSTRUCT, GTFS-Madrid-Bench OBDA track, Ontop differential oracle; ADR-0005) — **never against production mappings**, which remain runtime build output (the upstream source-mapping decision). Building the engine authors no production mapping.

This **supersedes the *executor* choice** in the upstream mapping-conformance requirements / source-mapping decision only: the mappings and standards (R2RML) are unchanged — only *what runs them* changes from off-the-shelf JVM tools to semantic-fabric. (Cross-project; the matching amendment lands there.)

### Consequences

* Good — one owned, single-binary Rust engine on the runtime-critical path; no JVM; RDF 1.2 / SPARQL 1.2 native; the `M ⋈ T` gate runs natively.
* Good — fits the greenfield capability-altitude posture from the upstream design corpus: architect the target, don't patch a pipeline.
* Bad — large build effort: a full SPARQL 1.2 → SQL OBDA rewriter is a major undertaking, and we take on the conformance, SQL-dialect, and perf-regression maintenance the upstreams carry today.
* Neutral — design-phase: a charter + decision, not running code.

### Confirmation

* The engine answers SPARQL 1.2 over R2RML-mapped relational sources, cross-checked against **Ontop** on a shared R2RML set and the **W3C RDB2RDF** suite (run via CONSTRUCT); the source-mapping `M ⋈ T` gate runs against its output.
* No JVM process is present on the runtime path.

## More Information

* **Scope:** ADR-0002. **Architecture:** ADR-0003. **Substrate (Oxigraph crates):** ADR-0004. **Conformance/bench:** ADR-0005. **Execution/crates:** ADR-0006. **Rewriting:** ADR-0007. **Reasoning:** ADR-0008. **1.2 readiness / Jena replacement:** ADR-0019.
* **Cross-project (executor only):** the upstream mapping-conformance requirements / source-mapping decision (the source-mapping toolchain). Greenfield posture: the upstream design corpus.
</content>
