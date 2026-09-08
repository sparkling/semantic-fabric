---
status: accepted
date: 2026-06-27
updated: 2026-09-08
tags: [provenance, lineage, prov-o, query-time, rdf-1.2, source-mapping]
supersedes: []
depends-on:
  - ADR-0003
implements:
  - ADR-0001
---

# Provenance & lineage — query-time

> **Implementation status (2026-09-08): accepted, partially implemented.**
> The public opt-in constant-mapping/source SELECT and CONSTRUCT profiles emit
> per-solution PROV-O and native graph reification under the pinned request.
> Full ADR-0017 remains open: dynamic multi-origin operators, federation,
> declared/verified row-key authority and wider native-profile/release qualification.
> ADR-0055, not historical ADR-0038, controls v1.

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

### Implemented incremental profile: constant-mapping-source-v1

Send exactly `Accept: application/vnd.semantic-fabric.lineage+json-seq` on the
existing authenticated GET/form/raw POST `/sparql` path. No configuration switch,
extra identity header, persistence or source re-query is involved. Ordinary
SPARQL result media types remain unchanged.

Eligibility is proved before source-generation acquisition: one authored
TriplesMap (nonempty ID, at most 1,024 bytes), no referencing object maps, and a
SELECT or CONSTRUCT without dataset clauses over nonempty BGPs with only JOIN,
UNION, projection, DISTINCT/REDUCED and slicing. The eligibility walk caps combined
algebra/triple visits and CONSTRUCT template triples at 256; this does not bound
parsing, nested term recursion or total compiler CPU.
All positive witnesses necessarily use the same authored map, including its
saturated class/predicate variants. Thus final-row annotation remains exact
after projection, bag UNION, duplicate elimination and LIMIT. A candidate-map
list is never substituted for actual origins. Ambiguous/multiple maps, OPTIONAL,
GROUP, FILTER/expressions, paths, ASK/DESCRIBE and federation currently reject
provenance with `501` before source I/O; ordinary-query support is unaffected.

The response is an RS/LF-framed JSON text sequence. Its first `header` record
contains projected variables, mapping ID, snapshot-local source index, opaque
snapshot and policy IDs, and a logical compiler-input fingerprint. The snapshot
ID includes an opaque generation-instance nonce, not a database URL or address;
it differs even for content-identical independent servers/generations. Policy
identity exposes neither raw identity nor a per-subject cache partition. The
logical-plan fingerprint is not a physical SQL/release/source-content attestation.
Each `solution` record contains its post-modifier ordinal, a standard SPARQL JSON
single-row `result`, and a JSON-LD `prov:Bundle` with a generated entity/activity,
the used mapping document/source, and snapshot/plan/policy references. Bundles
contain no source values, inferred row keys or `prov:wasDerivedFrom` row claim.
An empty result has no activity. Only successful execution and cleanup produce
the final `complete` record and solution count. Consumers must require both
that record and clean transport completion; a failed stream may expose a prefix.

Metadata is bounded independently of source size; eligibility and fixed metadata
charge the existing request budget, every serialized byte is charged, and the
existing backpressured terminal-body/cancellation/connection-ownership path is
retained. These are not a total-heap or total-compiler governance claim.

### Graph profile: constant-mapping-source-graph-v1

The same explicit Accept selects graph lineage for admitted CONSTRUCT queries.
The header retains the mapping/source/snapshot/logical-plan/policy identities and
adds `profile: constant-mapping-source-graph-v1`, `blankNodeScope: response`,
`productGraph: default`, and `datasetFormat: application/n-quads;version=1.2`.
Each nonempty emitted template solution has a `graph-solution` record with its
visible `ordinal` and a `dataset` string: a native RDF 1.2 N-Quads fragment.
Concatenate these fragments in response order and parse as **one RDF dataset**
with one blank-node scope, not separate RDF documents. Template blank nodes stay
shared within each solution and fresh between solutions; mapped blank nodes may
intentionally recur across records. No blank nodes are reminted by provenance.

Only unchanged emitted product triples occupy the default graph. All PROV-O
metadata, including bundle typing, occupies per-solution named bundle graphs.
Each emitted triple occurrence has a request/ordinal/index reifier IRI whose
`rdf:reifies` object is the native triple term and whose `prov:wasGeneratedBy`
links its solution activity. This repeats only authorized, already-emitted RDF
terms, including nested terms and language/direction; no unselected columns,
inferred row keys or raw identities are exposed. Metadata is not a product graph
and must remain outside SHACL validation of that graph.

Empty templates and invalid/unbound template outputs create no activity or
ordinal. Repeated product triples retain RDF graph-set semantics; no additional
whole-result deduplication state is introduced. Final `complete` reports
nonempty `solutions` and `tripleOccurrences`, **not a unique graph cardinality**.
As for SELECT, it follows successful execution and cleanup and requires clean
transport completion. Native RDF escaping precedes direct JSON-string escaping
through the charged writer; no second whole-dataset buffer or source query is
created. The existing result, deadline, backpressure and cleanup controls remain.

Required evidence: `cargo test --locked -p sf-serve --test lineage`,
`cargo test --locked -p sf-serve --lib lineage`, and
`cargo test --locked -p sf-core --test security_context_contract`. Public SQLite
tests cover bags, saturation, modifiers, no-PK promotion, row-policy isolation,
cache/generation identity, pinned reload, rejection and exact byte limits;
terminal tests cover deadline and source/cleanup failure.
Graph tests additionally compare the ordinary product graph, parse native
reification, preserve mapped/template blank nodes and nested/directional terms,
check empty/invalid/duplicate outputs and template admission, and verify exact
response bytes and cleanup failure without a successful completion record.

Required native evidence (2026-09-08):
`cargo test --locked -p sf-cli --no-default-features --test source_tls_live -- --ignored --exact authenticated_public_queries_require_verified_source_tls`.
This existing CI gate owns digest-pinned PostgreSQL 16.15/MySQL 8.4.11 fixtures and
now exercises the serving-only CLI's lineage media type and complete transport,
exact SELECT/CONSTRUCT results, parsed PROV-O/native reification, empty results,
bag UNION, cache/snapshot/policy identities and missing/invalid authentication.
A two-subject portable equality-row registry alternates allowed/empty callers on
each native backend and proves results and metadata do not expose the other
caller. Unsupported ASK/FILTER lineage rejects while a native table-lock witness
remains held, then ordinary admitted lineage recovers. Encrypted sessions are
observed on both providers. This qualifies those native cases, not every operator
combination, source-RLS lineage, lineage-specific reload/cancellation, production
admission or an exact packed release artifact. Shared lifecycle checks remain
required; no successful ordinary-query test is renamed as lineage-specific proof.

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
current truth; the incremental evidence above does not complete this ADR or
establish W3C conformance, backend admission or release qualification.

## More Information
* **Architecture / rewriter:** ADR-0003, ADR-0007. **Reasoning (provenance must survive saturation):** ADR-0008. **Security (sensitivity composes with provenance tags):** ADR-0018. **RDF 1.2 reification:** ADR-0019.
* **Cross-project:** the platform's provenance / source-mapping concerns. **Research:** `docs/research/provenance-security`.
