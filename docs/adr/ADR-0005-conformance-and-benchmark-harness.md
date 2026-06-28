---
status: accepted
date: 2026-06-27
tags: [conformance, benchmarks, w3c, rdb2rdf, earl, gtfs-madrid, obda-oracle, ontop, m-join-t, shacl, fitness-function]
supersedes: []
depends-on:
  - ADR-0002
  - ADR-0003
implements:
  - ADR-0001
---

# Conformance & benchmark harness — the correctness gate and the fitness function

## Context and Problem Statement

ADR-0001 commits the engine to a standardised correctness gate and SOTA performance numbers while honouring the cross-repo `M ⋈ T` gate. This harness wires those into runnable form. It is doubly load-bearing: the **correctness gate** (no SOTA claim is admissible without a standardised conformance result) and the **fitness function** for the engine-perf Path-B loop (ADR-0013) — the conformance pass-rate is the non-degradation gate, and OBDA query / first-result latency + constant memory are the efficiency objectives.

## Decision Outcome

### Correctness gate — W3C RDB2RDF test cases (via CONSTRUCT)
Vendor the suite into `tests/w3c/rdb2rdf/` (~49–63 named cases across D000–D025, positive **and** error cases; the W3C document licence permits redistribution). Base IRI fixed at `http://example.com/base/`. The engine has no materialiser, so each case runs as a **`CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }` through the rewriter**, streaming the produced triples. Comparison is **graph isomorphism** (blank-node aware, via `oxrdf`) against the case's expected output (R2RML cases → N-Quads/Turtle; Direct Mapping cases → the auto-generated-R2RML path). Execute against embedded **SQLite** for fast per-push CI and **PostgreSQL** for the full run; **per-DBMS forked fixtures** capture dialect-specific expected output (ADR-0015). Emit `earl-semantic-fabric-{r2rml,direct}.ttl` (the first Rust entry in the implementation report).

### Performance benchmark — GTFS-Madrid-Bench (OBDA track)
The virtualiser is measured on the **GTFS-Madrid-Bench OBDA / query-rewriting track** (scale factors 1×–1000×): match or beat **Ontop** query latency, and — the differentiator — hold **constant engine memory and bounded first-result latency under growing source data** (the streaming invariant, ADR-0006 / ADR-0010). Materialisation benchmarks (KROWN) do not apply. Driven by `criterion`; results feed the Path-B objective (ADR-0013).

### Differential oracle — native in-memory (Oxigraph) + Ontop
Ground truth for an OBDA answer: load the case's **expected RDF graph into an in-memory store and evaluate the same SPARQL** (`spareval`, ADR-0004), diffed against the virtualiser's live-SQL answer. This tests rewriter correctness directly, keeps CI **zero-JVM**, and — since the in-memory evaluator handles property paths — validates `P+`/`P*`. **Ontop** is retained as an *optional, offline* cross-check on a shared R2RML set (and the tier-2 OWL-QL oracle, ADR-0008), never a CI dependency.

### Cross-project `M ⋈ T` gate
Evaluate the upstream modelling project's mapping-output validation (shapes) — `mf:MappingClassConformanceShape`, `mf:MappingPredicateConformanceShape`, `mf:MappingDatatypeConformanceShape`, `mf:EntitySubjectGroundingShape` (the upstream mapping-conformance requirements) — over the `M ⋈ T` closure for the virtualised path. **SHACL runner = rudof's `shacl` crate** (pin `shacl = "0.3"` + `oxrdf = "0.3"`), `ShaclValidationMode::Native` (pure Rust; its `sparql` feature is on by default, so Native is pinned explicitly — ADR-0019). Its in-memory graph is oxrdf-native (via `rudof_rdf`), so no second RDF stack enters the engine; the four shapes use only SHACL Core constraints (`sh:class`, `sh:datatype`, `sh:nodeKind`, `sh:property`, cardinality, `sh:in`/`sh:hasValue`), which `shacl` fully covers (engine rationale: `docs/research/shacl-engine-selection.md`).

## Consequences
* Good — objective, standardised SOTA measurement from day one; a real fitness function (pass-rate gate + OBDA latency/memory objectives) for the Path-B loop; the cross-project `M ⋈ T` obligation becomes executable, not prose.
* Bad — the vendored W3C suite needs a documented refresh discipline (a stable pinned snapshot).

## Confirmation
* `cargo test -p sf-conformance` drives the vendored W3C suite via CONSTRUCT (red until engine logic lands) and writes an EARL report.
* `cargo bench -p sf-bench` compiles the GTFS-Madrid OBDA-track driver.
* The `M ⋈ T` hook wires rudof `shacl` (Native) over the four shape IRIs.

## More Information
* **Scope:** ADR-0002. **Architecture:** ADR-0003. **Substrate (in-memory oracle):** ADR-0004. **Execution:** ADR-0006. **Datatype correctness + per-DBMS fixtures:** ADR-0015. **Inner test layers:** ADR-0012. **Reasoning oracle:** ADR-0008. **SHACL / 1.2:** ADR-0019.
* **Cross-project (authoritative):** the upstream mapping-conformance requirements; shape IRIs in `src/ontology/07-validation-constraints/validation-constraints-meta-shapes.ttl`.
* **Research:** `docs/research/` — `r2rml-spec-tests`, `foundations-benchmarks`, `shacl-engine-selection`.
</content>
