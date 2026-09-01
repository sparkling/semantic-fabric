---
status: accepted
date: 2026-09-01
updated: 2026-09-01
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

It does not accept ADR-0039 or ADR-0041 through ADR-0047, claim that a Rust
supervisor exists, authorize a database or deployment, or weaken any final
correctness, security, performance, reproducibility or release gate. Existing
TypeScript artefacts remain non-authorizing reference evidence.

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
11-database inventory is red. The gold contains 30,696 ontology, 3,617 shape,
4,501 total mapping and 900 provenance quads across 14 categories. Its Source
Mapping facet declares 134 generic RML TriplesMaps/492 predicate-object maps;
the in-charter qualified development KAT slice covers only one table/two columns. It is a
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
lack of TLS and partial R2RML coverage prohibit backend admission or release
claims.

Semantic Fabric continues to support R2RML, not generic RML. The one in-charter
qualified development R2RML map may seed an end-to-end vertical slice. Coverage of the
remaining relational schema is an upstream mapping workstream, not permission
to expand this application's charter or infer mappings.

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

### 7. Implementation status (2026-09-01)

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

Commit `cbb63ab` adds a separate development-only table/column inventory gate.
It recounts the sealed 11 stores, 112 tables and 598 columns, then inspects each
explicit live database in its own read-only repeatable-read transaction; it does
not claim one globally atomic snapshot. The 2026-09-01 live run failed closed:
each of ten populated databases had five unexpected infrastructure tables and 45
columns, `Style360` lacked three tables/21 columns, and `ProductDesign` had one
changed column. The gate compares table identity plus ordered column name, type
and nullability only—not keys, constraints, defaults, indexes, views or privileges.
It infers no mapping, mutates no database and grants no production authority.

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

Current `sf-serve` loads authored R2RML before opening the backend and does not
generate Direct Mapping. The Direct Mapping utility and conformance runners use
explicit frozen fixture schemas. Because PK/FK facts determine the generated
mapping itself, any future live Direct-Mapping path must bind mapping generation
and the entire streamed execution to one verified source generation; removing
optimiser facts after generation would be insufficient. These changes close the
later-DDL integrity-constraint wrong-answer path, but do not provide structural/
type schema digests or drift detection, atomic reload, a verified-constraint
lease, federation, production admission or release authority.

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
[ADR-0046](ADR-0046-sealed-postgresql-supervisor-migration-authority-bundle.md), and
[ADR-0047](ADR-0047-canonical-postgresql-16-15-public-acl-baseline-projection.md).
