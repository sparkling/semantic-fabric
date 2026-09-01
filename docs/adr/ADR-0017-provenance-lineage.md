---
status: accepted
date: 2026-06-27
updated: 2026-09-01
tags: [provenance, lineage, prov-o, query-time, rdf-1.2, source-mapping]
supersedes: []
depends-on:
  - ADR-0003
implements:
  - ADR-0001
---

# Provenance & lineage — query-time

> **Implementation status (2026-09-01): accepted, not implemented.** No
> production query path emits mapping/source/row-key lineage or PROV-O. Accepted
> ADR-0038 milestone M5 retains query-time, non-persisted provenance but binds it
> to immutable runtime snapshots, source identity, security policy, and request
> budgets before it is advertised.

## Context and Problem Statement

Operators need to know which mapping / source / row produced each result triple (source-mapping lineage; provenance). The engine virtualises — it stores nothing (ADR-0002) — so provenance is intrinsically **query-time**: recomputed from the rewrite, never persisted.

## Considered Options

* **Query-time recomputed provenance (nothing stored)** — recompute lineage from the rewrite (`⟨T, M⟩` + the query), materialised on demand only for returned rows.
* **Persisted/materialised provenance store** — rejected: storing provenance conflicts with virtualisation-only (the engine stores nothing, ADR-0002).
* **Whole-graph provenance materialisation** — rejected: per-triple bloat; provenance is needed only for the rows a query returns, never for the whole graph.

## Decision Outcome

> **Accepted-decision amendment (2026-09-01).** The original decision assumed
> that an observed source primary key could become row identity. Commit
> `24a0e20` makes mutable catalogue constraints non-authoritative in serving.
> This amendment retains query-time provenance but requires an explicitly
> declared lineage key or a verified execution-scoped authority for row keys.

### Query-time where/how-provenance
The rewriter already knows, per solution, which triples-map produced it and which source columns it read — that is how it built the SQL. Recompute mapping/source provenance from that and tag the mapping IRI. A `{mappingId, sourceId, row-key}` lineage may project a row key only when the runtime snapshot carries an explicitly declared lineage key or a verified constraint authority held through streamed execution. Current serving deliberately quarantines catalogue PK/UNIQUE facts, so it cannot silently promote an observed primary key to provenance identity. Where no authorized stable row key exists, expose mapping/source lineage without a row key or reject a requested row-key profile before execution. This answers the source-mapping question "which mapping/source produced this" plus coverage and impact, computed from `⟨T, M⟩` + the query, with **nothing stored**.

### Exposure
Provenance is materialised-on-demand only for the rows a query returns — never for the whole graph:
* graph results → **RDF 1.2 reifying triples** (`rdf:reifies` a triple term; oxrdf-native, ADR-0004);
* a per-solution **PROV-O** bundle (`prov:Activity` *used* the source + mapping document, `prov:wasDerivedFrom` the row) for provenance, FAIR-aligned.
Triple-level metadata rides RDF 1.2 reification; keep it out of any SHACL-validated graph (the RDF-star/SHACL interaction, ADR-0019).

### Consequences

* Good, because source-mapping lineage + provenance with zero stored state and no per-triple bloat; consistent with virtualisation-only; reuses RDF 1.2 reification natively.
* Bad, because provenance is recomputed per query (a cost on the response path); compute it only when requested.

### Confirmation

When implemented, the ADR-0012 strategy must verify that provenance is
recomputed from `⟨T, M⟩` + the query with nothing stored and materialised only for
returned rows (graph results as RDF 1.2 reifying triples, per-solution PROV-O
bundles). It must also prove that unverified PK/UNIQUE observations never create a
row key, while an explicitly authorized lineage key survives mapping, cache and
streamed-execution boundaries. The implementation-status note above remains the
current truth; no existing conformance or benchmark gate proves this feature.

## More Information
* **Architecture / rewriter:** ADR-0003, ADR-0007. **Reasoning (provenance must survive saturation):** ADR-0008. **Security (sensitivity composes with provenance tags):** ADR-0018. **RDF 1.2 reification:** ADR-0019.
* **Cross-project:** the platform's provenance / source-mapping concerns. **Research:** `docs/research/provenance-security`.
