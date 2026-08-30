---
status: proposed
date: 2026-08-30
updated: 2026-08-30
tags: [postgresql, supervisor, catalog, migrations, rls, least-privilege]
supersedes: []
depends-on: [ADR-0038, ADR-0039, ADR-0042, ADR-0043]
implements: [ADR-0043]
---

# PostgreSQL supervisor catalogue contract

## Status boundary

This ADR is **proposed**. It refines ADR-0043's dormant PostgreSQL slice; it
does not activate the supervisor, accept ADR-0042/0043, provision credentials,
or grant database, signer, network, publication, witness, runner, capture,
import, promotion, or release authority.

The exact-row prepare/finalize materializer is implemented and protected as a
private build input outside the public bundle. Migrations, catalogue verifier,
adapter, deployment identity, TLS, credentials, pools, HSM reconciliation,
receipt ingestion, outbox delivery, and activation remain later work.

## Context

ADR-0043 fixed the topology, transaction order, eight minimum tables, role
separation, RLS rule, and exact catalogue verification requirement. Independent
architecture, security, and test reviews found four gaps that prevent an exact
migration from being reviewable:

1. the materializer had no finalized database-row contract;
2. the tables had no exact columns, constraints, indexes, ACL matrix, or policy
   inventory;
3. neither the startup-verifier principal nor the external database bootstrap
   boundary was named; and
4. `COPY` is not a revocable PostgreSQL object privilege, while raw catalogue
   OIDs, ACL order, and expression trees are not replayable semantic identities.

Generating SQL before closing those gaps would make the database, rather than a
reviewed repository artifact, the source of truth.

## Decision

### 1. Finalize rows before generating the catalogue

Implementation order is fixed:

1. this catalogue contract;
2. sealed prepare/finalize materialization with exact database-row types;
3. reviewed exact catalogue JSON with expanded identifiers, ordered key/FK
   tuples, match modes, and literal expressions;
4. immutable migration SQL and raw-byte manifest derived from that contract;
5. driver-free migration runner and catalogue verifier;
6. dormant transaction adapter; and
7. exact-pinned PostgreSQL 16.15 required-live evidence.

Prepare exact-key checks both roots before traversal and snapshots a decision candidate plus the full locked configuration, authority-state, predecessor-receipt, and keyed absent/full run rows before its first await.
Each graph rejects proxies, accessors, cycles, and excess beyond 32 levels, 8,192 nodes, or 1,048,576 cumulative byte-leaf bytes; wide arrays fail before key enumeration and wide records before descriptor expansion.
It closes every deterministic equality before constructing the exact signing payload under `semantic-fabric/programme-capture/supervisor-run-event-signing-v2`, then returns a
module-private WeakMap-backed one-use identity plus an independent signing-byte copy.
Finalize consumes that identity before parsing the exact 64-byte signature, rederives the private snapshots, verifies the locked 44-byte canonical SPKI/fingerprint and signature,
then returns all rows. No caller can inject an envelope, response, resulting state head, public
leaf, database value, clock, random value, retry marker, or project-selected key;
the run-genesis prior head remains a caller assertion carried as unverified.

Finalize computes the post-event controller state head as SHA-256 of canonical
bytes for:

```text
{
  domain: "semantic-fabric/programme-capture/supervisor-controller-state-head-v2",
  priorControllerStateHeadDigest,
  eventDigest
}
```

It preserves the locked authority-configuration head, advances only the event-chain
last/next sequence and last-event fields, creates the exact status-specific result and
privacy-minimized ADR-0042 public leaf, and returns independent DB-shaped expected-old
snapshots: all nine authority-state fields, all eleven 409 run fields, or the keyed absent run.
The stored public commitment digest is the raw SHA-256 of those exact leaf
bytes. All byte hashes are raw SHA-256; no timestamp, generated identifier,
sequence object, or database-derived default enters a semantic row.

### 2. Pin the deployment identities and bootstrap boundary

The dedicated schema is `sf_supervisor_v1`. The nonoperational fixture and
catalogue contract use these literal roles:

| Role | Login | Purpose |
|---|---:|---|
| `sf_supervisor_owner_v1` | no | Owns the database, schema, domains, and tables. |
| `sf_supervisor_migration_login_v1` | yes | Deployment-only migration connection. |
| `sf_supervisor_readiness_login_v1` | yes | Reads migration facts, seed state, and system catalogues only. |
| `sf_supervisor_readiness_capability_v1` | no | Readiness identity marker; zero object ACLs. |
| `sf_supervisor_writer_capability_v1` | no | Writer identity marker; zero object ACLs. |
| `sf_supervisor_recovery_capability_v1` | no | Recovery identity marker; zero object ACLs. |
| `sf_supervisor_project_scope_v1` | no | One-database project marker; zero object ACLs. |
| `sf_supervisor_writer_login_v1` | yes | Direct data-plane writer. |
| `sf_supervisor_recovery_login_v1` | yes | Direct exact-result recovery reader. |

Every role is `NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION
NOBYPASSRLS NOINHERIT`, has connection limit `-1`, no validity limit, and no
role/database settings. The four marker roles and owner are `NOLOGIN`.

The exact membership edges are:

| Granted role | Member | ADMIN | INHERIT | SET |
|---|---|---:|---:|---:|
| owner | migration login | false | false | true |
| project scope | readiness login | false | false | false |
| readiness capability | readiness login | false | false | false |
| project scope | writer login | false | false | false |
| writer capability | writer login | false | false | false |
| project scope | recovery login | false | false | false |
| recovery capability | recovery login | false | false | false |

Role names in the table expand to their literal `sf_supervisor_*_v1` names.
There are no other incident or transitive membership edges. The provisioning
manifest pins the external bootstrap grantor because PostgreSQL records that
cluster identity on each membership edge; migrations neither create roles nor
learn a grantor from observed state.

The owner is the database owner. Before migration, the external bootstrap
administrator revokes all database privileges from `PUBLIC`, grants direct
`CONNECT` only to the four login roles, grants no `TEMP`, and revokes all
privileges on the `public` schema from `PUBLIC`. The provisioning contract pins
the resulting `public`-schema owner and ACL. No service step relies on implicit
`pg_database_owner` membership or privileges, and migration SQL never alters the
`public` schema. The migration transaction switches locally to the owner and
creates or revokes privileges only in the dedicated schema. Credentials,
`pg_hba.conf`, and TLS are deployment inputs.
The verifier models the database owner's implicit `pg_database_owner` relation
separately from `pg_auth_members` and checks database-wide service ownership.

The readiness login is the only non-owner principal with `SELECT` on
`schema_migrations`. It has schema `USAGE`, direct membership in only the
project-scope and readiness-capability markers, and column `SELECT` on only the
configuration and singleton state needed to replay the seed. Writer and
recovery paths never borrow this connection or gain its capability.

`COPY TO` requires `SELECT` and applies SELECT RLS policies. PostgreSQL 16.15
rejects `COPY FROM` on an RLS-enabled table rather than applying INSERT policy.
The adapter issues neither form; live tests prove both behaviours. The verifier
rejects membership in `pg_read_server_files`, `pg_write_server_files`, and
`pg_execute_server_program`, and runtime roles have no server-file/program, DDL,
temporary-object, sequence, truncate, reference, trigger, or role-switch
authority.
The `COPY FROM` RLS claim applies to project tables. On non-RLS
`schema_migrations`, readiness is denied by its missing INSERT ACL; the
deployment-only owner path is never used for `COPY`.

### 3. Use exact domains

The owner creates these domains in `sf_supervisor_v1`; each constraint has the
domain name plus `_check_v1` as its stable name:

| Domain | Base and exact constraint |
|---|---|
| `sha256_digest_v1` | `bytea`; exactly 32 bytes and not all zero. |
| `uint64_v1` | unconstrained `numeric`; `scale(VALUE)=0` and `0 <= VALUE <= 18446744073709551615`. |
| `opaque_id_v1` | `text COLLATE pg_catalog."C"`; 8..128 ASCII bytes matching `[A-Za-z0-9_-]+`. |
| `project_scope_role_v1` | `text COLLATE pg_catalog."C"`; exactly `sf_supervisor_project_scope_v1`. |
| `configuration_bytes_v2` | `bytea`; 1..131072 bytes. |
| `request_bytes_v2` | `bytea`; 1..32768 bytes. |
| `event_envelope_bytes_v2` | `bytea`; 1..65536 bytes. |
| `registration_result_bytes_v2` | `bytea`; 1..196608 bytes. |
| `public_commitment_bytes_v2` | `bytea`; 1..1024 bytes. |
| `ed25519_spki_der_v1` | `bytea`; exactly 44 bytes with DER prefix `302a300506032b6570032100`. |

All semantic constants use `text COLLATE pg_catalog."C"` with named column
checks. Runtime codecs convert digests to/from bytea and uint64 values to/from
canonical unsigned decimal text. The database never uses bigint, serial,
identity, or a sequence for semantic order.

Adapter and verifier SQL never casts a parameter to a private domain. Parameters
use base PostgreSQL text, bytea, numeric-text, boolean, integer, or smallint and
let target-column assignment apply the domain check. Every selected domain is
explicitly cast to its base type (`::bytea` or `::text`); uint64 and
`response_status` are projected as text. This prevents driver domain-OID parsing
from changing the fixed row-codec representation.

### 4. Freeze the eight-table row contract

All names below are literal. `scope` means leading columns
`project_authority_digest sha256_digest_v1` and
`project_scope_role project_scope_role_v1`. Every project primary, unique, and
foreign key includes both columns, except the database-wide
`UNIQUE(singleton_key)` that enforces exactly one `TRUE` state row. Primary keys end `_pk_v1`, unique constraints
`_uq_v1`, foreign keys `_fk_v1`, checks `_check_v1`, and any partial unique
index `_uidx_v1`; the committed catalogue JSON enumerates every expanded name.
Every listed column is `NOT NULL` unless it is explicitly marked `NULL`.
`singleton_key` is additionally checked `IS TRUE`; domain checks are never
relied on to reject a nullable column.

| Table | Exact columns (`schema_migrations` alone has no `scope`) |
|---|---|
| `schema_migrations` | `migration_version integer`, `script_sha256 sha256_digest_v1`, `catalog_contract_sha256 sha256_digest_v1`, `authority_seed_sha256 sha256_digest_v1`; PK version, positive-version check. This table is database-global, non-RLS, and exempt from every scope rule. |
| `authority_configurations` | `configuration_epoch uint64_v1`, `configuration_digest sha256_digest_v1`, `genesis_authority_head_digest sha256_digest_v1`, `serialized_configuration configuration_bytes_v2`, `serialized_configuration_sha256 sha256_digest_v1`, `project_principal_id opaque_id_v1`, `project_authentication_policy_digest sha256_digest_v1`, `service_principal_id opaque_id_v1`, `service_key_epoch uint64_v1`, `service_key_fingerprint sha256_digest_v1`, `service_signing_spki_der ed25519_spki_der_v1`, `genesis_semantic_receipt_digest sha256_digest_v1`; PK epoch; unique configuration digest, epoch+digest+genesis-head, and epoch+digest+genesis-receipt tuples; M0 epoch fixed to zero; service-key epoch positive. |
| `authority_state` | `singleton_key boolean`, `active_configuration_epoch uint64_v1`, `active_configuration_digest sha256_digest_v1`, `authority_head_digest sha256_digest_v1`, `last_global_sequence uint64_v1`, `next_global_sequence uint64_v1`, `last_event_digest sha256_digest_v1 NULL`; PK scope, database-wide unique singleton key, FK active epoch+digest+authority head to configuration, deferred FK last sequence+event, and exact genesis/successor event-chain checks. The seed supplies literal `true`; there is no column default. |
| `semantic_events` | `event_digest sha256_digest_v1`, `event_kind text C`, `semantic_request_digest sha256_digest_v1`, `run_id opaque_id_v1`, `authority_configuration_epoch uint64_v1`, `authority_configuration_digest sha256_digest_v1`, `authority_head_digest sha256_digest_v1`, `global_sequence uint64_v1`, `run_sequence uint64_v1`, `previous_global_kind text C`, `previous_global_sequence uint64_v1 NULL`, `previous_global_event_digest sha256_digest_v1 NULL`, `previous_global_genesis_configuration_epoch uint64_v1 NULL`, `previous_global_genesis_configuration_digest sha256_digest_v1 NULL`, `previous_global_genesis_receipt_digest sha256_digest_v1 NULL`, `previous_global_event_receipt_digest sha256_digest_v1 NULL`, `previous_run_kind text C`, `previous_run_sequence uint64_v1 NULL`, `previous_run_global_sequence uint64_v1 NULL`, `previous_run_event_digest sha256_digest_v1 NULL`, `prior_controller_state_head_digest sha256_digest_v1`, `resulting_controller_state_head_digest sha256_digest_v1`, `serialized_envelope event_envelope_bytes_v2`, `serialized_envelope_sha256 sha256_digest_v1`; PK event; unique global position/reference, run position, full run snapshot including resulting state head, and semantic-request provenance; deferred FKs to configuration, run, global predecessor, full prior-run snapshot/head, genesis configuration/receipt, and semantic-event receipt; `global_sequence >= 1` plus exact literal-kind, adjacency, state-head, and nullability checks. |
| `semantic_receipts` | `event_digest sha256_digest_v1`, `semantic_receipt_digest sha256_digest_v1`; PK event, unique receipt and event+receipt tuple, FK event. Genesis is configuration-owned; this table contains verified semantic-event predecessor digests only. |
| `registration_runs` | `run_id opaque_id_v1`, `original_registration_request_digest sha256_digest_v1`, `original_registration_request_sha256 sha256_digest_v1`, `original_registration_event_digest sha256_digest_v1`, `last_run_event_digest sha256_digest_v1`, `last_run_global_sequence uint64_v1`, `current_controller_state_head_digest sha256_digest_v1`, `last_run_sequence uint64_v1`, `first_changed_replay_request_digest sha256_digest_v1 NULL`; PK run; unique original request/event provenance; deferred original-event, full last-event snapshot/state-head, and original/first-result FKs; exact open-sequence-0 versus changed-replay-terminal-sequence-1 checks. |
| `registration_results` | `semantic_request_digest sha256_digest_v1`, `run_id opaque_id_v1`, `original_registration_request_digest sha256_digest_v1`, `original_registration_request_sha256 sha256_digest_v1`, `original_registration_event_digest sha256_digest_v1`, `serialized_request request_bytes_v2`, `serialized_request_sha256 sha256_digest_v1`, `response_status smallint`, `response_content_type text C`, `serialized_response registration_result_bytes_v2`, `serialized_response_sha256 sha256_digest_v1`, `current_event_digest sha256_digest_v1`; PK request; unique current event, run+request, and run+status; deferred immutable-run-provenance and current-event FKs; status-specific 201 equality/409 inequality checks; exact content type `application/json; charset=utf-8`. |
| `publication_outbox` | `event_digest sha256_digest_v1`, `public_commitment_leaf_bytes public_commitment_bytes_v2`, `public_commitment_digest sha256_digest_v1`, `publication_state text C`; PK event, unique commitment, FK event, state fixed to `pending`. |

Every project table except `authority_configurations` and `authority_state` has
a composite scope FK to the singleton `authority_state`; active state has the
reciprocal deferred last-sequence/event FK. Configuration and state are seeded together, and the deployment
verifier requires every configuration row to have exactly the singleton scope.
The event/run circular references are `DEFERRABLE INITIALLY DEFERRED` so one
atomic transaction can create the first pair. No other table, view, sequence,
function, procedure, user trigger, rule, partition, extension, or owner-created
object is allowed in the schema.

Every FK with a nullable source component is `MATCH SIMPLE`; named checks require
all-null or all-present tuples. The state last-event FK is the sole exception:
its check permits exactly sequence-zero/null genesis or positive/non-null event.
The status-specific circular FKs make the run's original request/hash/event
resolve to its 201 result and its non-null first changed request resolve to its
409 result. Each result repeats and FKs the run's immutable original provenance;
its current request/event FKs the matching semantic event. Controller heads are
per-run, never global: each last snapshot/head and non-genesis prior head is in
the full prior-run FK. Genesis initial head is materializer-validated;
`previous_global` binds order/receipt only. State and every M0 event FK the
configuration epoch/digest/genesis head, which registration never changes.
Cross-wired rows therefore cannot commit.

M0 admits exactly `claim-registered-v2` at run sequence zero and
`capture-run-terminal-v2` at run sequence one. A global-sequence-one row has
`previous_global_kind = 'authority-genesis'`, null prior event/sequence and
event-receipt columns, and a complete configuration epoch/digest/genesis-receipt
tuple that FKs the locked configuration. A later global row has
`previous_global_kind = 'semantic-event'`, null genesis-reference columns, an
adjacent predecessor sequence/event, and an event-receipt digest that FKs the
matching `semantic_receipts` row. Likewise, run sequence zero has
`previous_run_kind = 'run-genesis'` and all prior-run snapshot columns null;
run sequence one has `previous_run_kind = 'run-event'` and a complete adjacent
run/global/event snapshot. The materializer selects the genesis or event receipt
only when constructing the protocol envelope; SQL never accepts a caller-
supplied discriminator.

### 5. Freeze direct ACLs and the policy inventory

Marker roles receive zero object ACLs. Runtime grants are column-level; an
unlisted command or column has no ACL and no admitting policy.

| Table | Writer | Recovery | Readiness | Migration owner |
|---|---|---|---|---|
| `schema_migrations` | none | none | SELECT all | SELECT/INSERT all |
| `authority_configurations` | SELECT all | none | SELECT all | SELECT all |
| `authority_state` | SELECT all; UPDATE last/next sequence and last event | none | SELECT scope, singleton, active epoch/digest, authority head | SELECT the same seed-identity columns |
| `semantic_events` | SELECT exact-join columns; INSERT all | SELECT exact-join columns | none | none |
| `semantic_receipts` | SELECT all | none | none | none |
| `registration_runs` | SELECT/INSERT all; UPDATE last event/global/run sequence, state head, first changed digest | SELECT scope, run ID, original request/event pointers | none | none |
| `registration_results` | SELECT/INSERT all | SELECT all | none | none |
| `publication_outbox` | INSERT all | none | none | none |

The migration-owner column describes post-RLS commands the runner exercises, not
an ACL ceiling: seed INSERTs precede policies, and ownership is broader. Its power
remains behind the deployment login's sole `SET TRUE` edge and absent runtime credential.

The event exact-join SELECT grant set is scope, event digest/kind, semantic
request digest, global sequence, prior controller state head,
serialized envelope, and envelope SHA-256. Every exact-result join predicate
compares both project-scope columns. Its fixed 27-column projection deliberately
omits the repeated `project_scope_role` values while returning each joined
project digest for provenance; the role equality is enforced by the join
predicates and the fixed-literal domain, not inferred by the codec.
`response_status::text AS result_response_status_text` is mandatory, and the
ordered aliases in catalogue JSON must equal the committed `RAW_ROW_KEYS`
exactly, with no missing or extra key. The project
tables have exactly 19 admitted principal/command combinations and 38 policies:
one permissive and one
restrictive policy for each pair. There are no dummy policies for denied
commands.

Policy names are `<principal>_<command>_permit_v1` and
`<principal>_<command>_scope_v1`, where principal is `writer`, `recovery`,
`readiness`, or `migration_owner`. Data-plane/readiness policies target the
corresponding direct login, never `PUBLIC` or a marker role. Migration-owner
policies target the no-login owner and additionally require
`CURRENT_USER = 'sf_supervisor_owner_v1'` and
`SESSION_USER = 'sf_supervisor_migration_login_v1'`. SELECT uses `USING`, INSERT
uses `WITH CHECK`, and UPDATE repeats the same expression in both clauses. The
permissive expression requires the literal scope-role value. The restrictive
expression repeats that equality and requires:

```sql
pg_catalog.pg_has_role(
  SESSION_USER, '<literal capability role>', 'MEMBER'
)
AND pg_catalog.pg_has_role(
  SESSION_USER, project_scope_role, 'MEMBER'
)
```

The preceding `pg_has_role` expression applies only to writer, recovery, and
readiness. The migration-owner restrictive expression is literally the two
`CURRENT_USER`/`SESSION_USER` equalities above plus
`project_scope_role = 'sf_supervisor_project_scope_v1'`; it performs no marker
membership lookup. The migration login's sole `SET TRUE` edge is verified before
role switch.
The `project_scope_role_v1` domain admits only the one provisioned literal, so a
stored row cannot make `pg_has_role` resolve an attacker-selected or nonexistent
role name; the catalogue verifier proves that literal role and its graph exist
before readiness succeeds.
The singleton and composite FKs, not a caller-settable setting, bind the one
project digest. The closure-owned mapper separately requires the request and
database project digest to equal its pinned deployment value. Every project
table has row security enabled and forced. The owner seeds configuration/state
before policies are installed; its persistent policies are read-only and cover
only exact seed replay.

Schema `USAGE` is granted directly to writer, recovery, and readiness; none has
schema `CREATE` or domain/type `USAGE` grants. Owner default privileges revoke
the built-in `PUBLIC` routine `EXECUTE` and type `USAGE` defaults; only those two
classes produce expected default-ACL rows. Table, sequence, and schema classes
expect absent default-ACL rows, while explicit object/column ACLs and grantors
remain exact. The verifier rejects an unnecessary explicit row where absence is
canonical.

### 6. Make migration and verification replayable

The sealed migration set contains:

- `migrations/0001-registration-state-v1.sql` for the dedicated schema, default
  ACLs, domains, tables, named constraints, and dedicated-schema/object
  revocations; `public`-schema hardening remains an external bootstrap fact;
- one parameterized, closed deployment seed between scripts for the canonical
  configuration, key, project binding, genesis receipt/head, and singleton
  state; it is not request data or a migration file;
- `migrations/0002-registration-rls-v1.sql` for direct grants, policies, and
  enable/force RLS;
- `migrations/provisioning-contract-v1.json` for the password-free database,
  owner, schema, role attributes, bootstrap grantor, membership edges/options,
  database ACL, `public`-schema ACL, forbidden predefined-role contract, and
  attested absence of migration credentials from runtime profiles;
- `migrations/manifest-v1.json` with schema version, PostgreSQL server version
  `16.15`, advisory-lock key `800874507948546278`, catalogue-contract digest,
  provisioning-contract digest, and contiguous ordered entries containing
  version, path, byte length, and raw SHA-256; and
- `migrations/catalog-contract-v1.json`, with a normalized database-catalogue oracle compared by readiness and an exact-query alias section compared by build KAT to `RAW_ROW_KEYS`.

The authority-seed preimage is duplicate-free exact pretty JSON plus LF. Its
ordered root keys are `domain`, `schemaVersion`, `authorityConfiguration`, and
`authorityStateIdentity`; domain is
`semantic-fabric/programme-capture/supervisor-postgresql-authority-seed-v1` and
schema version is `1`. The two nested objects use the table-column order above;
state identity contains only scope, literal singleton, active epoch/digest, and
stable authority head. Digests are lowercase hex, uint64s decimal strings,
bytea unpadded base64url, and the singleton a JSON boolean. Mutable last/next
event-chain fields are excluded; empty-apply verification separately requires
zero, one, and null. `authority_seed_sha256` is raw SHA-256 of these exact bytes.

The deployment connection verifies its direct migration principal, sole
membership, and `server_version_num = 160015` before any DDL. A mismatch destroys
the connection and leaves the database empty. It then executes `BEGIN ISOLATION
LEVEL READ COMMITTED READ WRITE`, immediately sets/verifies
`search_path=pg_catalog` and `row_security=on`, verifies isolation, read-write state, and
`session_replication_role=origin`, then takes the fixed
transaction advisory lock, and uses `SET LOCAL ROLE sf_supervisor_owner_v1`.
Every migration identifier, type, function, and operator reference is schema-
qualified. An empty database applies both
scripts and the seed in one transaction and records both externally computed
script digests plus the final contract and canonical seed digests in
`schema_migrations`. After FORCE RLS, the two migration-owner SELECT policy
pairs re-read the actual configuration/state rows under the same lock. An exact
database executes no DDL. Any other migration set, stored or recomputed seed
mismatch, digest drift,
gap, future version, partial set, or catalogue mismatch rolls back and fails;
the dormant v1 runner does not infer or repair an upgrade path.

Only an exact PostgreSQL `COMMIT` acknowledgement is success. Loss or malformed
completion after sending `COMMIT` destroys the connection, returns unknown, and
is never retried in that invocation. A fresh invocation reclassifies from
committed facts: an acknowledged prior commit performs no DDL, while a rolled-
back empty database applies once. Atomic transactions leave no valid partial
classification.

Catalogue verification runs fixed queries in one `SERIALIZABLE READ ONLY
DEFERRABLE` transaction that sets/verifies `search_path=pg_catalog`,
`row_security=on`, fixed timeouts, and
closed-schema row codecs. It uses `pg_catalog`, expands ACLs with `aclexplode`,
resolves OIDs to qualified stable names, sorts set-like records, rejects
duplicates, and compares complete arrays to committed canonical JSON. It covers
server/database identity, schema, domains, relations, columns, constraints,
indexes, RLS flags, policies, object/column/default ACLs, service-role
attributes, complete incident membership edges and grantors, and absence of
unexpected service ownership or inbound grants. Normalized FK-trigger linkage
and `tgenabled='O'`, cluster-wide `pg_parameter_acl`, tablespace privileges, and
effective `CONNECT` to every database are included. M0 live evidence requires a
dedicated cluster whose provisioning oracle closes every service principal's
database-wide and cluster-wide authority.
The readiness principal also recomputes the canonical seed digest from the two
narrowly readable tables and compares it to the supplied sealed seed and every
`schema_migrations` row.

Before `BEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY DEFERRABLE`, readiness
verifies its exact direct login, attributes, incident/transitive graph, and
`server_version_num = 160015`. Wrong identity or version destroys the client. Any absent,
gapped, future, malformed, duplicate, thrown, or drifted migration/catalogue/
provisioning/seed result rolls back and releases after acknowledged rollback;
transaction-control or connection uncertainty destroys instead. Only exact
read-only commit acknowledgement releases a ready result.

The oracle excludes OIDs, regclass numbers, tuple/transaction identifiers,
relfilenodes, TOAST identities, statistics, freeze horizons, raw ACL order, raw
`pg_node_tree`, password hashes, timestamps, physical paths, activity state,
and generated FK-trigger OIDs/names (their constraint linkage, type, and enabled
state remain included). Expressions are obtained with fixed
`pretty=false` deparsing under `search_path=pg_catalog`; M0 readiness is valid
only on the exact tested PostgreSQL 16.15 lane until cross-minor KATs establish
a wider support set. The verifier emits a digest receipt but never regenerates
or overwrites its oracle from observed state.

## Acceptance gates

This contract is implemented only when:

1. prepare/finalize and independent seed KATs on both Node lanes reproduce every
   event, signature, result, state, public-leaf, row, seed byte, and digest;
2. manifest/provisioning/catalogue tests reject malformed bytes, traversal,
   reorder/gap/future/duplicate entries, mutation, drift, substitution, any
   non-expanded tuple/expression, or identifier longer than 63 bytes;
3. fake migration/readiness tests prove exact entry, order, hostile-search-path
   resistance, replication check, rollback/commit acknowledge-versus-uncertain
   release/destroy branches, no escaped ready result, and no partial repair;
4. catalogue/data mutations cover every missing/extra object and inbound grant,
   disabled/relinked FK trigger, parameter ACL, other-database access, mandatory
   null, partial/mixed predecessor tuple, wrong receipt, and independently valid
   result/run/current/original-event cross-wire;
5. PostgreSQL 16.15 live tests prove exact-version preflight plus provider-free
   mismatch rejection; empty apply and replay; positive readiness/owner seed
   replay; 201/409 writer and recovery query rows passed directly to the
   untouched 27-key decoder; external-admin receipt/cardinality fixtures;
   post-commit acknowledgement suppression followed by no-DDL replay and paired
   pre-send termination followed by one apply; concurrency, scoped `FOR UPDATE`
   success with forbidden column-update denial, `COPY`, and least-authority
   denials; otherwise-valid deferred-constraint mutants failing
   at `COMMIT` with zero rows; and every non-deferred ADR-0043 fault outcome; and
6. Node 20.0.0 and 24.14.1 suites, protected registries, hardened builds,
   security review, public exports, empty runtime dependencies, nonoperational
   readiness/manifest flags, and public bundle bytes remain unchanged.

## Consequences

### Positive

- Migration SQL is derived from a closed row contract rather than guessed.
- Runtime, recovery, readiness, migration, owner, capability, and scope powers
  are independently auditable.
- Exact catalogue drift has a stable semantic projection without treating OIDs
  or ACL ordering as identity.
- Dynamic project configuration is seeded atomically without a persistent owner
  write bypass.

### Negative

- The initial migration path is intentionally strict and supports only empty or
  exact databases, not in-place upgrades.
- PostgreSQL 16.15 is narrower than major-version compatibility.
- Thirty-eight policies and column ACLs increase migration and verifier size.
- Receipt ingestion and outbox completion require later roles and migrations.
- Extending a run beyond the fixed registration/changed-replay sequences zero
  and one requires a reviewed forward migration and catalogue-contract version.

### Neutral

- The separate readiness login/capability is a new least-privilege composition boundary,
  not a data-plane or product-architecture change.
- `COPY TO` remains governed by SELECT ACL/RLS; PostgreSQL 16 rejects
  `COPY FROM` on the forced-RLS project tables.

## Alternatives rejected

- **Generate the oracle from the live database** — makes compromised observed
  state authoritative.
- **Let recovery verify migrations** — violates its exact-result-only grant.
- **Grant catalogue reads through a security-definer function** — adds an
  owner-rights code surface and function ACL/default-ACL risk.
- **Target policies to non-inheriting marker roles** — the direct login would
  not receive the required permissive policy.
- **Add policies for forbidden commands** — converts default denial into
  unnecessary authority.
- **Hard-code a project digest in generic SQL or trust a custom setting** —
  makes migration bytes deployment-specific or caller-selectable.

## Primary references

- [ADR-0038](ADR-0038-sota-application-completion-programme.md) and [ADR-0039](ADR-0039-minimal-production-serving-artifact.md)
- [ADR-0042](ADR-0042-witnessed-single-use-capture-supervisor-protocol.md) and [ADR-0043](ADR-0043-postgresql-supervisor-registration-state-and-dormant-adapter.md)
- [PostgreSQL 16 row security](https://www.postgresql.org/docs/16/ddl-rowsecurity.html) and [`CREATE POLICY`](https://www.postgresql.org/docs/16/sql-createpolicy.html)
- [PostgreSQL 16 role grants](https://www.postgresql.org/docs/16/sql-grant.html)
- [PostgreSQL 16 `pg_auth_members`](https://www.postgresql.org/docs/16/catalog-pg-auth-members.html)
- [PostgreSQL 16 privileges](https://www.postgresql.org/docs/16/ddl-priv.html) and [default privileges](https://www.postgresql.org/docs/16/sql-alterdefaultprivileges.html)
- [PostgreSQL 16 `COPY`](https://www.postgresql.org/docs/16/sql-copy.html)
- [PostgreSQL 16 system catalogues](https://www.postgresql.org/docs/16/catalogs.html)
