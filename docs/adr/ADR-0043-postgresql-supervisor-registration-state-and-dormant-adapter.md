---
status: proposed
date: 2026-08-30
updated: 2026-08-30
tags: [postgresql, supervisor, serializable, persistence, outbox, security]
supersedes: []
depends-on: [ADR-0037, ADR-0038, ADR-0039, ADR-0042]
implements: [ADR-0042]
---

# PostgreSQL supervisor registration state and dormant adapter

## Status boundary

This ADR is **proposed**. It defines the first PostgreSQL persistence slice for
ADR-0042, but does not accept that ADR, enable the supervisor, or grant database,
signer, network, publication, witness, runner, capture, import, promotion, or
release authority.

The first implementation remains sealed, build-only, and absent from the public
supervisor bundle. The existing service manifest and readiness result remain
nonoperational with every capability flag false. A driver may be present only as
an exact-pinned development dependency for required-live tests. The runtime
dependency, transport composition root, credentials, TLS, pool, signer, and
deployment belong to a later activation decision and artifact profile.

## Context

The committed registration kernel already provides canonical request parsing,
duplicate-first decision logic, exact joined-row validation, candidate staging,
same-transaction exact reread, literal commit-gated response release, and
disjoint write/recovery capability roots. It deliberately has no database or
materializer.

ADR-0042 requires one linearizable PostgreSQL transaction to own the exact
request, immutable event, run projection, authority counter, exact response, and
publication outbox. The store must also distinguish a true exact-result miss
from damaged provenance, preserve unsigned 64-bit decimal values, and never
interpret a lost commit acknowledgement as an abort.

PostgreSQL SERIALIZABLE can abort a transaction with SQLSTATE 40001.
PostgreSQL requires the application to rerun the complete transaction, including
the logic which chose statements and values. Retrying only COMMIT, a write, or
a previously staged candidate is nonconforming. A deadlock 40P01, or a named
uniqueness/exclusion conflict proven to represent the same race, may use the same
bounded whole-transaction path. All other errors and any connection-loss or
commit ambiguity remain fixed indeterminate outcomes.

The programme needs executable PostgreSQL evidence without weakening the sealed
dependency-free public bundle or pretending that tests provision independently
administered authority.

## Decision

### 1. Add bounded complete-transaction retry before the adapter

The coordinator synchronously snapshots the peer-consumer and checkout root
methods before request hashing can yield, consumes the authenticated peer
exactly once after canonical preclassification, then permits at most three fresh
serializable attempts through those captured roots. Each retry:

1. obtains a new checkout and calls its sole open operation;
2. repeats mapping and exact lookup first;
3. commits that read transaction and returns the exact stored bytes on a hit;
4. only on a miss, repeats head, predecessor, and run reads, reconstructs and
   stages a new candidate from those fresh reads;
5. repeats the same-transaction exact reread; and
6. commits only that attempt.

A known server-declared abort is represented only by an internal,
unforgeable-by-data retry marker; no string, result object, SQL message, or other
adapter data can request retry. A read or staging abort is rolled back
successfully before retry. A commit-time known abort terminally destroys its
connection, and the bound coordinator re-emits the marker only after that
destruction succeeds. Rollback or destruction failure, an exhausted attempt
limit, an unclassified error, or an ambiguous commit returns the existing fixed
500 and is never retried.

This is an internal database retry, not a transport retry. The client sends one
request and receives no intermediate bytes. The peer registry is not re-consumed.

### 2. Keep the adapter driver-free and outside the public bundle

The adapter depends on narrow injected capabilities:

- writer and exact-recovery client acquisition;
- parameterized query execution and explicit connection release/destruction;
- a closure-owned authenticated peer-to-project mapper;
- pinned service/configuration identity; and
- a prepare/sign/finalize materializer.

No PostgreSQL driver type enters a service interface. Adapter source and
migrations are sealed artifact build inputs but are not source inputs of
src/index.ts. Test sources are protected harness-policy inputs and are bound by
the programme's test and review receipts rather than by the deployable artifact
digest. Runtime packages remain empty and the public bundle digest must remain
byte-identical.

Required-live tests use exact-pinned pg and type declarations as development
dependencies. This proves PostgreSQL behavior but does not make the built service
deployable. Adding a runtime driver, exporting an adapter, or setting
databaseAccessEnabled to true requires a separate reviewed activation slice.

### 3. Use a dedicated schema with exact domains

The first migration creates a dedicated owner-controlled schema and exact
domains:

- SHA-256 values are 32-byte bytea;
- canonical bytes are bounded bytea, decoded as fatal UTF-8 and re-encoded
  byte-identically at the adapter boundary;
- protocol sequences use numeric(20,0) constrained to
  0..18446744073709551615; and
- opaque identifiers use the protocol's ASCII length and alphabet.

PostgreSQL bigint, serial, identity columns, sequences, and JavaScript numbers
are not used for semantic event, run, or fence order. Transactional counters
advance only with the transaction and have no committed gaps after abort.

The registration slice owns these minimum tables:

| Table | Purpose |
|---|---|
| schema_migrations | Ordered version and exact script digest; runtime verifies but never applies. |
| authority_configurations | Immutable pinned configuration, project binding, service identity, genesis receipt, and active-head inputs. |
| authority_state | Singleton global semantic head and next sequence; write path locks it first. |
| semantic_events | Immutable event/envelope chain with unique global and per-run order. |
| semantic_receipts | Independently supplied predecessor receipts; supervisor writer has read only. |
| registration_runs | Current run projection with immutable original-registration pointers. |
| registration_results | Exact request/result bytes and hashes, status, and current-event reference. |
| publication_outbox | Exact privacy-minimized public commitment inserted pending in the semantic transaction. |

Composite foreign keys retain project and run scope. Unique constraints prevent
two event positions, two registration results, two changed-replay terminals, or
two outbox payloads for one event. Append-only tables grant no update, delete, or
truncate privilege to runtime roles.

### 4. Preserve exact-first reads and one joined provenance query

An exact lookup starts from registration_results and uses left joins to the
current event, the run's immutable original-registration pointers, and the
immutable original registration event:

- no result root is absent;
- one complete, well-typed, internally consistent joined row is found; and
- an existing root with a missing join, unexpected null, duplicate cardinality,
  malformed field, or failed byte/hash constraint is indeterminate.

Original registration event digest and global sequence come from the immutable
original event, never from mutable registration_runs last-event fields. This is
required for exact 409 recovery after the run has advanced.

No separate current head, receipt, current-run classification, or staging query
is consulted for an exact hit. The run table may appear only inside the single
joined provenance query for its immutable original-registration pointers; those
columns are never inferred from its mutable last-event projection. Recovery uses
a separate read-only pool and role in a serializable read-only transaction. It
cannot acquire write, head, receipt, run-classification, or materialization
capabilities.

### 5. Lock, validate, materialize, and persist in one attempt

After an exact miss, the write transaction:

1. locks the singleton authority state and active configuration;
2. validates the exact project and asserted authority head;
3. reads the required genesis or semantic predecessor receipt;
4. locks the run row, with the authority lock protecting absent-run creation;
5. revalidates the complete candidate against the locked snapshots;
6. prepares immutable signing bytes from pinned service identity;
7. signs only those bytes and verifies the returned signature under the pinned
   public key;
8. inserts the event, exact result, run transition, and pending outbox;
9. advances the authority state with an expected-old-state predicate;
10. rereads the exact joined row in the same transaction; and
11. releases response bytes only after a literal acknowledged commit.

Every statement is fixed, schema-qualified, and parameterized. Request data
never selects an identifier, relation, role, policy, or SQL fragment. Every
affected-row count is exact.

The materializer has three capabilities:

- prepare(candidate, locked snapshots) performs closed-schema equality checks
  and constructs exact signing bytes;
- sign(bytes) returns signature bytes only; and
- finalize(prepared, signature) verifies the signature and constructs the exact
  envelope, response, state head, public leaf, and database row values.

Database rollback does not roll back an HSM. A post-sign/pre-commit orphan has no
event, result, outbox, or recovery path and is never returned. A real HSM
invocation journal and reconciliation protocol are deferred to activation.

### 6. Separate ownership and runtime roles

Use distinct no-login owner/migrator roles, one immutable no-login database scope
role per project, and narrowly privileged login roles:

- schema owner;
- deployment-only migrator;
- one registration writer per project scope;
- one exact-recovery reader per project scope;
- future receipt ingestor;
- future outbox publisher; and
- sanitized monitor.

Runtime roles are not owners, superusers, role members of owners/migrators, or
BYPASSRLS. Each runtime login is a member of exactly one immutable project scope
role. Project rows bind that scope role, and row-security policies authorize only
when the nonspoofable session_user is a member of the bound role. Custom
transaction settings may provide redundant equality inputs but are never an
authorization source: a connected role can select arbitrary custom-setting
values. Revoke public schema, object, and default privileges. Enable and force
row security on every project-scoped table with both read and write policies.
Grant no DDL, membership administration, role switching beyond the one bound
scope, temporary-object creation, sequence mutation, COPY, TRUNCATE, or broad
function execution.

The writer cannot create semantic receipts or update or delete immutable events,
results, or outbox payloads. The recovery role can select only the mapping and
exact joined-row inputs. No security-definer function is introduced.

### 7. Version migrations explicitly

Migration files and a canonical digest manifest are protected build inputs.
Deployment applies them with the migrator role in strict order and records their
exact SHA-256 values. Runtime startup only verifies the exact supported set.
Missing versions, gaps, future versions, checksum drift, partial application, or
ownership, policy, or grant drift keep readiness false.

Migrations are never automatically applied by the service.

## Acceptance gates for this slice

The dormant adapter slice is complete only when:

1. retry tests inject known aborts at every decision, stage, exact-reread, and
   commit boundary and prove full recomputation, one peer consumption, bounded
   attempts, and no ambiguous-commit retry;
2. fake-client tests prove allocation-free checkout, sole acquisition in open,
   parameterized fixed SQL, exact call ordering, row-count checks, cleanup, and
   connection destruction;
3. row-codec mutations distinguish absence from damaged joined provenance and
   cover uint64 minimum, maximum, noncanonical, negative, fractional, and
   overflow values;
4. materializer KATs and mutations pass the already committed event and result
   validators for 201 and 409;
5. migrations apply once to an empty PostgreSQL 16 database, serialize concurrent
   migrators, and reject digest drift, gaps, future versions, partial failure,
   wrong ownership, policy, and privileges;
6. required-live tests cover 201, exact replay, changed-replay 409, post-close
   recovery, concurrent identical and different requests, rollback, known abort,
   commit ambiguity, connection cleanup, project isolation, direct-login
   cross-project custom-setting attacks, and role denial;
7. event, result, run, head, and outbox cardinalities prove atomicity after every
   injected fault;
8. service and parent suites, audits, hardened builds, protected registries,
   historical anchors, and Node 20 and 24 byte-determinism gates remain green;
   and
9. the public export set, runtime package set, readiness result, manifest flags,
   and public bundle digest remain byte-identical.

A real PostgreSQL process restart, promotion or failover, and post-WAL-flush
network fault remain later controlled-environment gates. Backend termination is
a test substitute and must not be reported as ADR-0042 acceptance gate 4.

## Consequences

### Positive

- The existing product and semantic compiler architecture do not change.
- PostgreSQL semantics become executable behind the verified coordinator.
- Exact recovery remains independent of current authority head and run state.
- Database races have one bounded, documented, whole-transaction retry path.
- Public bundle and runtime dependency closure remain frozen.

### Negative

- The test package gains an exact-pinned PostgreSQL development dependency.
- Migrations, row codecs, materialization, privileges, and live concurrency add a
  substantial independently sealed surface.
- Safety-first ambiguity may permanently spend availability and require exact
  recovery or operator intervention.
- HSM orphan reconciliation and operational activation remain separate work.

### Neutral

- Until a semantic witness supplies the next predecessor receipt, a fresh
  witnessless fixture can commit at most one event. Tests for later events must
  explicitly seed a nonauthorizing receipt fixture; this is not operational
  multi-event evidence.
- Pending outbox rows prove atomic staging only. No publisher or log exists.
- ADR-0042 remains proposed and all records remain
  development-only-no-promotion with positive authority flags false.

## Alternatives rejected

- **Rely on a global advisory lock instead of retry** — serializable failures may
  arise from dependencies outside that lock; PostgreSQL still requires complete
  transaction retry.
- **Retry COMMIT, a write, or the staged candidate** — reuses decisions from an
  invalid snapshot and can append incorrect bytes.
- **Treat every error as retryable** — can repeat persistent corruption, signer
  faults, permission errors, or ambiguous commits.
- **Use bigint, identity, or sequences** — cannot represent protocol uint64 and
  produces nontransactional gaps.
- **Use inner joins for exact recovery** — damaged provenance becomes a false
  miss and could authorize another append.
- **Add pg as a runtime dependency now** — changes the operational artifact
  before the transport, signer, attestation, and deployment gates exist.
- **Reuse the Rust product PostgreSQL lane** — crosses the independently
  deployable supervisor boundary and would be an architecture rewrite.
- **Auto-migrate at startup** — gives a runtime principal DDL authority and makes
  deployment drift harder to attest.

## Primary references

- [PostgreSQL serialization failure handling](https://www.postgresql.org/docs/18/mvcc-serialization-failure-handling.html)
- [PostgreSQL transaction isolation](https://www.postgresql.org/docs/18/transaction-iso.html)
- [PostgreSQL explicit locking](https://www.postgresql.org/docs/17/explicit-locking.html)
- [PostgreSQL row security policies](https://www.postgresql.org/docs/17/ddl-rowsecurity.html)
- [ADR-0037](ADR-0037-dual-host-ruflo-engineering-metaharness.md)
- [ADR-0038](ADR-0038-sota-application-completion-programme.md)
- [ADR-0039](ADR-0039-minimal-production-serving-artifact.md)
- [ADR-0042](ADR-0042-witnessed-single-use-capture-supervisor-protocol.md)
