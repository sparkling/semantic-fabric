---
status: accepted
date: 2026-09-02
updated: 2026-09-23
tags: [schema, lifecycle, snapshot, digest, lease, reload, direct-mapping, postgres, sqlite, mysql]
supersedes: []
depends-on: [ADR-0006, ADR-0007, ADR-0011, ADR-0015, ADR-0038, ADR-0048]
implements: [ADR-0038]
---

# Verified source-generation leases, schema identity, and atomic runtime activation

## Decision status

This ADR is **accepted as architecture**. Acceptance binds Decision sections
1–7, Rules R1–R9, and the two normative appendices. It does not claim that every
phase is implemented, enable a backend profile, or grant production admission.

### Implementation status (non-normative)

The Phase 1 pure `sf-core` Observed Schema Identity V1 kernel is implemented as
a non-authorizing content-identity utility.
[ADR-0051](ADR-0051-postgresql-16-public-observed-schema-profile.md) now accepts
the first production-shaped profile for observation qualification only. Its
private, opt-in `sf-sql` diagnostic can emit a branded identity for PostgreSQL
16.9/16.15 after bounded rich observation. The tracked pair receipt binds two
fresh generation runs per exact patch and independent clean replay, and passes
only that observation-profile gate. Ordinary authored
mapping carries the whole identity or one closed unavailable reason only as a
non-authorizing backend/`SourceId`-bound diagnostic. The private Direct-Mapping
foundation may retain the identity with the complete rich table and session
expectations checked by an unforgeable lease; the digest itself still grants no
compiler, cache, readiness, mapping or execution authority.

The 2026-09-05 Phase 5 foundation is implemented in Rust: `RuntimeSnapshot` owns
a source-keyed immutable registry and deterministic compile identities; one
`RuntimeManager` state linearizes readiness and whole-snapshot replacement;
each ready request acquires exactly one application-snapshot lease before body
polling and retains it through response EOF, error, cancellation, or drop; and
generation-bound not-ready state rejects new requests as a redacted pre-I/O
`503`. Checked activation identities prevent ABA, complete expected-readiness
comparison rejects ready-to-not-ready and slow-candidate races, and stale
watchers cannot mark a newer activation unavailable. An opaque checked state
revision now advances on activation and every accepted not-ready observation,
including same-cause repeats. Request-generation failures remain request-scoped;
only the private lifecycle coordinator may fence after a completed control
observation. Shutdown atomically fences the current
state after closing transitions, so it cannot leave a racing activation ready, and a transition
losing that race reports `ShuttingDown` rather than a misleading stale state. Deterministic tests
cover these races, old/new HTTP results, failed construction, response lifetime,
and last-pin release. Checked revision exhaustion terminalizes readiness with a
closed `StateRevisionExhausted` cause before reporting the counter error.

The publication primitive is crate-private and deliberately non-authorizing.
The construction path requires bounded sealed `M ⋈ T` validation before a
binding, and its policy-v2 receipt partitions compile/cache identity. The
all-or-nothing registry validates every source before constructing any binding.
The closed `PgDirectLifecycleV1` is promoted to public Rust/CLI startup on 2026-09-08, with independent review and required owned-TLS CLI qualification on exact PostgreSQL 16.9/16.15. `--direct-mapping-base` selects the one-source PK-backed profile below. It uses fixed startup ontology/base/configuration, independent request/control TLS pools, mandatory five-second observation (nonzero `--reload-interval-secs` overrides), a 30-second control budget and a 60-second startup acceptance deadline. Candidate timeout retains native work, forbids overlap/late publication and makes panic terminal. Control discovery, probes and native cleanup retain a cap-one ownership permit. Shutdown joins the coordinator and counts request/control owners under the existing drain plus three-second forced allowance; exhaustion errors, without a hard CPU-preemption claim. This is public profile qualification, not general backend or production admission.

The 2026-09-07 authored profile exposes `--reload-interval-secs` (zero/off by default, otherwise 1–86400 seconds). One serialized off-path worker captures bounded regular-file bytes, rebuilds all configured bindings and sealed semantic admission, rechecks the captured files, then publishes by exact readiness CAS. Changed bytes fence before parsing/source I/O; changed schema fences immediately after that source observation, before remaining validation or the next source. Unchanged observations keep readiness while rebuilding. Fresh pools are a new resource identity even with equal schema, so SQLite replacement cannot keep an old inode through a digest no-op.
A 60-second attempt deadline fences readiness; the coordinator retains a timed-out worker until actual completion rather than queueing replacements. Panic, including after timeout, is terminal; shutdown cannot be healed. Source endpoints, resolved credentials/TLS trust, caller policies, source slots and service limits remain fixed. Required tests exercise old/new public request results, immediate drift fencing, invalid-input recovery, portable row-policy preservation, FIFO rejection, timeout ownership, actual CLI reload/shutdown, and encrypted PostgreSQL/MySQL single/mixed-source reload.
Ordinary authored reload is coherent application-generation publication with observational drift detection, **not** a backend DDL-generation lease, configuration/policy hot reload or completion of the broad lifecycle profile.
**2026-09-10 protected authored profile:** `--require-verified-generation` (TOML `[serve] require_verified_generation`, environment `SEMANTIC_FABRIC_REQUIRE_VERIFIED_GENERATION`) additionally requires one PostgreSQL 16.9/16.15 source, nonzero authored reload, bounded unqualified public `rr:tableName` mappings, and read-all or portable equality-row admission (extended 2026-09-20). Native PostgreSQL RLS and mixed registries remain excluded. This explicit fail-closed requirement never silently downgrades; ordinary authored modes are unchanged. Reuse of the existing non-owner, NOINHERIT read-all role and whole-public observation means unrelated unsupported public DDL can fence readiness. Authored mappings need no primary key. The locked snapshot's legacy tables feed existing authored semantic admission/compilation; its separate rich identity/tables/session bind `Authored` origin, source ID and exact mapping digest in the common generation expectation. Each request keeps the existing 34-unit reservation, retained compiler permit, same-connection execution, final recheck and bounded rollback. A clean acknowledged rollback permits reuse; unfinished cleanup detaches. The serialized reload retains a cap-one control owner through actual completion and forced shutdown, uses a 30-second source/candidate budget, compares verified identity as well as normalized compiler tables and preserves typed schema/capability/unavailability causes. Required owned-TLS CLI evidence on both patches covers authenticated SELECT/ASK/CONSTRUCT, NOWAIT DDL refusal, invalid-input/schema recovery, complete old streamed results across a published successor, disconnect/deadline/forced-shutdown cleanup, whole-public coupling, and wrong-CA/role/raw-SQL/zero-reload/33-unit refusal. This qualifies that profile only, not other backend/policy generations, hard CPU preemption or production admission.

A private PostgreSQL candidate/request foundation now implements the essential
verified-generation ownership law. One pool member is marked dirty before
`BEGIN`; the exact public-table set is relation-locked before the first
repeatable-read snapshot; bounded rich identity, complete table facts, database,
role, session and policy context are captured on that connection. Candidate
primary-key-backed Direct Mapping is generated while it is protected, then
rechecked and rolled back cleanly before its inseparable expectation is stored.
For each internal request, a non-cache-authorizing preflight reserves the exact
compiler permit before source I/O; the permit remains held without requeue while
the request acquires and revalidates the lease, then moves into authoritative
compilation. Typed executable inventories derive a 34-unit metadata reservation,
with 33 rejected before pool I/O. Required-live SELECT, ASK and CONSTRUCT execute
through the lease-owned connection before final recheck and acknowledged rollback.

A fixed cleanup allowance is independent of the expired user deadline. If
timeout, cancellation, error, drop or a retained execution view prevents
acknowledged rollback, the member remains dirty, attempts one bounded native
cancel and detaches the pool object instead of recycling uncertain state. Isolated disposable PostgreSQL 16.9 and 16.15
live gates exercise lock-before-snapshot, clean close, incompatible DDL
exclusion, compatible additive-FK old-generation coherence followed by
next-acquisition drift, policy mutation, cancellation and dirty-member
replacement. Public startup now consumes this same sealed path; the required owned-TLS CLI check adds exact authenticated SELECT/ASK/CONSTRUCT and lineage, traffic-independent drift/rebuild, unchanged startup ontology, NOWAIT DDL conflict/recovery, bounded shutdown and wrong-CA/no-PK startup rejection on both patches. Other backend leases, no-PK identity and production admission remain open; this is not general Phase 5 or Phase 6 completion.

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
allocation-free compare-and-publish linearization. A checked replacement under
one state-cell lock and a lock-free compare-and-swap are semantically
equivalent here: readers may observe only a complete old or complete new state.
An expected-state mismatch rejects and drops the candidate without partial
publication; every other failure also drops candidate resources and leaves the
active state byte-for-byte unchanged.

One atomic state cell exposes either `Ready(snapshot)` or a generation-bound
`NotReady { activation_id, cause }`; readiness has no second owner inside the
snapshot. Publication compares against an opaque state revision that changes on
every semantically relevant readiness transition. A slow older candidate or
watcher cannot overwrite or heal a newer state.

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

#### Sealed semantic admission is a separate authority

Before `RuntimeBinding` or its cache exists, executable mapping IR is projected
to a bounded ground RDF graph containing only the class, predicate, object-map,
and effective datatype facts consumed by the four sealed shapes. The product-
owned `sf-validation` crate runs a deterministic workload/cardinality preflight,
three Core shapes through rudof Native, and the datatype component's exact
parsed sealed `sh:select` once globally over `M ⋈ T`; violations fail the whole
candidate. Blank POM focus preserves the prior redacted fail-closed behavior.

Projection structural nodes are named only below
`urn:semantic-fabric:mjoin-t:v1:`. An ontology using that reserved prefix as an
asserted named subject, predicate, or named object is rejected before merge, so
T cannot forge facts about M's structural nodes. A private `ValidatedMapping`
receipt owns the mapping and binds its origin, exact ontology document digest,
canonical projection, count-only redacted outcome, warning policy and validation
policy v2. That policy covers shape/query bytes, Native/global topology, exact
evaluator/parser versions and features, limits, preflight revision and blank-focus policy. The ontology
and source-effective projection are recomputed before receipt consumption, and
their digests enter compile/cache identity.

This receipt binds semantic compatibility, not the physical database,
connection, source generation, or DDL lifetime. It cannot construct an
`ObservedSchemaIdentity`, `RuntimeSnapshotLease`, or `VerifiedGenerationLease`;
the latter authority problems remain governed independently by this ADR.

### 5. Qualify backend-generation leases separately

PostgreSQL is the first target. A verified lease must bind and recheck the exact
database, role/user, schema resolution, row-security state, backend capability
policy and live structural/type/constraint identities on the same owned
connection and transaction used for execution. It acquires the relation-level
protection or equivalent generation guarantee needed to prevent relevant DDL
through every branch and the complete stream. Transaction, lock and statement
limits are capped by the request's remaining `QueryBudget`; cancellation,
rollback and dirty-connection discard are mandatory.

The initial PostgreSQL verified profile admits only a closed public-base-table dependency profile. It rejects RLS-dependent relations, views, functions,
unsupported user-defined types or collations, and every unresolved dependency. RLS admission requires the later `SecurityContext` and policy-dependency
contract; recording `row_security` alone does not authorize it. Pinned 16.9/16.15 native authority tests now prove table ACCESS SHARE excludes policy DDL but not role-capability or membership changes. A retained cursor keeps its earlier qualified rows while later statements can use changed authority, even when repeatable-read `pg_roles` still returns old flags. Forced generic prepared plans retain stale role-dependent decisions on 16.9; 16.15 invalidates them. This prerequisite evidence does not admit protected native RLS; same-transaction catalogue rechecks alone cannot close its authority race.

A non-cache-authorizing semantic/resource-shape preflight may run before source
I/O only while retaining the exact compiler permit. Authoritative compilation
occurs after the lease is established without requeue. All statements and
branches execute inside it; a pool checkout, transaction, lock, binding or
generation mismatch rejects before semantic response commitment. A digest
precheck on one connection followed by execution on another is not verified
mode.

Raw `rr:sqlQuery` is rejected in verified mode unless a future design extracts,
validates and holds its complete relation/view/function/result-type dependency
closure in the same lease. Base-table digests alone cannot authorize raw SQL.

The explicit authored SQLite profile uses file-backed, fresh read-only sealed members opened from canonical filesystem paths (URI/VFS connection options are rejected), WAL or DELETE journaling, bounded unqualified base-table mappings, one source, nonzero reload and read-all or portable equality-row admission. In-memory/attached/temporary schemas, virtual/shadow tables, read-uncommitted/writable-schema settings and other journal modes fail closed. A `BEGIN DEFERRED` read transaction owns the coherent schema observation, authoritative compilation/cache use, every metadata probe and response branch, final observation and acknowledged rollback. WAL preserves the old read snapshot across committed successor DDL; DELETE prevents DDL commit while that reader holds its shared lock. This is SQLite's local transaction law, not a distributed database snapshot or a hard I/O deadline.

The equality record includes exact schema object SQL, compiler column facts, schema cookie and journal mode. It is not an ObservedSchemaIdentity V1 profile. Logical observation admits at most 8,192 objects, 4,096 tables, 65,536 columns and 4 MiB of copied text, with request source-work charges before copies/growth and sorting only after bounds. Native catalogue allocation and physical I/O are not measured by those logical bounds. Candidate semantic admission runs while its separate control transaction remains owned. Request acquisition compares the activated expectation on its own sealed physical member before authoritative compilation; legacy raw handles cannot expose that member. Queued operations register before blocking submission. Closing rejects new operations, drains registered workers and rolls back before success; cancellation retains cleanup ownership, and panic, uncertain rollback or lost runtime poisons member admission. The caller drops executor views before awaiting final cleanup. SELECT, ASK, CONSTRUCT and single/multiple-mapping lineage use this path.

`sqlite_generation` CLI tests exercise authenticated typed bags, all normal forms, lineage completion, semantic/schema refusal, automatic reload/recovery, source-work refusal and shutdown for WAL and DELETE. Backend and serving tests cover held snapshots, queued-worker ordering, cache drift refusal, terminal cleanup and exact runtime binding. **2026-09-23 ordinary SQLite (G3, `1d788779`):** an ordinary authored mapping over file-backed WAL/DELETE SQLite that this sealed builder admits now holds the same per-request `BEGIN DEFERRED` lease without `--require-verified-generation` or a reload interval, refusing with a typed 503 on drift; with reload disabled a changed source refuses until restart. Sources the builder declines (virtual/shadow tables, schema bounds, exact-case names) fall back to the unverified path so none stops starting; in-memory, URI and other-journal SQLite stay unverified. Reload never downgrades (user decision 2026-09-23): once a source is admitted verified, a refresh whose protected builder declines is refused with `CapabilityDrift`, keeping readiness fenced (typed 503) until a verified candidate or restart; `reload::tests::verified_source_never_reloads_unverified` proves it with an FTS5 table, and removing the guard fails it. **2026-09-23 ordinary PostgreSQL (G3, protected profile only, user decision):** ordinary authored PostgreSQL startup now attempts this same protected authored builder when the query admission permits verified generations (read-all or portable rows) and every mapping is a bounded unqualified base table; it inherits the protected profile's full requirements (16.9/16.15, qualified non-owner NOINHERIT reader role, pinned session, whole-public observation) and falls back to the unchanged unverified path when the builder declines, so reach is limited to deployments that already meet that profile. Native RLS bearers always keep the unverified path, because the binding applies RLS only to unverified PostgreSQL generations. The live fixture (`pg_generation/live_tests/ordinary_startup.rs`) proves an RLS bearer keeps RLS, a portable startup binds the verified generation and answers, and a same-name table replacement then refuses with 503; disabling either gate fails it. The same no-downgrade reload rule applies, because both use the same `open_ordinary` fallback and reload baseline. This does not close the remaining backend/policy programme. The qualified MySQL extension is specified below. Observed digests alone do not promote either backend.

Portable equality policies are authorized on the uncached preflight plan before shape admission or source acquisition, then reapplied to the authoritative plan under its lease. The immutable registry must contain only portable subjects; native RLS and mixed registries remain excluded, and Direct Mapping remains read-all-only. Candidate admission checks every policy column targeting a mapped table against the protected observation before activation; uncovered tables/sources remain intentional request denials. Existing opaque subject/cache and binding identities isolate cold/warm plans and reloads. Required SQLite WAL/DELETE and owned PostgreSQL 16.9/16.15 TLS CLI tests prove A/B isolation, denial before generation work, policy-column drift fencing/recovery, and mapping reload. PostgreSQL also proves a pinned A-only stream across activation and disconnect cleanup. Policy configuration remains immutable for the service lifetime; this does not close native RLS, ordinary-mode or federated generation obligations. The MySQL extension is specified below.

The explicit authored MySQL profile qualifies exact MySQL 8.4.11, one current database with `lower_case_table_names=0`, 1-256 unqualified base-table names and InnoDB only. It needs nonzero reload and read-all or portable equality-row admission; raw SQL, views and other engines reject. No primary key is required. Reset runs under the discard owner before the cancellation marker. The session fixes autocommit, READ ONLY / REPEATABLE READ, UTC, UTF8MB4 client/connection/results, `utf8mb4_bin` connection collation and the strict mode set plus `NO_BACKSLASH_ESCAPES`; emitted exact-comparison collation remains separate. Plain `START TRANSACTION READ ONLY` precedes a single constant-false UNION over all mapped tables. Exact MySQL source inspection and the pinned two-order positive/early-snapshot negative test establish that every table opens and acquires MDL before execution. An early consistent snapshot would retain stale data and dictionary facts and is prohibited.

Only after that barrier does bounded observation capture the session, table engine/type/collation/options and every ordered column's full type, nullability, charset/collation, generated/invisible attributes and default. Native fields and row counts are capped before copies; the aggregate copied metadata cap is 4 MiB, with at most 1,024 columns per table. Compiler schemas retain columns only, without unverified keys/statistics. These are equality facts, not ObservedSchemaIdentity V1 authority. A separate unpooled control connection holds candidate validation; request pool settings remain native MySQL options. Requests compare the activated expectation, compile and execute all response forms on the same owned connection, recheck metadata/session and active read-only state, then acknowledge rollback before restoring timeout/releasing the marker/reuse. Failures and cancellation hard-discard through the existing native stop owner. This is local transaction consistency, not distributed atomicity or a hard native-allocation/I/O bound.

`authored_generation::mysql::public_authored_mysql_generation_is_protected` qualifies the actual pinned TLS CLI: exact SELECT/ASK/CONSTRUCT/lineage, incompatible initial session defaults, authored backslash/apostrophe literals, invalid mapping/schema recovery, old streamed data across DML and activation, both-table DDL exclusion, partial-MDL deadline cleanup, disconnect/deadline/forced shutdown and cap-one recovery, portable A/B isolation and repeated policy-column drift fencing, and view/engine/raw-SQL/source-work refusal. The 2026-09-20 protected federation extension admits one or two authored sources from these exact PostgreSQL, SQLite and MySQL profiles. One candidate budget sums both source allowances under a single control owner; publication accepts both sources atomically. Requests acquire both binding/backend-matched local generation leases before authoritative compilation, execute UNION/join/lineage on those same owners, and close unused fragments on failure. Explicitly typed columns, templates and reference-join columns are checked against the held candidate schema with executor-compatible name resolution; a datatype cannot conceal a missing column. Required CLI evidence covers all 25 ordered profile pairs and six backend/position lifecycle cases: portable isolation, exact results, false-positive join rejection, atomic successor, drift, both held leases, cancellation and cap-one recovery. Independent local snapshots are not a distributed snapshot. Ordinary modes and native PostgreSQL RLS remain open; no production admission is granted.

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

#### Initial PostgreSQL lifecycle profile

`PgDirectLifecycleV1` is the only initial live Direct-Mapping profile. Its
builder, coordinator and public integration passed independent review and the required exact-patch public check. It admits exactly one PostgreSQL source using
ADR-0051's qualified `Postgres16PublicBaseV1` observation profile, permanent
`public` base tables, a primary key for every mapped table, one immutable
ontology, one validated absolute base IRI and one immutable resolved source
configuration. Plain read-all bearer, default-deny and explicitly unrestricted development admission remain available. Portable row policies and provisioned row-policy subjects reject at startup, alongside authored-plus-Direct mapping mixtures, RLS, raw SQL, no-PK
tables, federation, additional sources and other backends reject.

The profile has three closed source-failure classes. Connection, checkout,
query, statement-timeout, cancellation and row-decode failures before a
complete context is decoded are `SourceUnavailable`. A held verified lease that
successfully reobserves a different expected generation is `SchemaDrift`. A
complete decoded database, role, session or policy context that differs from
the expectation is `CapabilityDrift`. All three return the same redacted
request-scoped `503`; a request never changes application readiness directly.

Until a complete candidate builder and recovery coordinator are enabled, no
source-failure path may arm a global drift fence. Once enabled, exactly one
serialized coordinator owns source-readiness transitions. It uses a dedicated
bounded control connection, never a request-pool member; skips missed polling
ticks; permits at most one probe/build at a time; and compares the complete
opaque runtime-state revision before fencing or activation. A completed failed
control observation fences the source. While not ready, the coordinator keeps
retrying under fixed deadlines; only a completely validated successor candidate
may recover readiness through the atomic activation primitive. Unexpected
coordinator termination fails the source closed.

For this single-source profile, source readiness determines application
readiness. `/readyz` only projects that immutable state and never polls the
database. Digest equality, request traffic and caller-provided identifiers
cannot trigger or heal a transition. This profile adds no public administrative
reload and no hot reload of source credentials, files or configuration.

### 7. Deliver in authority-preserving phases

1. **Pure Observed Schema Identity V1 kernel (implemented 2026-09-02):** neutral
   `sf-core` values, generic validation, canonical encoding, hashing and exact
   Rust test vectors. It performs no I/O and changes no adapter or runtime
   binding. Test-reserved profile IDs exercise the contract but register no
   production profile.
2. **Observational source integration:** register backend profiles, bound
   catalogue collection, improve type fidelity, construct V1 inputs and carry
   the three digests in the current binding while every compiler authority
   remains `Unverified`.
3. **Runtime generation identity:** replace process-only cache identity with
   activation/content inputs while retaining fresh per-snapshot caches. This
   cannot replace process-unique binding identity until every cache-semantic
   mapping, ontology, capability and policy digest has a canonical contract.
4. **PostgreSQL verified lease (private foundation implemented 2026-09-06):**
   retain one compiler permit across preflight and lease acquisition, then bind
   one dirty, protected transaction through authoritative compilation and mapped
   SELECT/ASK/CONSTRUCT, final recheck and acknowledged rollback. Public profile
   qualification is now recorded for the closed Direct profile; other profiles remain open.
5. **Atomic activation and drift (closed profile implemented 2026-09-06):** the
   immutable registry, opaque validated-candidate publication, body-lifetime
   leases and full-state CAS are joined to one serialized `PgDirectLifecycleV1`
   coordinator. It skips missed ticks, fences only completed control failures,
   retries while not ready, heals only from a completely rebuilt candidate and
   fails closed on abnormal worker exit. Public Direct and authored reload are qualified separately; general lifecycle remains open.
6. **Typed row identity and Direct Mapping (public PK-backed profile promoted
   2026-09-08):** validate/generate the PostgreSQL candidate from its leased schema
   and reacquire the exact expectation for each request. No-PK identity and other
   backend promotion remain open.

Phases 1 and 2 do not add reload, verified authority or live Direct Mapping.
Phase 1 completion therefore grants no source, compiler, serving, cache or
activation authority. Each later phase requires its own executable evidence
before capability promotion.

Crate ownership follows ADR-0006: `sf-core` owns neutral validated values and
pure canonical hashing; `sf-sql` owns catalogue and lease I/O; `sf-mapping`
owns pure validated-schema-to-mapping generation and its ground admission
projection; `sf-validation` owns the sealed bounded Native/global split; and
`sf-serve` owns semantic receipts, activation, and readiness.

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
instance. The PostgreSQL 16.15 lifecycle gate closes the listed lock ordering,
same-generation execution, final-recheck, cancellation and dirty-cleanup
foundations. The separate exact 16.9/16.15 pair receipt closes only the
observation-profile qualification gate; neither evidence closes reload, public
Direct Mapping, backend admission or production admission by itself. Public promotion instead requires `cargo test --locked -p sf-cli --no-default-features --test source_tls_live direct::public_direct_mapping_has_authenticated_tls_startup -- --ignored --exact --nocapture`, alongside focused startup, lifecycle ownership and existing generation checks. Adversarial redaction tests seed public
errors, debug output, readiness and metrics with credentials, paths, raw SQL,
names and values and require that none escape.

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
- **R3** — an activated generation is immutable and published as one
  compare-and-publish-protected state after all fallible work succeeds.
- **R4** — every request pins one snapshot through response termination; verified
  source authority, where supported, spans compilation and the complete stream.
- **R5** — drift/readiness and rollback are activation-ID bound and ABA-safe.
- **R6** — verified raw SQL and no-PK Direct Mapping reject until their complete
  dependency and row-identity laws are independently proved.
- **R7** — backend qualification is per profile; observation never implies
  production admission.
- **R8** — product implementation is Rust; Node remains evidence-only.
- **R9** — requests never own readiness transitions; a source-failure fence is
  enabled only with its bounded recovery coordinator and complete validated
  candidate path.

## Links

[ADR-0006](ADR-0006-crate-layout-and-performance-model.md), [ADR-0007](ADR-0007-sparql-to-sql-rewriting-strategy.md),
[ADR-0011](ADR-0011-observability-and-configuration.md), [ADR-0015](ADR-0015-datatype-dialect-correctness.md),
[ADR-0038](ADR-0038-sota-application-completion-programme.md), and [ADR-0048](ADR-0048-rust-production-and-node-evidence-runtime-boundary.md).
The Phase 2 profile is [ADR-0051](ADR-0051-postgresql-16-public-observed-schema-profile.md).
Normative companions: [Appendix A](../design/ADR-0050-observed-schema-identity-v1-contract.md) and [Appendix B](../design/ADR-0050-observed-schema-identity-v1-known-answer-vectors.md).
