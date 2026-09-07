---
status: accepted
date: 2026-09-01
updated: 2026-09-07
tags: [rust, node, metaharness, evidence, supervisor, packaging, postgresql]
supersedes: []
depends-on: [ADR-0038]
implements: [ADR-0038]
---

# Rust production and Node evidence runtime boundary

## Status boundary

This ADR is **accepted** by explicit maintainer direction on 2026-09-01. It
fixes the implementation-language, packaging and authority boundary for the
application, coding harness and proposed capture supervisor.

It does not accept ADR-0039, ADR-0041 through ADR-0047, ADR-0052 or ADR-0053,
claim that a complete or production-active Rust supervisor exists, authorize a
database or deployment, or weaken any final correctness, security, performance,
reproducibility or release gate. Existing TypeScript artefacts remain
non-authorizing reference evidence.
ADR-0050 is independently accepted as a lifecycle design, not as a claim that
its incomplete runtime phases or initial PostgreSQL profile are enabled.

## Context

Semantic Fabric is a Rust application. Its product logic, public server and
supported database adapters live in the Cargo workspace. Node is already useful
for Ruflo/MetaHarness orchestration, test generation, mutation, replay and
independent executable oracles.

The proposed supervisor work drifted across that boundary. In particular,
ADR-0042 described `coding-harness/supervisor-service/` as independently
deployable and required future writer, signer and network adapters to remain in
that TypeScript package. ADR-0043 and ADR-0046 then anticipated a pinned Node
`pg` runtime bridge. The package is not operational: it has no runtime
dependencies and every database, network, signer, publication and readiness
flag is false. Calling it deployable would turn evidence infrastructure into a
second production runtime without a reviewed reason.

The canonical `semantic-builder` gold for product-mock, the sealed
`semantic-product-mock` source revision and the narrow mutable ProductDesign/Style
live vertical provide a faster development path; the separately inspected
11-database inventory is red. The gold contains 38,962 ontology, 7,495 shape,
10,167 total mapping and 900 provenance quads across 14 categories. Its Source
Mapping facet declares 134 generic RML TriplesMaps/492 predicate-object maps;
the in-charter qualified development KAT covers 148 relational R2RML TriplesMaps/721
predicate-object maps over all 112 tables/598 columns. It is a
deterministic development oracle, not standards qualification or production
admission.

## Decision

### 1. Keep the product runtime Rust-only

The public application and every product-runtime dependency are Rust/Cargo
artefacts. The production closure must contain no Node executable, npm package,
JavaScript runtime dependency, `node_modules`, `package-lock.json`/npm lock or MetaHarness
component.

ADR-0039's public `semantic-fabric` server remains a product artefact. A future
supervisor is a separate Rust bounded context and separately packaged service;
it is never linked into `sf-server` and never reuses the product query path.
If proposed ADR-0053 is accepted, its selected V1 compiler-containment worker
is likewise a Rust/Cargo product component with a bounded versioned wire
contract. Ruflo, MetaHarness and Node may test that boundary but may not
implement it at runtime.

### 2. Keep all committed Node code non-deployable

`coding-harness/`, including the historically named
`coding-harness/supervisor-service/`, is development/evidence infrastructure.
Its TypeScript modules, bundles, fixtures, scripts and Node 20/24 tests may act
as:

- an executable specification for canonical bytes and state transitions;
- an independent differential oracle for a Rust implementation;
- mutation, replay, security and adversarial evidence; and
- Ruflo/MetaHarness coordination infrastructure.

They may not own a production listener, database pool, credentials, mTLS,
signer/HSM connection, migration execution, persistent authority store or
deployment. Operational manifest flags stay false. No runtime `pg` dependency
or exported database adapter may be added to this tree.

The word `service` in the existing path and protocol identifiers is historical
and does not imply deployability. Package metadata and sealed descriptions must
say `development/evidence oracle` rather than `independently deployable`.

### 3. Isolate any production supervisor as Rust

If ADR-0041/0042 is accepted and activated, its implementation is a new Rust
package/service with no dependency on product `sf-*` crates. It may reuse locked
generic ecosystem crates such as Tokio, Axum, Serde, SHA-2, `tokio-postgres`
and `deadpool-postgres`, but owns distinct:

- writer, recovery and readiness pools;
- database, schema, roles, credentials and migrations;
- fixed parameterized statements with no caller-supplied SQL;
- mTLS/peer authentication and network policy;
- signer/HSM and transparency/witness ports; and
- build, SBOM, attestation, deployment and rollback evidence.

It must not share `sf-serve` pools, source-database credentials, `Backend`,
`sf-sql`, `sf-sparql`, query execution or request authority. Driver APIs that
normalize away required PostgreSQL protocol evidence need an independent
wire-transcript test adapter or narrower protocol component; product driver
convenience is not authority.

### 4. Separate normative contracts from host hardening

Language-neutral SQL, migrations, canonical byte grammars, record schemas,
state transitions, time/deadline semantics, receipts and fixtures remain
normative inputs. JavaScript-specific `Promise`, `WeakMap`, proxy/accessor,
`Uint8Array`, `Buffer` and event-loop defenses qualify the Node oracle only.

The Rust service must reproduce the normative behavior through private Rust
types, ownership and explicit async state, then pass differential vectors
against the Node oracle. Exact Node 20/24 results never substitute for Rust
build, unit/property/mutation, live PostgreSQL, fault, transport and packaging
gates.

### 5. Use the gold corpus without copying its authority

The canonical gold remains in `semantic-builder` under
`docs/reviews/semantic-product-mock-gold-candidate-v0.1.0/artifacts/`, specifically:

- `expected-ontology.json`, the machine-readable bundle;
- `categories/`, the reviewable Turtle split across all 14 categories; and
- `candidate-manifest.json`, the bundle manifest.

Generated `.metaharness` copies are run evidence only. Semantic Fabric records
the manifest/source revision and digests it consumes; it does not fork or
silently refresh the gold. Ordinary CI requires the in-repo seal-policy,
mutation and loader tests but supplies neither external root, so the exact
external KAT remains diagnostic unless a controlled job explicitly provides
both roots. The sealed development source snapshot is the exact committed tree
of `semantic-product-mock` revision
`7c45292fccb8b88afe263e18de6806667ae18573`.

The live PostgreSQL instance is a development integration/differential source.
Tests separately verify every byte in the sealed 171-file development source snapshot
and the live server/version/schema posture. They do not attest the source Git
worktree, OCI image bytes, build process, or a source-to-container provenance
link; SQL observations cannot prove the container was built from that source.
Its operational rows are not gold, and mutable volume, trust authentication,
lack of TLS and unqualified mutable full-inventory state prohibit backend admission or release
claims.

Semantic Fabric continues to support R2RML, not generic RML. The development
adapter unions every sealed Category-13 shard, takes the RDF-reachable closure
from exactly 148 `rr:TriplesMap` roots, rejects reachable generic-RML terms, and
validates 721 predicate-object maps over all 112 tables/598 columns before the
unchanged production R2RML parser. The Style map seeds an end-to-end vertical;
this development authority is not permission to expand the application's
charter, infer mappings, admit a backend or claim full live-inventory conformity.

### 6. Run four lanes in parallel

1. Rust product packaging and deterministic correctness work starts
   immediately and carries M0 application foundation plus M1-M6 progress.
2. The exact gold/live product-mock vertical supplies fast development,
   introspection and differential gates, with an explicit coverage ledger.
3. The Node oracle remains frozen/protected while a separate Rust supervisor is
   implemented only to the extent required by accepted evidence decisions.
4. Controlled runner, transparency, witnesses and deployment attestation run as
   an operational-evidence lane and gate authoritative performance and M7
   release claims.

Lanes 3 and 4 do not block application feature implementation. They still block
claims that require their authority. Harness scores, plans and receipts do not
earn product progress; deterministic application behavior and direct product
tests do.

### 7. Implementation status (through 2026-09-07)

Under accepted ADR-0055, `sf-cli --no-default-features` is the standalone serving
build. Its Cargo graph excludes optional `sf-conformance`/`sf-bench` and their
development-only backend features, without removing SQLite, PostgreSQL, MySQL
or semantic validation. The default `development-tools` feature preserves the
developer commands. The root-specific graph guard and public CLI tests run in
CI, including owned PostgreSQL/MySQL TLS providers. A workspace build is not
proof of this minimal graph because Cargo unifies features. The default CLI
receipt remains developer-profile evidence; neither it nor this split closes
ADR-0055's exact-artifact release bundle. No new server crate or Node runtime is
introduced; proposed ADR-0039 is not accepted by implementing this boundary.

Commit `7c12aa7` enforces the Rust product boundary in protected harness and CI
metadata while preserving the dependency-free Node oracle. Commits `13b8187`,
`8c6181b` and `9b60dc2` add Rust-only development KATs that:

- seal the 38,321-byte candidate manifest and all 139 transitive artifacts;
- verify all 171 sealed development source files plus two required migration pins;
- preserve the exact one-table/two-column R2RML coverage and its explicit gaps;
- qualify only literal-loopback PostgreSQL 16.9 with the expected Style schema;
- compare direct SQL with parse-to-translate-to-execute results inside one
  read-only, repeatable-read transaction and explicitly roll it back.

These KATs passed against the canonical external gold/source roots and the live
development database. They establish neither production backend admission nor
image, build, deployment, TLS, authentication, data-provenance or release
authority. ADR-0042 through ADR-0047 were reviewed against this boundary: their
committed Node code remains explicitly non-deployable oracle evidence, and each
future production implementation is assigned to a separate Rust service.

Commit `a050db3` begins that separate Rust service as the independent
`sf-capture-supervisor` crate. Its bounded transactional kernel and PostgreSQL
store implement immutable exact replay, one lease and attempt, stable overlap
locking with monotonic fences, the closed terminal matrix, atomic pending-outbox
state, post-lock database time, same-primary writer/recovery binding, and
redacted adapter errors. Deterministic in-memory crash-boundary tests and
developer-local isolated PostgreSQL 16.15 differential/contention tests exercise
that slice. It has no product data-plane dependency and does not make Node a
runtime. HTTP/mTLS, principal authentication, signer/materializer, controlled
runner, database role/RLS/operational hardening and restart recovery,
transparency/witness publication, controlled performance, production admission,
and release authority remain absent; ADR-0042 stays proposed.

The native Ruflo reader also remains development-only. Its optional
`SF_HARNESS_RUFLO_PACKAGE_ROOT` is a source locator, not trust: the path must be
absolute and canonical; every non-overlaid selected source plus each protected
replacement must produce the pinned 1,552-file materialized execution closure;
and the value is omitted from the networkless child environment. This permits
an exact sealed cache to remain usable when a shared global Ruflo installation
is intentionally patched, without trusting that patch, mutating the shared
installation or adding a Cargo/product dependency.
The root is resolved once for pre/post source inspection and private-runtime
construction; execution occurs only at `/runtime/package/bin/mcp-server.js`.
Schema V2 retains its original meaning: its global `entryPath` names the
physical source used by historical captures. V2 remains strictly replayable but
is never emitted for relocated-source execution. Schema V3 instead binds
`content-addressed-relocatable-package-root-v1`, the aggregate digest/count/bytes
and the private executed `entryPath`; V2 and V3 identities cannot cross-parse.

The private supervisor oracle's exact Linux CI closure now declares
`@rollup/rollup-linux-x64-gnu@4.63.1` as a direct development dependency because
npm 9.6.4 otherwise omits Rollup's optional native package on Node 20.0.0. Fresh
local exact Node 20.0.0 and 24.14.1 clean installs each pass all 715 tests. Runtime
dependencies remain empty, the dependency-free public bundle is byte-identical,
and this host-specific evidence-tool pin enters neither Cargo nor any product or
production artefact.

The additive evidence-only V3 PUBLIC-ACL replay retains the historical V1/V2
receipts and replay implementations byte-exact and never invokes those runners.
It invokes only `baseline-v1`, `baseline-v2`, `branch` and `final-where`, each
twice in distinct fresh networkless anonymous-volume containers with no ports,
and accepts readiness only after PID 1 is `postgres` and `pg_isready` succeeds.
Hosted run [`33636424967`](https://github.com/sparkling/semantic-fabric/actions/runs/33636424967)
at exact `d0cc5fb938a1ff8b70859c19882934461fe23c5a` passes exact Node 20/24 V3 CI. No hosted run receipt is tracked. This Node work is non-deployable development evidence and grants no runtime or admission.

Commit `cbb63ab` adds a separate development-only table/column inventory gate.
It recounts the sealed 11 stores, 112 tables and 598 columns, then inspects each
explicit live database in its own read-only repeatable-read transaction; it does
not claim one globally atomic snapshot. The 2026-09-01 live run failed closed:
each of ten populated databases had five unexpected infrastructure tables and 45
columns, `Style360` lacked three tables/21 columns, and `ProductDesign` had one
changed column. The gate compares table identity plus ordered column name, type
and nullability only—not keys, constraints, defaults, indexes, views or privileges.
It infers no mapping, mutates no database and grants no production authority.

The 2026-09-05 gold refresh seals the current 63,091-byte manifest and all 246
transitive artifacts, keeps the 171-file source snapshot and two migration pins,
and validates the Category-13 relational closure at 148 TriplesMaps/721
predicate-object maps over 112 tables/598 columns. Its current Style differential
passes in one rolled-back read-only PostgreSQL 16.9 snapshot. A separate optional
KAT sends the same exact `ORDER BY ?styleNumber ?version LIMIT 10001` query and
sealed mapping through `sf-serve` HTTP admission, request control and PostgreSQL
execution: the 2026-09-05 run returned `200` and all 500 typed bindings equalled
the direct SQL rows in order. Its `LIMIT 10002` control returned redacted `501`
before opening a deliberately unreachable PostgreSQL pool. This is a sequential
cross-session mutable observation, not a coherent shared snapshot. The separate
full inventory gate still fails closed on the drift recorded above. None of these
results depends on ignored `.metaharness` output or grants production authority.

Commits `9d228dd` and `67a779a` move neutral schema ownership into `sf-core` and
centralize compiler dialect capabilities without adding Node to Cargo. Commit
`faee07a` adds an enforcing immutable single-source compiler/backend/cache
binding and rejects a foreign bound plan before source I/O. Commit `9d0da85`
then fixes PostgreSQL to one coherent read-only repeatable-read `public`
catalogue snapshot and pins, recycles and verifies the unqualified execution
`search_path`; the follow-up relation-identity guard also rejects a `public`
base table shadowed by an earlier `pg_catalog` relation. Hostile same-name
schema/temp relations and cross-schema foreign keys fail closed for catalogued
base tables. Trusted raw `rr:sqlQuery` remains verbatim and can explicitly name
other schemas, so this is not a public-only SQL sandbox.

Commit `24a0e20` converts each raw serving observation to a
`CompilerSchema` with `ConstraintAuthority::Unverified`. It retains table/column
names, SQL types and estimates, removes PK, UNIQUE, FK, functional-dependency and
NOT-NULL claims, and includes the authority in `CompileScope`; cache hits and
misses therefore cannot use mutable startup constraints to change an answer.
Constraint-driven optimiser passes remain available to explicit frozen-schema
translation/conformance tests, but that capability grants no serving authority.
Duplicate safety stays conservative when keys are quarantined.

Public `sf-serve` startup still accepts authored R2RML only: its mapping-profile
gate rejects every Direct Mapping selection before connector I/O. Behind that
gate, a private Rust-only PostgreSQL foundation now consumes the ADR-0051 rich
snapshot and complete table projection from one owned transaction. It marks the
pool member dirty before `BEGIN`, locks the exact public-table set before the
first repeatable-read snapshot, generates only the primary-key-backed Direct
Mapping candidate, compares its rich identity and database/role/session policy,
then performs a final exact recheck and acknowledged rollback before packaging
the inseparable schema, observation, mapping and request-generation expectation.

For an internal verified request, semantic/resource-shape preflight first
reserves one opaque server-wide compiler permit. The same permit remains held
without requeue while the request acquires and revalidates its generation lease,
then moves into the authoritative compiler worker. Typed executable inventories
derive the exact 34-unit metadata reservation and reject 33 before pool I/O;
required-live SELECT, ASK and CONSTRUCT use
the lease-owned connection; completion rechecks the same generation and rolls
back under a fixed cleanup allowance. If timeout, cancellation, error, drop or
a retained execution view prevents acknowledged rollback, the member stays
dirty, issues a bounded best-effort native cancel and detaches from the pool
rather than recycling an uncertain transaction.

One required-live Rust test provisions an isolated restricted-role PostgreSQL
16.15 database and covers the pre-lock no-snapshot state, clean close,
`ACCESS EXCLUSIVE` exclusion, compatible additive-FK old-generation coherence
plus next-acquisition drift, local policy mutation, cancelled work and dirty
member replacement. This is direct product evidence, not Node authority.
Separately, the tracked exact PostgreSQL 16.9/16.15 pair passes the
observation-profile gate after two fresh runs per patch and independent clean
replay. Compiler type and constraint authorities remain `Unverified`; public
Direct Mapping, no-PK identity, reload/watchers, other backend leases,
federation, production admission and release authority remain absent. ADR-0050
is accepted as a design but remains partially implemented; ADR-0051 is accepted
for observation qualification only.

Commit `824bb74` begins proposed ADR-0053's Rust-only boundary with a fixed-size
parser-worker handshake codec. Later Rust-only slices hold and observe the
current executable, provide a private `x86_64-unknown-linux-gnu`
descriptor-exact launcher with stage-one pre-exec controls, own pidfd/process-
group termination/reap and bind parent-pipe I/O under one spawn deadline. A
hidden first-statement Rust dispatcher exact-matches private two-token
invocations and requires a raw-empty Linux environment.

Commits `c754165`, `56c2236`, `7c87fae` and `3a0199e` retain the exact normal
control-only parser tuple and add the selector-free parser-free normal tuple
against the same held ELF. Integrated evidence commits `e55fccd` and `ce5487e`
add a separately feature-gated same-executable mutant peer and parent; `d103438`
adds both modules to the Node development-harness source inventory, and
`fa9d977` adds live request-EOF ordering. These commits evidence the Rust
boundary, not a
shipped or release-qualified capability.

The normal exchange preserves one immutable deadline and cumulative accounting
through `Hello`/`Ready`, parent write/close of the prepared 96-byte request plus
source, child stack preflight, one complete request allocation/body read, exact
stdin EOF, nonce/digest/UTF-8 validation, and the static
128-byte-header/100-byte-`QueryV1` result. The parent
stack-preflights and prospectively caps before one complete result allocation,
then requires stdout EOF, pidfd waitability, group sweep, exact reap and success
before replay, correlation, digest, decode, direct re-encode and static equality.
Only unit escapes from either hidden evidence seam.

The mutant tuple sends a closed two-byte big-endian directive before `Hello`.
Its ten cases prove exit-zero nonce/source-digest/payload-digest/invalid-
`QueryV1` classification only post-reap; status 78 and deadline precedence over
wrong correlation at 412 accepted output bytes; and trailing-output rejection
before semantics with its extra byte unaccepted. Exact-cap output accepts
`Ready` 184 + header 128 + body 8,388,608 = 8,388,920 bytes before post-reap
invalid-`QueryV1`; cap+1 fails prospectively at 312 bytes before body allocation.
Request-frame allocation refusal means zero result bytes after the required
184-byte `Ready`, followed by EOF, exact reap and raw 78. Live bad nonce, digest
and UTF-8 requests stay alive and silent until EOF, then close output and
raw-exit 78. Every mutant and bad-request probe permits a clean next launch.

The provisional whole-life input cap remains 1,048,856 bytes: normal source
ceiling 1,048,576 and mutant ceiling 1,048,574 after its two directive bytes.
Output remains capped at 8,388,920 bytes, independently of the 67,108,864-byte
`RLIMIT_FSIZE`. The default-kill policy, dependency/profile digest, parser
syscall/randomness surface and dynamic closure remain unqualified; GNU build-ID
comparison is correlation, not release attestation. No parser runs and no
parser-produced wire, qualified parser profile, paired corpus, witness, cache,
admission, serving, release or attestation exists. Parser execution and complete
profile qualification remain next; ADR-0053 stays proposed. Every product/runtime
component here is Rust/Cargo. Node/MetaHarness only preserves or exercises
development evidence and adds no runtime authority or dependency.

## Consequences

- **Positive:** the application retains one production language and dependency
  ecosystem while preserving the substantial TypeScript oracle investment.
- **Positive:** product features can progress in parallel with high-assurance
  evidence infrastructure without relaxing the final gates.
- **Positive:** the gold/live corpus removes synthetic setup work and exposes
  Semantic Fabric's measurable relational-R2RML admission gap without implying
  that the gold's generic RML evidence is absent.
- **Cost:** production supervisor semantics must be implemented independently in
  Rust and checked differentially rather than activated from the prototype.
- **Cost:** high-assurance operational evidence remains a separate deployment
  programme and may outlast application implementation.

## Alternatives rejected

- **Deploy the TypeScript prototype** — adds a second production runtime and
  promotes evidence code whose transport, TLS, signer and operations are absent.
- **Embed the supervisor in `sf-server`** — mixes evidence authority with the
  product query/data plane and shares credentials and failure domains.
- **Discard the Node work** — loses valuable independent fixtures, mutation
  oracles and executable specifications.
- **Treat the gold or live database as release authority** — exceeds its stated
  development purpose and its partial relational mapping coverage.
- **Block all product work on the witnessed supervisor** — confuses evidence
  authority with product behavior and lengthens the critical path without
  increasing feature correctness.

## Links

[ADR-0038](ADR-0038-sota-application-completion-programme.md),
[ADR-0039](ADR-0039-minimal-production-serving-artifact.md),
[ADR-0041](ADR-0041-manifest-bound-controlled-observational-evidence-capture.md),
[ADR-0042](ADR-0042-witnessed-single-use-capture-supervisor-protocol.md),
[ADR-0043](ADR-0043-postgresql-supervisor-registration-state-and-dormant-adapter.md),
[ADR-0044](ADR-0044-postgresql-supervisor-catalogue-contract.md),
[ADR-0045](ADR-0045-canonical-postgresql-supervisor-catalogue-oracle-representation.md),
[ADR-0046](ADR-0046-sealed-postgresql-supervisor-migration-authority-bundle.md),
[ADR-0047](ADR-0047-canonical-postgresql-16-15-public-acl-baseline-projection.md),
[ADR-0050](ADR-0050-verified-source-generation-leases-schema-identity-and-atomic-runtime-activation.md),
[ADR-0051](ADR-0051-postgresql-16-public-observed-schema-profile.md),
[ADR-0052](ADR-0052-sparql-compilation-safety-envelope-and-versioned-logical-work-accounting.md), and
[ADR-0053](ADR-0053-grammar-coupled-sparql-parser-governance-and-process-isolation-fallback.md).
