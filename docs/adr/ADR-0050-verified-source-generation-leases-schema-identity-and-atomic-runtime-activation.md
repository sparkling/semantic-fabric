---
status: proposed
date: 2026-09-02
updated: 2026-09-02
tags: [schema, lifecycle, snapshot, digest, lease, reload, direct-mapping, postgres]
supersedes: []
depends-on: [ADR-0006, ADR-0007, ADR-0011, ADR-0015, ADR-0038, ADR-0048]
implements: [ADR-0038]
---

# Verified source-generation leases, schema identity, and atomic runtime activation

## Status boundary

This ADR is **proposed**. It proposes the design boundary for ADR-0038 M5; it does
not claim that canonical schema identities, a runtime snapshot manager, reload,
drift detection, a backend-generation lease, or live Direct Mapping exists.

The current Rust serving path safely owns one startup mapping, ontology,
constraint/type-quarantined schema observation, backend and plan cache inside a
single `RuntimeBinding`. Its process-local compile scope prevents detached-plan
reuse. PostgreSQL catalogue reads use one read-only repeatable-read startup
transaction. Those are sound precursors, not a mutable-schema authority: the
transaction ends before compilation and streamed execution, later requests may
use another pooled connection, and there is no digest, watcher, readiness
transition or atomic replacement path.

Node and MetaHarness may test vectors and lifecycle properties but remain
development/evidence infrastructure under ADR-0048. Every product type,
dependency, backend lease and activation mechanism in this decision is Rust.

## Context

M5 requires an immutable
`RuntimeSnapshot { T, M, schemas, sources, epochs, digests }`, off-path
validation, zero-downtime activation, exact stale-plan invalidation and schema
drift readiness. Three different concepts must not collapse into one word:

1. a repeatable content digest says that two normalized observations are equal;
2. an application snapshot lease keeps one immutable Rust value alive; and
3. a verified backend-generation lease proves that the source facts authorizing
   compilation remain valid through the complete streamed execution.

A digest or `Arc` cannot close source DDL time-of-check/time-of-use. Conversely,
holding a database transaction does not define stable cross-run content identity.
The runtime needs both, with separate types and authority.

The existing `TableSchema` is also deliberately insufficient as a digest
preimage. It uses unqualified names and a lossy driver type string, and it mixes
semantic facts with volatile row/distinct estimates. PostgreSQL currently omits
typmod, type namespace/definition, domains, enums, arrays and collation. Hashing
that DTO would miss semantic drift and make statistics churn invalidate caches.

Direct Mapping makes the boundary sharper. PK/FK facts change its RDF graph;
no-PK rows need a typed identity; and generated mapping must execute against the
same verified source generation. Redacting constraints after mapping generation
does not repair stale authority.

## Decision

### 1. Keep three authority types disjoint

The implementation introduces three non-interchangeable concepts:

- `ObservedSchemaIdentity` is bounded, canonical, repeatable content identity.
  It is diagnostic/cache input and grants no constraint, type or execution
  authority.
- `RuntimeSnapshotLease` pins an immutable activated application snapshot for a
  request. It keeps its mapping, ontology, source registry, compiler bindings,
  cache namespaces and readiness generation alive through response EOF, error,
  cancellation or body drop. It grants no database-generation authority.
- `VerifiedGenerationLease` is a private, unforgeable, backend- and
  `SourceId`-specific capability. Only a qualified backend adapter may construct
  it after coherent revalidation, and it owns the database resources and guards
  needed from that point through compilation and the complete streamed cursor.

Digest equality never constructs or upgrades either lease. Public callers
cannot construct a verified authority enum, token or snapshot from raw DTOs.

Non-authorizing parse and dependency discovery may precede lease acquisition,
but may create no cacheable or executable plan. The adapter then protects and
reobserves the closed dependency set, or the complete bounded admitted
catalogue, before authoritative compilation. A multi-source request acquires one
lease per participating `SourceId` before semantic response commitment and
makes no cross-database atomic-snapshot claim.

### 2. Validate a bounded schema before hashing it

Phase 2 `sf-sql` adapters will bound catalogue collection with cap-plus-one or
an equivalent bounded stream before constructing the normalized input. The
pure `sf-core` builder independently revalidates Appendix A's exact limits and
cross-references before producing an immutable observation. Duplicate structural coordinates,
ordinals, exact relation/column names, facet keys, dangling references,
malformed keys and overflow reject. Exact duplicate semantic constraint records
collapse only under Appendix A's closed law; case-distinct identifiers remain
representable and mapping ambiguity is resolved separately.

Planning statistics are stored separately. Row counts, distinct estimates,
histograms and collection timestamps never enter semantic schema digests or
grant rewrite authority.

### 3. Use versioned, domain-separated canonical digests

Observed Schema Identity V1 defines three SHA-256 newtypes:

- `StructuralSchemaDigestV1`: qualified relation identity/kind and ordered
  column identity/ordinal;
- `TypeSchemaDigestV1`: each column's normalized source type semantics,
  including backend family, qualified type identity, length/typmod,
  precision/scale, time-zone behavior, domain/base definition, enum/array graph
  and collation where relevant; and
- `ConstraintSchemaDigestV1`: normalized NOT NULL, PK, UNIQUE and FK facts,
  including validation/enforcement state, unique-null semantics and FK-match
  semantics.

The complete Observed Schema Identity V1 contract is split into two files only
to satisfy the repository's strict file-size rule:

- [Normative Appendix A — model and canonical byte contract](../design/ADR-0050-observed-schema-identity-v1-contract.md)
- [Normative Appendix B — known-answer vectors](../design/ADR-0050-observed-schema-identity-v1-known-answer-vectors.md)

Both appendices are inseparable parts of this ADR and always share its status.
Neither is an independent ADR, implementation claim, evidence receipt or
authority source. Appendix A, not this summary, fixes the exact admitted model,
caps, octets, tags, ordering, duplicate law and semantic fields. Appendix B
fixes the literal preimages and expected digests. No implementation may emit V1
until it reproduces Appendix B exactly.

Database-local identifiers such as PostgreSQL OIDs may bind the current live
lease, but durable type identity also contains normalized qualified semantics.
SQLite records its dynamic per-value typing policy rather than pretending a
declared affinity is an authoritative value type. MySQL uses `COLUMN_TYPE`, not
lossy `DATA_TYPE`, but remains observational until independently qualified.

### 4. Activate one complete generation atomically

`RuntimeSnapshot` owns all semantic and execution state that must agree:

- ontology and mapping, including `Authored` versus `Direct` origin;
- validated source registry, backends and capability policy;
- compiler-safe schemas and explicit authority modes;
- structural, type, constraint, ontology, mapping, capability and policy
  digests;
- fresh compiler bindings and private plan-cache namespaces; and
- an activation identity.

Candidate construction performs every source observation, validation,
`M ⋈ T` check, capability check, Direct-Mapping generation, cache creation or
warmup, and readiness calculation off-path. All fallible work precedes one final
allocation-free compare-and-swap. An expected-generation mismatch rejects and
drops the candidate without partial publication; every other failure also drops
candidate resources and leaves the active state byte-for-byte unchanged.

One atomic state cell exposes either `Ready(snapshot)` or a generation-bound
`NotReady { activation_id, cause }`; readiness has no second owner inside the
snapshot. Publication compares against the expected active activation. A slow
older candidate or watcher cannot overwrite or heal a newer generation.

`ActivationId` is checked-monotonic and distinct from repeatable content
digests. This prevents A-to-B-to-A ABA. A reload is a no-op only while the state
is ready and every activation-semantic input plus resource/configuration identity
is unchanged; schema-content equality alone is insufficient. Deliberately
returning from B to earlier A content builds a new candidate and receives a new
activation ID. Rollback never resurrects an old pointer, cache or backend lease.

Each request loads the state once. If ready, it holds the resulting
`RuntimeSnapshotLease` through the entire response body. Existing requests keep
their original application snapshot and new requests see the successor. Source-
generation coherence through completion is claimed only for an admitted verified
backend profile; observational profiles retain no such claim. After detected
relevant drift, new requests fail readiness until a validated candidate
activates. Old pools/caches drop only after their last request lease ends.

### 5. Qualify backend-generation leases separately

PostgreSQL is the first target. A verified lease must bind and recheck the exact
database, role/user, schema resolution, row-security state, backend capability
policy and live structural/type/constraint identities on the same owned
connection and transaction used for execution. It acquires the relation-level
protection or equivalent generation guarantee needed to prevent relevant DDL
through every branch and the complete stream. Transaction, lock and statement
limits are capped by the request's remaining `QueryBudget`; cancellation,
rollback and dirty-connection discard are mandatory.

The initial PostgreSQL verified profile admits only a closed public-base-table
dependency profile. It rejects RLS-dependent relations, views, functions,
unsupported user-defined types or collations, and every unresolved dependency.
RLS admission requires the later `SecurityContext` and policy-dependency
contract; recording `row_security` alone does not authorize it.

Compilation occurs only after the lease is established. All statements and
branches execute inside it; a pool checkout, transaction, lock or generation
mismatch rejects before semantic response commitment. A digest precheck on one
connection followed by execution on another is not verified mode.

Raw `rr:sqlQuery` is rejected in verified mode unless a future design extracts,
validates and holds its complete relation/view/function/result-type dependency
closure in the same lease. Base-table digests alone cannot authorize raw SQL.

SQLite may be qualified later with a documented file/database generation and
transaction law. MySQL verified mode remains rejected until equivalent
consistency and DDL-race evidence exists. Observed digests may still be emitted
for both without promotion.

### 6. Make live Direct Mapping a leased lifecycle

Direct Mapping accepts only the validated schema representation and a validated
absolute base IRI whose composition cannot create invalid class, predicate or
row IRIs. Duplicate tables/columns, malformed keys, FK arity mismatch, dangling
parents and unchecked generated IRIs reject before cache creation, activation or
data-query I/O; bounded catalogue collection is itself source I/O.

The snapshot binds mapping origin, base IRI, structural/type/constraint
digests, row-identity policy and generated mapping digest. Candidate Direct
Mapping is generated under a candidate-build verified lease and records its
schema and mapping-input identities; that lease is released before publication
and `RuntimeSnapshot` never stores a live database transaction. Each request
obtains a fresh execution lease and revalidates those exact identities before
authoritative compilation and streaming. Mapping-generation authority remains
distinct from optimizer constraint authority. A type-only change therefore
cannot retain a falsely reusable mapping/cache identity merely because the IR
shape is equal.

No-PK identity is a typed backend capability, never a magic string column.
PostgreSQL `ctid` and the current `rowid` sentinel are not stable generation
identity. PostgreSQL no-PK Direct Mapping rejects until one transaction-bound
identity proves same-row blank-node stability across every branch and concurrent
update/vacuum. Row identities and blank-node labels never enter telemetry or
persist across generations.

### 7. Deliver in authority-preserving phases

1. **Pure Observed Schema Identity V1 kernel:** add only neutral `sf-core`
   values, generic validation, canonical encoding, hashing and exact Rust test
   vectors. It performs no I/O and changes no adapter or runtime binding.
2. **Observational source integration:** register backend profiles, bound
   catalogue collection, improve type fidelity, construct V1 inputs and carry
   the three digests in the current binding while every compiler authority
   remains `Unverified`.
3. **Runtime generation identity:** replace process-only cache identity with
   activation/content inputs while retaining fresh per-snapshot caches. This
   cannot replace process-unique binding identity until every cache-semantic
   mapping, ontology, capability and policy digest has a canonical contract.
4. **PostgreSQL verified lease:** bind one owned protected transaction through
   revalidation, compilation and complete streaming.
5. **Atomic activation and drift:** add candidate construction, CAS publication,
   readiness, stale-watcher rejection and body-lifetime snapshot leases.
6. **Typed row identity and Direct Mapping:** validate/generate from the leased
   schema and admit backend profiles one at a time.

Phases 1 and 2 do not add reload, verified authority or live Direct Mapping.
Each later phase requires its own executable evidence before capability
promotion.

Crate ownership follows ADR-0006: `sf-core` owns neutral validated values and
pure canonical hashing; `sf-sql` owns catalogue and lease I/O; `sf-serve` owns
activation and readiness; and `sf-mapping` remains pure validated-schema-to-
mapping generation.

## Required evidence

- exact digest known-answer, domain/version separation and canonical-boundary
  tests;
- relation-permutation invariance only where semantics are unordered, with
  column/key-order sensitivity;
- type-only, constraint-only and statistics-only mutations changing exactly the
  intended identity;
- structural/type duplicate rejection, exact semantic-constraint
  deduplication, dangling, Unicode/framing, exact-cap and cap-plus-one rejection;
- PostgreSQL old-or-new coherent observation under deterministic DDL barriers;
- request-generation pinning across reload, body EOF/drop/error/cancellation and
  exact once-only old-resource release;
- invalid-candidate nonactivation at every stage, slow-A/fast-B CAS, A-B-A ABA,
  stale readiness and rollback-as-new-generation tests without sleeps;
- isolated PostgreSQL DDL barriers between revalidation/preparation/branches,
  pool-member mismatch, raw-query view/function drift and dirty-transaction
  cleanup; and
- Direct-Mapping malformed-schema, base-IRI, real-`rowid`, duplicate no-PK row,
  concurrent update/vacuum and blank-node stability tests.

Live tests use project-owned isolated databases and never the product-mock
instance. Adversarial redaction tests seed public errors, debug output,
readiness and metrics with credentials, paths, raw SQL, names and values and
require that none escape.

## Consequences

- **Positive:** content identity, application lifetime and source authority are
  explicit and cannot accidentally promote one another.
- **Positive:** reload swaps the whole semantic generation and preserves old
  requests without mutating caches in place.
- **Positive:** Direct Mapping becomes the same validated lifecycle as authored
  mapping rather than a parallel compiler architecture.
- **Cost:** a verified PostgreSQL request may hold a transaction and relation
  protection for the full stream, so admission, timeouts and cancellation are
  mandatory.
- **Cost:** richer bounded catalogue observation and canonical type graphs add
  adapter work before reload can ship.
- **Neutral:** observed digests improve diagnosis and cache identity before any
  backend is admitted to verified mode.

## Alternatives rejected

- **Hash the current `TableSchema` debug/JSON form** — lossy, statistics-sensitive
  and not a versioned canonical contract.
- **Use a digest as a generation lease** — leaves DDL and pool-member TOCTOU.
- **Treat `Arc<RuntimeSnapshot>` as database authority** — pins Rust memory, not
  source facts.
- **Mutate a live binding/cache and increment an epoch** — lets mixed-generation
  state escape and complicates rollback.
- **Swap schema, mapping and readiness independently** — admits partial states.
- **Generate live Direct Mapping then quarantine constraints** — stale PK/FK
  authority already changed the generated RDF mapping.
- **Use PostgreSQL `ctid` as durable row identity** — snapshot-local and unsafe
  across generations or unconstrained branches.
- **Qualify every backend at once** — hides materially different consistency and
  type systems behind an unproved common interface.

## Rules

- **R1** — canonical content digests, snapshot leases and verified backend leases
  are distinct types; none promotes another.
- **R2** — raw catalogue input is bounded and validated before hashing or mapping
  generation; planning statistics are not semantic identity.
- **R3** — an activated generation is immutable and published as one CAS-protected
  state after all fallible work succeeds.
- **R4** — every request pins one snapshot through response termination; verified
  source authority, where supported, spans compilation and the complete stream.
- **R5** — drift/readiness and rollback are activation-ID bound and ABA-safe.
- **R6** — verified raw SQL and no-PK Direct Mapping reject until their complete
  dependency and row-identity laws are independently proved.
- **R7** — backend qualification is per profile; observation never implies
  production admission.
- **R8** — product implementation is Rust; Node remains evidence-only.

## Links

[ADR-0006](ADR-0006-crate-layout-and-performance-model.md),
[ADR-0007](ADR-0007-sparql-to-sql-rewriting-strategy.md),
[ADR-0011](ADR-0011-observability-and-configuration.md),
[ADR-0015](ADR-0015-datatype-dialect-correctness.md),
[ADR-0038](ADR-0038-sota-application-completion-programme.md), and
[ADR-0048](ADR-0048-rust-production-and-node-evidence-runtime-boundary.md).
Normative companions:
[Appendix A](../design/ADR-0050-observed-schema-identity-v1-contract.md) and
[Appendix B](../design/ADR-0050-observed-schema-identity-v1-known-answer-vectors.md).
