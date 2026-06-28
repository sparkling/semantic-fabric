---
status: accepted
date: 2026-06-27
tags: [architecture, virtualizer, obda, mapping-ir, sparql-to-sql, oxigraph, pipeline]
supersedes: []
depends-on:
  - ADR-0001
  - ADR-0002
implements:
  - ADR-0001
---

# Architecture — the virtualiser pipeline (SPARQL 1.2 → SQL over R2RML)

## Context and Problem Statement

ADR-0001 / ADR-0002 fix semantic-fabric as a **virtualisation-only OBDA engine** over relational sources. This ADR fixes its internal architecture: how a SPARQL 1.2 query becomes SQL against the live source, what is loaded once (`⟨T, M⟩`), and the module boundaries (ADR-0006 maps these to crates).

The engine holds two small **intensional** graphs in memory — the **ontology / T-Box (T)** and the **mapping graph (M, R2RML as RDF)** — and answers queries by **unfolding** the SPARQL algebra against the mappings into SQL. Instance data (the A-Box) is never instantiated; it is produced on demand as bindings/triples, streamed, and discarded.

## Considered Options

* **Single virtualiser pipeline over a shared core** — one model, one binary; SPARQL is unfolded against the mappings into SQL, with parse, term-generation, datatypes, and the source layer existing exactly once.
* **A separate materialisation mode / `materialize` verb** — rejected: ADR-0001 / ADR-0002 fix semantic-fabric as virtualisation-only, and a one-off RDF dump is just `CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }` streamed through the same pipeline — a query, not a second mode.

## Decision Outcome

A single **virtualiser** pipeline over a shared core:

```
SPARQL 1.2 query
  → parse + algebra (spargebra)
  → unfold against M (mapping IR) + saturate with T (hierarchy)      [ADR-0008]
  → optimise (ISWC-2018 base translation + order-disciplined cascade) [ADR-0007]
  → emit dialect SQL → execute on the live source (cursor-streamed)   [ADR-0006]
  → generate RDF 1.2 terms (R2RML §10 datatypes)                      [ADR-0015]
  → stream results: SPARQL 1.2 Results (SELECT/ASK) | JSON-LD (CONSTRUCT/DESCRIBE)
```

### Loaded once — the OBDA `⟨T, M⟩` (intensional, in memory)

- **T — ontology / T-Box** — held in Oxigraph (ADR-0004); the rewriter reads its hierarchy to saturate queries (subClassOf / subPropertyOf / inverse / symmetric; ADR-0008).
- **M — mapping graph** — R2RML parsed once into the **mapping IR** (`sf-mapping`), the single source of truth for what triples each source row produces.

### The shared core

1. **Mapping IR** — R2RML (+ Direct Mapping as auto-generated R2RML) parsed (Turtle, via `oxttl`) into one typed model. R2RML-only — no RML-readiness (ADR-0002).
2. **Term generation** — given a row + a term map, produce an `oxrdf` (RDF 1.2) term applying the R2RML §10 SQL→XSD datatype mapping (ADR-0015).
3. **Source / SQL layer** — connection management, dialect SQL emission, schema introspection (PK/FK/uniqueness for the optimiser), cursor-streamed result iteration, and cross-source semi-join planning (ADR-0006).
4. **RDF / results I/O** — `oxrdf` terms end-to-end; streaming SPARQL Results + JSON-LD serialisers (ADR-0019).

A one-off RDF dump, where ever needed, is `CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }` streamed through this same pipeline — a query, not a second mode.

### Consequences

* Good, because one model, one binary, one operational + conformance surface; parse, term-gen, datatypes, and the source layer exist exactly once.
* Good, because datatype / term semantics cannot drift (a single term-generation path).
* Bad, because the mapping IR must be walkable as a rewrite target; the hardest correctness surface is NULL / datatype semantics across SPARQL `OPTIONAL` → SQL `LEFT JOIN` (ADR-0007; Chebotko et al. as the proof target).

### Confirmation

* `cargo tree` shows the core crates (`sf-core` / `sf-sql` / `sf-mapping`) with no dependency on the virtualiser frontend; one mapping-IR type, one parser.
* `sf-cli` exposes `serve` (+ `conformance` / `bench`) and **no `materialize` verb**.

## More Information

* **Charter / scope:** ADR-0001, ADR-0002. **Substrate:** ADR-0004. **Crate layout + execution:** ADR-0006. **Rewriting + cascade correctness:** ADR-0007. **Reasoning (T-saturation):** ADR-0008. **Datatype/dialect:** ADR-0015.
* **Foundation:** the OBDA unfolding model — Ontop; Xiao & Kontchakov (ISWC-2018) — `docs/research/ontop.md`, `docs/research/foundations-benchmarks.md`.

## Rules

### R1 — One mapping IR, one parser
R2RML / Direct-Mapping is parsed into the `sf-core` IR exactly once (`sf-mapping`); nothing re-parses mapping documents.

### R2 — `oxrdf` terms end-to-end
RDF terms are `oxrdf` types (RDF 1.2; ADR-0004 / ADR-0019) throughout. No bespoke term type.

### R3 — Datatype mapping is shared, not duplicated
The R2RML §10 SQL→XSD datatype mapping lives once in `sf-core` term generation (ADR-0015).
