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

The canonical semantic-product-mock gold and its exact-revision PostgreSQL
instance also provide a faster development path, but their authority and
coverage must remain honest. The gold candidate contains 30,696 ontology,
3,617 shape, 4,501 mapping and 900 provenance quads across 14 categories. Its
relational inventory covers 112 tables and 598 columns, while its admitted
relational R2RML currently covers only one table and two columns. It is a
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

The canonical gold remains in `semantic-builder` at
`docs/reviews/semantic-product-mock-gold-candidate-v0.1.0/artifacts/`.
Generated `.metaharness` copies are run evidence only. Semantic Fabric records
the manifest/source revision and digests it consumes; it does not fork or
silently refresh the gold.

The live PostgreSQL instance is a development integration/differential source.
Tests separately verify an exact source checkout and database image/schema;
SQL observations cannot prove the container was built from that source. Its
operational rows are not gold, and mutable volume, trust authentication, lack
of TLS and partial R2RML coverage prohibit backend admission or release claims.

Semantic Fabric continues to support R2RML, not generic RML. The one admitted
relational R2RML map may seed an end-to-end vertical slice. Coverage of the
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

## Consequences

- **Positive:** the application retains one production language and dependency
  ecosystem while preserving the substantial TypeScript oracle investment.
- **Positive:** product features can progress in parallel with high-assurance
  evidence infrastructure without relaxing the final gates.
- **Positive:** the gold/live corpus removes synthetic setup work and exposes a
  measurable upstream R2RML coverage gap.
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
