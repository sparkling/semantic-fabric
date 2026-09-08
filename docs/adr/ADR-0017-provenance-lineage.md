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
> A bounded multiple-mapping positive-query profile also carries actual origins.
> The bounded two-source UNION and join carry source-keyed actual origins.
> Full ADR-0017 remains open: wider multi-origin operators and federation,
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
list is never substituted for actual origins. This constant-origin profile excludes
multiple maps, OPTIONAL, GROUP, FILTER/expressions, paths, ASK/DESCRIBE and
federation; the bounded profile below separately admits multiple maps. Other
shapes reject with `501` before source I/O; ordinary-query support is unaffected.

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

### Bounded multiple-mapping profiles (2026-09-08)

The same media type additionally admits `bounded-mapping-source-v1` SELECT and
`bounded-mapping-source-graph-v1` CONSTRUCT. Its `mappingCatalog` is an identifier
dictionary, **not** a list of contributing origins. Only actual emitted solutions
carry used mapping-entry/source identities, the mapping document and the same
snapshot/logical-plan/policy references. Row keys are still not provided.

The opt-in compiler keeps atom/mapping alternatives separate from ordinary query
optimization. It admits positive BGP/JOIN/UNION with a root-only projection,
DISTINCT/REDUCED/slice spine, constant non-`rdf:type` query predicates and constant
mapping predicates/graph maps; referencing maps, variable predicates, quoted WHERE
patterns, overlapping direct/inverse rewrites and nested operand modifiers reject
before source I/O. This is an additional physical execution profile, not a new
architecture or a claim that excluded shapes are complete. Ordinary query
optimization remains separate from the lineage recipe.

Each compiled atom reads through the existing owned native cursor and reconstructs
fresh private subject/object slots. Native RDF equality checks constants, repeated
variables and joins, avoiding SQL collation/coercion as provenance authority.
Missing mandatory terms cannot produce witnesses. Atom duplicates union their
origin bits; syntactic UNION choices preserve bag occurrences, including under
joins and projection. DISTINCT combines actual origins before slicing, so later
witnesses cannot disappear behind an early LIMIT. Graph output uses the unchanged
native template/reification path and preserves its response-wide blank-node scope.

Admission caps 64 distinct authored map IDs (1,024 bytes each), 256 prospective
atom alternatives/compiled branches, 128 algebra/triple visits, 256 projected
variables/template triples and 1,024 template-expansion units. Execution caps each
intermediate relation at 1,024 witnesses and charges source pulls, local join work,
retained containers/payloads and serialization to the same request budget. Borrowed
key lookup avoids full-key clones; candidate and template expansion are charged
before cloning. All origins are resolved in this bounded request-local buffer
before final records stream. An overflow fails; it never truncates or fabricates
complete provenance. This is not total compiler/native-row/heap governance.

There is no separate provenance re-query or persisted store. Multiple atom cursors
retain their existing backend isolation/transaction guarantees; an immutable
runtime-generation identifier does **not** assert a database-wide point-in-time
data snapshot. PostgreSQL generation/source-RLS paths retain their transaction
owners and fail-closed admission. Broader transactional/backend qualification is
not inferred from SQLite or ordinary-query tests.

Required SQLite HTTP tests cover actual/unused/overlapping mappings, hidden BGP
bindings, nested UNION/JOIN bags, RDF-vs-SQL equality, NULLs, late origins, both
forms, alternating portable-policy subjects and exact byte/witness failures.
Compiler tests pin graph/inverse/identifier/expansion rejection; stream tests pin
deadline and cleanup failure. The owned PostgreSQL/MySQL TLS CLI aggregate also
contains multiple-map SELECT/UNION/join/CONSTRUCT and allowed/empty portable-policy
checks with parsed returned provenance. That required aggregate passed on
2026-09-08 against owned PostgreSQL 16.15/MySQL 8.4.11 fixtures (63.39 seconds).
Its presence or ordinary test ignores alone are not qualification evidence.
The same required aggregate passed in 84.48 seconds after adding twelve native
multi-map SELECT/CONSTRUCT cases: deadline, disconnect and forced SIGTERM on
each pinned provider. Each case observes the exact encrypted target session
blocked on a held table lock, then proves native work stops while the lock stays
granted. A distinct same-credential CLI sibling remains blocked on its own held
lock and subsequently returns its exact bag. Deadline/disconnect cases recover
exact results and actual origins through the target's cap-one pool; forced
shutdown requires bounded clean exit and closed ingress. Failed responses cannot
emit successful chunked completion or a parsed lineage completion record, even
when the record crosses HTTP chunk boundaries. This is single-source multi-map
SELECT/CONSTRUCT evidence, not lineage UNION/JOIN cancellation, portable/source-RLS
cancellation, reload or every operator combination.
Full lineage, all native lifecycle/source-RLS combinations and exact-artifact
release admission remain open under ADR-0055.

### Bounded federated UNION profile (2026-09-08)

`bounded-federated-union-lineage-v1` adds the same media type to the existing
two-source SELECT UNION: exactly two one-triple arms, each belonging to a distinct
source, with no global modifiers. Both source-affinity admission and execution use
the bounded actual-origin compiler, including subproperty/inverse matching.
Ambiguous or absent sources reject before generation/source acquisition. Multiple
inverse declarations are retained as a deterministic set, never overwritten;
the bounded lineage compiler caps 256 inverse partners before copying them.

Each arm carries its private plan/spec through the existing immutable source and
security binding. The header contains two source-keyed mapping dictionaries and
one snapshot/logical-plan/policy identity. Per-result PROV-O names only actual
contributing mappings from that result's source; reversed arm order cannot swap
catalogs. Source IDs are snapshot-local. Policy identity names the configured
registry snapshot, not the caller or physical parameterized SQL. No row keys or
distributed point-in-time database snapshot are asserted.

Each arm resolves its at-most-1,024-witness relations before emitting rows. One
request budget covers both arms, metadata, source work, retained state and all
serialized bytes. UNION bag occurrences remain separate across sources. Blank
nodes, including nested triple terms, are standardized apart by SourceId in both
ordinary and lineage UNION responses, with scratch growth charged before mutation.
Both source owners are acquired before 200; successful completion requires both
executions and owned cleanup. A source/budget failure cannot complete a prefix.

Required SQLite HTTP tests cover actual multi-map origins, reversed/unbound bags,
entailed affinity, blank-node scope, alternating portable callers, rejection before
held pools, exact byte/result limits, witness overflow, cap-one recovery and pinned
activation. The existing required pinned PostgreSQL/MySQL TLS CLI aggregate checks
actual federated metadata against ordinary complete bags and includes six lineage
UNION deadline/disconnect/SIGTERM cases: exact encrypted target stop under held
locks, unaffected separately locked sibling, cap-one recovery and bounded clean
forced exit. This does not qualify federated lineage JOIN/CONSTRUCT, native lineage
reload, portable/source-RLS cancellation, protected generations, all operators or
exact-release admission. Full ADR-0017 remains open under ADR-0055.

### Bounded federated join profile (2026-09-08)

`bounded-federated-join-lineage-v1` uses the same explicit media type for the
already-admitted ADR-0006 two-pattern inner join. The existing base-table proof
seals exactly one actual direct mapping emitter per mandatory source arm; its
nonempty, source-unique ID is at most 1,024 bytes. Unused maps are not contributors.
The source-keyed proof follows cost-based build/probe swapping and the immutable
security binding; it is not a synthetic UNION witness recipe or a new join engine.

The header names both source catalogs and the pinned snapshot/logical-plan/policy,
with `maxBuildTriples: 128`, `maxProbeTriples: 4096`, and no row keys. Each exact
matched/projected bag occurrence emits one activity/result bundle using both
sources, actual mapping entries and mapping documents. Each mapping entry's
`sf:source` explicitly identifies its source, even when authored map IDs coincide.
Hidden keys and filtered/nonmatching rows create no provenance. Empty joins have
only a header and zero-count completion. Source blank-node scope and exact RDF
comparison remain unchanged. There is no distributed data-snapshot claim.

Ordinary and lineage results share the existing capped pre-200 serializer and
source/native cleanup owners. All output bytes, including completion, consume the
request budget; any overflow, source failure or observed cancellation discards
the staged response. Completion follows successful cleanup of both fragments.
Required public tests cover exact/projected bags, both origins, caller isolation,
blank scope/collation, pinned activation, rejection, caps and cap-one recovery.
The required pinned PostgreSQL/MySQL TLS CLI aggregate parses both-source PROV-O,
checks exact twelve-pair bags in both pattern orders, and tests twelve additional
join-lineage deadline/disconnect/SIGTERM cases with encrypted target, held-lock,
unaffected sibling, exact cap-one recovery and bounded clean exit witnesses.
The native reload qualification below now covers this join; portable/source-RLS
cancellation, protected generations and exact-release qualification remain separate.

### Native authored reload qualification (2026-09-08)

The required `source_tls_live` aggregate above passed in 120.14 seconds with
lineage-specific reload assertions on its owned PostgreSQL 16.15/MySQL 8.4.11
TLS fixtures. Valid authored replacements change returned values and mapping-document
identities for single-source constant and overlapping-map SELECT/CONSTRUCT and
mixed-source UNION. A nonempty federated join changes both subject templates,
preserving all twelve exact pairs in both pattern orders and both actual origins.
Constant/federated SELECT bags match ordinary results; the overlapping-map lineage
profile uses exact fixture values because the ordinary optimizer rejects that shape.

A held native query is observed by exact session ID before invalid Turtle is written:
multi-map SELECT on each provider, mixed UNION and forward join on PostgreSQL.
`/readyz` and new authenticated lineage queries become `503`, unauthenticated queries
remain `401`, and `/livez` remains `200`. The old native work stays active under its
held lock, then completes with its original exact bag, authored mapping documents,
policy and internally consistent generation references. Repair restores original
documents/results and readiness through a fresh generation. Periodic unchanged
rebuilds may change resource identities, so distinct-request generation equality is
not used as proof of semantic identity. Complete transport and returned PROV-O/RDF
are parsed; a header or readiness check alone cannot qualify this slice.

This is not policy/configuration reload, a distributed data snapshot, a held-query
test for every graph/operator/order, source-RLS lineage or exact-release admission.

### Native source-RLS qualification (2026-09-08)

The required `pg_generation::live_tests::rls::public_row_security_is_isolated_and_cleans_pool`
test now requests and parses actual lineage through the public router on an owned
PostgreSQL 16.15 fixture (5.73 seconds). Alternating A/B/A credentials, including
spoofed identity headers, return exactly the authorized constant/overlapping-map
SELECT/CONSTRUCT products and federated UNION/join bags. Returned PROV-O binds
actual map/source contributors to the header's snapshot/plan/policy; each joined
mapping entry explicitly identifies its source. No row keys, denied values or raw
configured identities appear. Empty authorized results create no solution/activity.

Each completed response reuses the same clean cap-one pool member (both independent
members for federation), with no transaction-local identity or read-only transaction
left behind. Concurrent constant-origin SELECT also isolates callers. Constant-origin
SELECT/CONSTRUCT additionally cover body-drop, slow-policy deadline and policy-error
cleanup/recovery; failures cannot complete the body successfully. Disabled source RLS
rejects lineage; invalid credentials reject, and unsupported ASK lineage rejects even
with the pool held. This qualifies these public source-RLS paths, not every failure
permutation, federated graph forms, policy installation or configuration hot reload.
The fixture uses local PostgreSQL transport; remote TLS is separately qualified by
the required native CLI aggregate, not newly inferred here. Exact-release admission
and remaining ADR-0055 guarantees still require candidate-bound evidence.
The declared bounded lineage profiles now have the required recorded public/native
coverage; `l-lineage` is non-blocking under ADR-0055. This does not complete the
historical broader ADR, authorize row keys or close separate budget/backend/release gates.

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
