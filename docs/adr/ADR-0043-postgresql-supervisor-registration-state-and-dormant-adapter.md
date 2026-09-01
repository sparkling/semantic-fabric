---
status: proposed
date: 2026-08-30
updated: 2026-09-01
tags: [postgresql, supervisor, serializable, persistence, outbox, security]
supersedes: []
depends-on: [ADR-0037, ADR-0038, ADR-0039, ADR-0042, ADR-0048]
implements: [ADR-0042]
---

# PostgreSQL supervisor registration state and dormant adapter

## Status boundary

This ADR is **proposed**. Its `implements` edge records only a subordinate
proposed design relationship; while either ADR is proposed it is neither shipped
implementation nor capability evidence. It defines the first PostgreSQL
persistence slice for ADR-0042, but does not accept that ADR, enable the
supervisor, or grant database, signer, network, publication, witness, runner,
capture, import, promotion, or release authority.

The retry/coordinator, row-codec, and prepare/finalize materializer slices are
implemented as sealed Node reference-oracle inputs absent from the public
bundle. The manifest and readiness result remain nonoperational with every
capability flag false. Node retains only provider-free fixed vectors, fakes and
mutation evidence. Migrations and language-neutral contracts may feed a future
Rust implementation; the live Rust adapter, transport root, credentials, TLS,
pool, signer and deployment do not exist.

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
a previously staged candidate is nonconforming. The first adapter's retry
allowlist is exactly serialization failure 40001 and deadlock 40P01 when they
come from a trusted driver error caught at a query or commit boundary. No
uniqueness, exclusion, message, result value, or plain object can request retry.
All other errors and any connection-loss or commit ambiguity remain fixed
indeterminate outcomes.

Node tests model the marker bridge only with fixed provider-free trusted-error
vectors and fakes; no driver, driver prototype or driver-bearing bridge enters
the Node evidence package. Production Rust must independently classify the
corresponding trusted `tokio-postgres` error/`SqlState` at the awaited driver
boundary. Plain code/message data, foreign errors, every 23505, and mapper,
materializer, signer or cleanup failures cannot request retry.

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
- a signer which receives exact signing bytes and returns signature bytes only.

Prepare and finalize are sealed local functions, not injectable capabilities.
Prepare exact-key checks both roots before traversal, snapshots all deterministic
inputs before its first await, and bounds each closed graph to 32 levels, 8,192
nodes, and 1,048,576 cumulative byte-leaf bytes. Wide arrays fail before key
enumeration; wide records fail before descriptor expansion; proxies, accessors,
cycles, and excess fail closed. It returns a
module-private WeakMap-backed, one-use identity with only an independent signing-
byte copy exposed. Finalize consumes that identity before signature parsing,
rederives the private snapshots, and verifies the returned Ed25519 signature
against the pinned public key before constructing any row value.

No PostgreSQL driver type enters a normative service interface. Node adapter
source is protected oracle input; language-neutral migrations are reusable by
the Rust service. Neither is a source input of `src/index.ts`. Test sources are
bound by harness receipts rather than a production-artifact digest. The Node
oracle's package dependencies remain empty and its public bundle stays byte-identical.

Node evidence dependencies remain empty. Live PostgreSQL apply, concurrency,
cleanup, role-denial and fault evidence belongs to Rust/`tokio-postgres` tests.
Production activation requires the separate Rust service, its own
pools/roles/TLS/signer ports, differential vectors, live tests and reviewed
deployment evidence under ADR-0048.

### 3. Use one project namespace per database and exact domains

The first Rust service/database instance admits exactly one immutable project
authority digest and one project scope role. The authenticated peer mapper is a
closure-owned capability; it performs no database lookup and its result must
equal that pinned database project. The database does not multiplex project
namespaces. A future multi-project global chain, sharded authority head, or
shared login pool changes ordering and authorization semantics and requires a
separate ADR.

The authority state is one singleton row for that database project. Every
project-scoped row still carries the project authority digest and scope-role
name. Every project-scoped primary key, foreign key, join predicate, and
expected-old-state update includes both columns. RLS policies bind the literal
scope role; the singleton and composite foreign keys bind the project digest.
This redundancy makes cross-wiring and foreign-row injection detectable rather
than turning the single-project topology into implicit ambient authority.

The first migration creates a dedicated owner-controlled schema and exact
domains:

- SHA-256 values are 32-byte bytea;
- canonical bytes are bounded bytea, decoded as fatal UTF-8 and re-encoded
  byte-identically at the adapter boundary;
- protocol sequences use unconstrained numeric with `scale(VALUE) = 0` and a
  range constraint of 0..18446744073709551615, preventing PostgreSQL from
  rounding fractional input before the constraint observes it; and
- opaque identifiers use the protocol's ASCII length and alphabet.

All numeric values cross the adapter boundary as canonical unsigned decimal
strings and are selected with an explicit text cast. On writes, JavaScript number
and bigint values, exponent notation, signs, whitespace, leading zeroes, and
fractional lexical forms are rejected before a query is issued. On reads, the
row codec applies the same lexical checks to selected text and treats malformed
database values as damage rather than absence.

All semantic ASCII identifiers and exact textual constants use
`text COLLATE pg_catalog."C"` (or a domain with that collation); canonical JSON
and digest-bearing protocol payloads remain bounded bytea. Default or
nondeterministic collation cannot participate in identity, uniqueness, joins,
or ordering.

PostgreSQL bigint, serial, identity columns, sequences, and JavaScript numbers
are not used for semantic event, run, or fence order. Transactional counters
advance only with the transaction and have no committed gaps after abort.

The registration slice owns these minimum tables:

| Table | Purpose |
|---|---|
| schema_migrations | Ordered version and exact script digest; runtime verifies but never applies. |
| authority_configurations | Immutable canonical configuration bytes, project binding, service identity, exact Ed25519 SPKI DER, genesis receipt, and active-head inputs. |
| authority_state | Singleton project semantic head and next sequence; write path locks it first. |
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

One fresh writer attempt has this externally observable order:

1. acquire a new direct-login client and verify its exact principal, attributes,
   and membership graph;
2. issue `BEGIN ISOLATION LEVEL SERIALIZABLE READ WRITE`;
3. issue fixed `SET LOCAL` statements, in order, for search path, 5-second lock
   timeout, 30-second statement timeout, 30-second idle-in-transaction timeout,
   `synchronous_commit=on`, and `row_security=on`, then query and verify all six
   actual settings plus isolation, read-only state, and
   `session_replication_role=origin`;
4. map the authenticated peer and perform the first joined exact lookup;
5. on a miss, lock the singleton authority state and snapshot-read its active
   configuration under that lock;
6. validate the exact project and asserted authority head;
7. read the required genesis or semantic predecessor receipt;
8. lock the run row, with the authority lock and named uniqueness constraints
   protecting absent-run creation;
9. prepare immutable signing bytes, sign only those bytes, verify the signature,
   and independently finalize the complete candidate;
10. insert the event, exact result, run transition, and immutable pending outbox;
11. advance only the event-chain last/next sequence and last-event fields with
    an exact expected-old-state predicate; the authority-configuration head is
    unchanged by a registration event;
12. map the same authenticated peer again and perform the sole post-stage joined
    exact reread, which must observe the transaction's uncommitted rows;
13. commit, release the client, and only then release the exact reread response.

The coordinator's `stageCandidate` call owns steps 9--11; its two decision phases
own the two mapper/exact reads. An exact hit performs steps 1--4 and 13 only.
The recovery path uses a fresh direct-login client and exactly `acquire -> verify
principal and membership -> BEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY -> fixed SET LOCAL ->
verify transaction state -> map -> joined exact lookup -> COMMIT -> release`.

Every data-bearing statement is fixed and schema-qualified, and every data value
is parameterized. Transaction-control, local-setting, and state-check statements
are exact fixed literals with no request-derived token. Request data never
selects an identifier, relation, role, policy, or SQL fragment. Every affected-
row count is exact.

Each transaction checks its actual isolation and read-only state, uses a fixed
safe search path, requires `synchronous_commit=on`, and pins `lock_timeout` to
5 seconds plus `statement_timeout` and `idle_in_transaction_session_timeout` to
30 seconds. An activation must additionally attest `fsync=on`. Only the literal
driver acknowledgement for PostgreSQL `COMMIT` is committed. A timeout,
connection loss, malformed completion, or other uncertainty after commit is
sent is unknown: destroy the client, never retry, and release no response bytes.

The materialization boundary has three operations:

- sealed local prepare(candidate, locked snapshots) snapshots before its first await,
  closes every deterministic equality, and returns a one-use identity plus copied signing bytes;
- injected sign(bytes) receives one fresh byte copy and returns signature bytes only; and
- sealed local finalize(prepared, signature) consumes the identity before parsing a copied
  signature, rederives and verifies it, then constructs the exact envelope, response,
  state head, public leaf, database rows, and full DB-shaped expected-old mutations.

The active configuration stores exact canonical configuration bytes and the
exact Ed25519 SubjectPublicKeyInfo DER bytes. The configuration bytes must parse
canonically and reproduce the protocol's domain-separated configuration digest;
the raw SPKI DER SHA-256 must equal the pinned key fingerprint. The materializer
never accepts a project-selected key, database client, retry marker, clock,
random source, or network capability.

Database rollback does not roll back an HSM. A post-sign/pre-commit orphan has no
event, result, outbox, or recovery path and is never returned. A real HSM
invocation journal and reconciliation protocol are deferred to activation.

### 6. Separate ownership and runtime roles

Use distinct no-login owner and capability roles, one immutable no-login database
scope role for the admitted project, one migration-only login, and narrowly
privileged runtime logins:

- schema owner;
- deployment-only migration login;
- fixed readiness, registration-writer, and exact-recovery capability roles;
- one immutable project-scope role;
- one direct readiness login bound only to the readiness capability and project
  scope roles;
- one direct registration-writer login and one direct exact-recovery login for
  the project, each bound to its capability and project scope roles;
- future receipt ingestor;
- future outbox publisher; and
- sanitized monitor.

The nonoperational live-test fixture pins the literal roles
`sf_supervisor_owner_v1`, `sf_supervisor_migration_login_v1`,
`sf_supervisor_readiness_login_v1`,
`sf_supervisor_readiness_capability_v1`,
`sf_supervisor_writer_capability_v1`, `sf_supervisor_recovery_capability_v1`,
`sf_supervisor_project_scope_v1`, `sf_supervisor_writer_login_v1`, and
`sf_supervisor_recovery_login_v1`. A later activation profile must seal any
deployment-specific names and their derivation; the dormant migration never
constructs identifiers from request data.

ADR-0044 refines this proposal with the exact M0 schema, finalized row contract,
direct column ACL and 38-policy inventory, separate readiness login/capability, bootstrap
boundary, seed point, raw-byte migration manifest, and normalized catalogue
oracle. Where this ADR states a broad policy rule, ADR-0044's least-authority
table/command inventory is the exact interpretation: denied commands receive no
dummy policy.

Runtime roles are not owners, superusers, members of the owner role, or
BYPASSRLS. The scope and capability roles are identity-only and have zero object
privileges. Each runtime login has disjoint direct object grants and direct
membership in exactly one project scope role plus its one capability role, with
`ADMIN FALSE`, `INHERIT FALSE`, and `SET FALSE` on both edges. Runtime role
switching is entirely forbidden. Writer and recovery use distinct pools; every
new client verifies exact `session_user = current_user`, role attributes, direct
membership edges, and the complete transitive graph before beginning a
transaction. Shared generic logins and `SET SESSION AUTHORIZATION` are forbidden.
Project rows bind the scope-role name. Each admitted command has one minimal
permissive policy and one restrictive policy; the restrictive expression requires
both membership in the corresponding literal readiness, writer, or recovery
capability role via `pg_has_role(..., 'MEMBER')` and
`pg_has_role(session_user, project_scope_role, 'MEMBER')` in every clause the
command supports: SELECT uses `USING`, INSERT uses `WITH CHECK`, and UPDATE uses
both. M0 admits no DELETE command or DELETE policy. ADR-0044 separately pins the
no-login owner's two read-only
seed-verification policy pairs. The catalog verifier rejects missing or extra policies,
including any additional `PUBLIC` permissive policy. Custom
transaction settings may provide redundant equality inputs but are never an
authorization source: a connected role can select arbitrary custom-setting
values. The external bootstrap revokes `PUBLIC` privileges on the database and
`public` schema; migrations revoke public object and default privileges only in
the dedicated schema. Enable and force row security on every project-scoped
table, with policy pairs only for the commands admitted by ADR-0044's exact
matrix.
Grant no DDL, membership administration, runtime role switching, temporary-
object creation, sequence mutation, TRUNCATE, or broad function execution. The
adapter never issues `COPY`; PostgreSQL 16.15 applies SELECT ACL/RLS to `COPY TO`
and rejects `COPY FROM` on an RLS-enabled table, while server-file/program
predefined roles remain forbidden.

The writer cannot create semantic receipts or update or delete immutable events,
results, or outbox payloads; its updates are column-limited to the run projection
and authority state. The recovery role can select only exact joined-row inputs;
mapping remains closure-owned. No owner-rights view or security-definer function
is introduced.

### 7. Version migrations explicitly

Migration files and a canonical raw-byte digest manifest are protected build
inputs. Role and login provisioning is a separate deployment input: migrations
never create activation credentials or dynamic project roles. Deployment takes
a fixed transaction-scoped advisory lock before schema creation. The canonical
fixture uses a non-superuser migration login whose sole membership is the
no-login schema owner with `ADMIN FALSE`, `INHERIT FALSE`, and `SET TRUE`; it
connects directly, enters the migration transaction, takes the lock, and uses
`SET LOCAL ROLE` to create owner-owned objects. Runtime logins have no membership
in either role. The migration login applies scripts in strict order and records
their externally computed exact SHA-256 values in the same transaction. Runtime
startup only verifies the
exact supported set and the exact catalog owners, domains, constraints, indexes,
policies, ACLs, default ACLs, and membership shape.
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
   overflow values plus collation-equivalent but byte-distinct identifiers;
4. materializer KATs and mutations pass the already committed event and result
   validators for 201 and 409, independently verify the exact
   `supervisor-run-event-signing-v2` canonical payload and Ed25519 signature
   under the pinned SPKI, reject wrong key/bytes/signature, and bind the exact
   ADR-0042 public-leaf bytes and digest;
5. a deployment-only Rust migrator applies once to an empty PostgreSQL 16.15 database, serializes concurrent
   migrators, and reject digest drift, gaps, future versions, partial failure,
   wrong ownership, policy, and privileges;
6. required-live Rust tests cover 201, exact replay, changed-replay 409, post-close
   recovery, rollback, known abort, connection cleanup, and role denial. Two
   identical concurrent requests return identical stored bytes with one semantic
   row set; two distinct request bodies yield one semantic 201 winner and one
   fixed receipt-pending 503 after complete-transaction retry, with no second
   event or outbox. After the winner's predecessor receipt is independently
   seeded, replaying the loser commits and returns the semantic changed-replay
   409. A wrapper
   that completes real `COMMIT` then suppresses its acknowledgement yields fixed
   500 with no retry, after which exact recovery returns the committed bytes and
   one atomic row set; a paired pre-send termination leaves zero rows. Project
   isolation includes a positive same-project direct-login case plus rejection of
   foreign mapper/row/custom-setting inputs. Provider-free provisioning mutations
   prove an additional project login or membership edge keeps readiness false;
   there is never a two-project-in-one-database fixture;
7. Rust event, result, run, head, and outbox cardinalities prove atomicity after every
   injected fault;
8. service and parent suites, audits, hardened builds, protected registries,
   historical anchors, and required Node 20.0.0 and Node 24.14.1
   byte-determinism lanes remain green;
   and
9. the public export set, runtime package set, readiness result, manifest flags,
   and public bundle digest remain byte-identical.

A real PostgreSQL process restart, promotion or failover, and post-WAL-flush
network fault remain later controlled-environment gates. Backend termination is
a test substitute and must not be reported as ADR-0042 acceptance gate 4.

## Consequences

### Positive

- The existing product and semantic compiler architecture do not change.
- PostgreSQL semantics would become executable only after its acceptance gates pass.
- Exact recovery remains independent of current authority head and run state.
- Database races have one bounded, documented, whole-transaction retry path.
- Public bundle and runtime dependency closure remain frozen.

### Negative

- Node evidence packages remain dependency-free; live PostgreSQL belongs to Rust
  tests and a future separately packaged supervisor, never `sf-serve` or the
  Semantic Fabric product runtime.
- Rust migrations, row codecs, materialization, privileges, and live concurrency add a
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
- A sealed external test-cluster administrator may insert an independently
  computed predecessor-receipt fixture and read post-transaction cardinalities.
  It is outside the service role graph, never enters application code, and is
  recorded as nonauthorizing test setup rather than receipt-ingestion evidence.
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
- **Add `pg` anywhere in the Node package** — violates the evidence-only Node boundary.
- **Reuse `sf-serve` pools or query execution** — crosses product and evidence
  credentials, authority and failure domains. A separate Rust supervisor using
  generic Rust PostgreSQL crates is the selected boundary, not a product rewrite.
- **Auto-migrate at startup** — gives a runtime principal DDL authority and makes
  deployment drift harder to attest.

## Primary references

- [PostgreSQL 16 serialization failure handling](https://www.postgresql.org/docs/16/mvcc-serialization-failure-handling.html)
- [PostgreSQL 16 transaction isolation](https://www.postgresql.org/docs/16/transaction-iso.html)
- [PostgreSQL 16 explicit locking](https://www.postgresql.org/docs/16/explicit-locking.html)
- [PostgreSQL 16 row security policies](https://www.postgresql.org/docs/16/ddl-rowsecurity.html)
- [ADR-0037](ADR-0037-dual-host-ruflo-engineering-metaharness.md)
- [ADR-0038](ADR-0038-sota-application-completion-programme.md)
- [ADR-0039](ADR-0039-minimal-production-serving-artifact.md)
- [ADR-0042](ADR-0042-witnessed-single-use-capture-supervisor-protocol.md)
- [ADR-0044](ADR-0044-postgresql-supervisor-catalogue-contract.md)
- [ADR-0048](ADR-0048-rust-production-and-node-evidence-runtime-boundary.md)
