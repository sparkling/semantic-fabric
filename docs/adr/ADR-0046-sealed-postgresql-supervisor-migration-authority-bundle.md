---
status: proposed
date: 2026-08-30
updated: 2026-08-31
tags: [postgresql, supervisor, migrations, provisioning, canonical-json]
supersedes: []
depends-on: [ADR-0038, ADR-0039, ADR-0042, ADR-0043, ADR-0044, ADR-0045]
implements: [ADR-0044, ADR-0045]
---

# Sealed PostgreSQL supervisor migration authority bundle

## Status boundary

This ADR is **proposed**. It closes provisioning, manifest, seed, empty-state and migration-runner
representation gaps left by ADR-0044/0045. It does not accept them, activate the supervisor,
provision credentials, contact PostgreSQL, grant runtime access, or make a production deployment.

The bundle remains private and dormant. Runtime startup/readiness may verify but never apply or
repair migrations. Five public exports, false authority/readiness, empty dependencies and bytes stay unchanged.

## Context

The reviewed catalogue/parser provide exact dedicated-schema truth. Three independent audits found
five accidental-authority risks: input schemas/limits, independent byte pinning, empty/ledger order,
commit versus cleanup uncertainty, and exhaustive predefined-role closure.

These are representation closures inside the architecture, not reasons to replace it.

## Decision

### 1. Keep four disjoint authorities

The committed bundle contains exactly these files under
`coding-harness/supervisor-service/migrations/`:

```text
0001-registration-state-v1.sql
0002-registration-rls-v1.sql
catalog-contract-v1.json
provisioning-contract-v1.json
manifest-v1.json
```

The parameterized authority seed is a sixth sealed in-memory input, not a migration file, request,
SQL fragment, or observation. One reviewed nonoperational seed is private source bytes derived from
canonical fixtures; the manifest binds domain, length and raw SHA-256. Callers cannot substitute it.
Activation requires a deployment-specific reviewed seed, manifest pin, artifact rebuild and evidence.

Catalogue alone owns dedicated-schema truth; provisioning owns database, roles, membership and
cluster-wide negative authority; manifest selects versions/bytes. Readiness may record observations
but never update authority. SQL is generated only from reviewed data, then committed/executed as
sealed bytes; observed state is never generator input.

### 2. Use one hostile-boundary grammar for the three smaller JSON inputs

Provisioning, manifest and seed use exact `JSON.stringify(value, null, 2)` plus LF: fatal UTF-8 with
no BOM, CR, trailing data, alternate whitespace, unsafe/negative-zero/non-finite numbers, unpaired
surrogates, sparse arrays or duplicate decoded keys. All member order is normative.

Each parser performs: intrinsic non-proxy `Uint8Array` admission; synchronous copy; fixed pre-decode
ceiling; fatal UTF-8 decode/re-encode; bounded duplicate/token scan; one `JSON.parse`; closed-schema,
semantic and cross-reference validation; byte replay; raw SHA-256; private brand. File limits never
raise compiled ceilings.

Mutable bytes live in module-private storage; unconstructable prepared handles return fresh copies.
No parser error contains input bytes, SQL, parameters, credentials, or connection strings.

Exact hard ceilings are:

| Input | Bytes | Depth | Nodes | Records | Array width | Object keys | String bytes |
|---|---:|---:|---:|---:|---:|---:|---:|
| provisioning | 65,536 | 8 | 2,048 | 512 | 64 | 16 | 4,096 |
| manifest | 16,384 | 5 | 256 | 16 | 2 | 11 | 1,024 |
| seed | 262,144 | 4 | 56 | 3 | 0 | 14 | 196,608 |

Root depth is one; objects, arrays and primitives are nodes, not keys. Records are non-root objects.
Seed width zero makes 57 nodes structurally valid, so its 56 ceiling has an isolated plus-one case.
Exact maximum and maximum-plus-one cases are required.

### 3. Freeze the provisioning contract

The provisioning domain is
`semantic-fabric/programme-capture/supervisor-postgresql-provisioning-contract-v1`
and `schemaVersion` is the JSON number `1`. Root keys are exactly:

```text
domain
schemaVersion
authority
readinessAuthorized
database
dedicatedSchema
publicSchema
roles
memberships
clusterGuards
runtimeCredentialAbsence
limits
```

`authority` is `none`; `readinessAuthorized` is false. `limits` repeats the
provisioning row above with keys `maximumBytes, maximumDepth, maximumNodes,
maximumRecords, maximumCollectionWidth, maximumObjectKeys,
maximumStringBytes, maximumIdentifierBytes`; the final value is `63`.

The nonoperational profile fixes database/schema `sf_supervisor_v1`, owner
`sf_supervisor_owner_v1`, bootstrap grantor `sf_supervisor_bootstrap_admin_v1`, and public schema
`public` owned by `pg_database_owner`.

Record key order is:

| Record | Keys |
|---|---|
| database | `name, ownerRole, aclState, privileges` |
| dedicated schema | `name, ownerRole` |
| public schema | `name, ownerRole, aclState, privileges` |
| role | `name, login, superuser, inherit, createRole, createDatabase, replication, bypassRls, connectionLimit, validUntil, settings` |
| membership | `grantedRole, memberRole, grantorRole, adminOption, inheritOption, setOption` |
| privilege | `grantorRole, granteeKind, granteeRole, privilege, grantable` |
| cluster guards | `forbiddenExplicitMembershipRolePrefix, implicitDatabaseOwnerRole, parameterPrivileges, tablespacePrivileges, systemPrivilegeBaseline, databasePrivilegeRule, serviceOwnershipRule, effectiveObjectPrivilegeRule, defaultAclRule, inboundGrantRule` |
| system baseline | `profile, recordCount, recordsBytes, recordsSha256` |
| runtime absence | `migrationRole, runtimeProfileIds, evidenceRule` |

Identifiers are 1..63 ASCII bytes. `aclState` is `explicit`. Privilege atoms use
`granteeKind=role|public`; PUBLIC has null `granteeRole`. Privileges are
`CONNECT|CREATE|TEMPORARY|USAGE`. Set identities are role `(name)`, membership
`(grantedRole,memberRole,grantorRole,adminOption,inheritOption,setOption)`, and
privilege `(grantorRole,granteeKind,granteeRole,privilege,grantable)`. Profile
IDs and settings compare by their string value. Every set is strictly increasing
under ADR-0045's typed comparator: unsigned ASCII strings, `null` before strings,
`false` before `true`, and tuple fields left-to-right. Duplicates fail.

The nine role records are the ADR-0044 roles, sorted by literal name. Every
boolean attribute is false except `login` for migration, readiness, writer and
recovery logins. Every role has `NOINHERIT`, connection limit `-1`, null
validity, and `settings=[]`; this requires absence of both global role settings
and every role-in-database setting.

The seven membership records are:

| Granted | Member | SET |
|---|---|---:|
| `sf_supervisor_owner_v1` | `sf_supervisor_migration_login_v1` | true |
| `sf_supervisor_project_scope_v1` | `sf_supervisor_readiness_login_v1` | false |
| `sf_supervisor_project_scope_v1` | `sf_supervisor_recovery_login_v1` | false |
| `sf_supervisor_project_scope_v1` | `sf_supervisor_writer_login_v1` | false |
| `sf_supervisor_readiness_capability_v1` | `sf_supervisor_readiness_login_v1` | false |
| `sf_supervisor_recovery_capability_v1` | `sf_supervisor_recovery_login_v1` | false |
| `sf_supervisor_writer_capability_v1` | `sf_supervisor_writer_login_v1` | false |

Every edge has the fixed bootstrap grantor and false ADMIN and INHERIT options.
There are no other incident or transitive edges.

Every database atom has grantor `sf_supervisor_owner_v1`, `granteeKind=role`,
and `grantable=false`: direct `CONNECT` for migration login; effective owner
`CONNECT`, `CREATE`, `TEMPORARY`; then direct `CONNECT` for readiness, recovery,
and writer. The public-schema `CREATE`, `USAGE` owner atoms instead have grantor
and grantee `pg_database_owner`, kind `role`, and `grantable=false`. There are no
PUBLIC atoms; raw-null, explicit-empty, and explicit states differ.

`clusterGuards` fixes prefix `pg_`, implicit role `pg_database_owner`, empty parameter/tablespace
arrays, and a system-baseline record. Its profile is
`postgresql-16.15-clean-template0-public-object-acl-v1`; count/bytes are positive safe integers and
`recordsSha256` is lowercase nonzero raw SHA-256. Its five rule strings are
`exact-database-privileges-v1`,
`database-and-dedicated-schema-only-v1`, `system-baseline-or-dedicated-schema-only-v1`,
`catalogue-default-acls-only-v1`, and
`no-inbound-grants-outside-dedicated-schema-v1`. Their exhaustive meaning is:

- across every database, effective `CONNECT|CREATE|TEMPORARY` expands PUBLIC,
  ownership and the bounded membership graph for all nine roles. In the named
  database the owner has all three and four logins only `CONNECT`; elsewhere all are empty;
- explicit `pg_parameter_acl` `SET|ALTER SYSTEM` grant atoms for all nine roles
  and PUBLIC are empty; effective tablespace `CREATE`, including PUBLIC and
  membership paths, is empty;
- service ownership is limited to the owner role's named database, dedicated
  schema, catalogue objects, and catalogue-derived array/composite/TOAST/index
  objects; the `public` schema's predefined owner is the sole exception; and
- outside the dedicated schema, excluding separately governed database/public-schema,
  parameter and tablespace facts, every ACL-bearing schema, table/view/materialized
  view/foreign/partitioned table, sequence, column, routine/procedure/aggregate,
  type/domain, language, FDW and server expands `COALESCE(raw_acl,
  acldefault(class,owner))` through `aclexplode`, retains only grantee OID zero,
  and equals the PUBLIC baseline. Array `typacl` is raw-null and derives from its
  element; any PUBLIC large-object atom fails because no stable identity exists;
- direct atoms to any of the nine roles across those classes and large objects
  are empty outside dedicated/current-database facts, and `public` has no objects;
- before `0001`, physical `pg_default_acl` is empty. Exact/post-apply has exactly two global rows
  owned by `sf_supervisor_owner_v1`: function EXECUTE owner-only and type USAGE owner-only. The
  catalogue's table/sequence/schema entries remain absent; every missing/extra owner, namespace,
  class, privilege, grantee or grant option fails. Default ACLs govern future creation only;
  raw-null current-object ACLs expand hard-wired defaults, never `pg_default_acl`; and
- outside the dedicated schema only named database facts plus the digest-bound
  PUBLIC baseline are effective. Code never learns, refreshes, or supplies its pin.

The baseline preimage is protected test fixture
`coding-harness/supervisor-service/__tests__/fixtures/postgresql-16.15-clean-template0-public-object-acl-v1.json`,
excluded from deployed artifacts. Its bytes are exactly `JSON.stringify(records) + "\n"`; record keys
are `objectClass,schemaName,objectName,subobjectName,objectKind,routineIdentityArguments,privilege,grantable`.
Explicit nulls, stable names/16.15 identity arguments and ADR-0045 typed tuple order are mandatory;
duplicates fail. Limits are 1,048,576 bytes, 8,192 records and 65,536 nodes. The clean profile is a
160015 `template0` clone with UTF8/C locale, no non-stock extension/user objects, and stock
`plpgsql` plus its objects included. Independent capture must
reproduce fixture count/bytes/digest; runtime hashes the same full PUBLIC-only projection.

Every explicit membership from a service role to any current PostgreSQL 16.15
role whose name starts `pg_` fails, including an unlisted predefined role. The
only role references without one of the nine records are the literal bootstrap
membership grantor `sf_supervisor_bootstrap_admin_v1` and implicit public-schema
owner `pg_database_owner`; every other reference must resolve to one of nine.

`runtimeCredentialAbsence` names the migration role and exactly the parent,
readiness, recovery, and writer profiles with literal IDs
`semantic-fabric-{parent,supervisor-readiness,supervisor-recovery,
supervisor-writer}-runtime-v1` (brace notation is explanatory, not wire data).

Its rule is `external-profile-observation-required-v1`, outside the database projection. The runner
compares every other member and cannot produce readiness. Activation needs separately branded,
independent evidence that none of four profiles carries the migration credential; PostgreSQL cannot self-attest it.

### 4. Freeze the authority seed

The seed domain is `semantic-fabric/programme-capture/supervisor-postgresql-authority-seed-v1`;
`schemaVersion` is `1`. Root keys are exactly:

```text
domain
schemaVersion
authorityConfiguration
authorityStateIdentity
```

`authorityConfiguration` keys, in table-column order, are:

```text
projectAuthorityDigest
projectScopeRole
configurationEpoch
configurationDigest
genesisAuthorityHeadDigest
serializedConfiguration
serializedConfigurationSha256
projectPrincipalId
projectAuthenticationPolicyDigest
servicePrincipalId
serviceKeyEpoch
serviceKeyFingerprint
serviceSigningSpkiDer
genesisSemanticReceiptDigest
```

`authorityStateIdentity` keys are:

```text
projectAuthorityDigest
projectScopeRole
singletonKey
activeConfigurationEpoch
activeConfigurationDigest
authorityHeadDigest
```

Digests are lowercase nonzero 64-hex; uint64s are canonical unsigned decimal; bytea is unpadded
base64url with decode/re-encode equality. Scope role is literal, singleton JSON `true`, opaque IDs
follow ADR-0044. The parser replays the configuration and independently recomputes configuration/
byte digests, genesis head, project binding and Ed25519 SPKI fingerprint.

`configurationEpoch` is `"0"`; state project/scope/active-epoch/configuration/head fields equal their
configuration counterparts with no defaults/omissions. Reviewed derivation alone creates the fixture
copied into private storage; the runner accepts a plan, not seed bytes. Deployment replacement reviews
and rebuilds seed source, manifest pin, private artifact and exact-byte receipts together.

Insertion separately supplies `last_global_sequence=0`, `next_global_sequence=1`, and
`last_event_digest=NULL`; those mutable fields are forbidden in the preimage. After RLS activation
both rows replay against the private snapshots; extra configuration/state rows fail.

### 5. Freeze and independently pin the manifest

The manifest domain is `semantic-fabric/programme-capture/supervisor-postgresql-migration-manifest-v1`;
`schemaVersion` is `1`. Root keys are exactly:

```text
domain
schemaVersion
authority
readinessAuthorized
postgresqlServerVersion
postgresqlServerVersionNumber
advisoryLockKey
catalogContract
provisioningContract
authoritySeed
migrations
```

Values are `authority=none`, `readinessAuthorized=false`, version `16.15`/`160015`, and advisory-lock
canonical decimal `800874507948546278`, never a JavaScript number.

Contract keys are `path,bytes,sha256`, paths exactly `migrations/catalog-contract-v1.json` and
`migrations/provisioning-contract-v1.json`; seed keys are `domain,bytes,sha256` with no path.
Migration keys are `version,path,bytes,sha256`, exactly versions 1/2 and paths
`migrations/0001-registration-state-v1.sql`/`migrations/0002-registration-rls-v1.sql`.
Lengths, server version, migration version and bytes are positive safe-integer numbers; only the
lock key is decimal text. Digests are lowercase nonzero raw SHA-256.

A private source constant pins the manifest SHA-256 independently of input; the
service artifact and parent manifest bind the constant and file. Coherent
replacement fails before checkout; pin changes require explicit byte review.

Loading starts at fixed `migrations/manifest-v1.json`; size is at most 16,384 before allocation, then its snapshot is parsed/pin-checked.
Its four fixed paths alone map to regular-file descriptors. Each descriptor size equals its manifest `bytes`
and is no greater than its compiled ceiling: catalogue/each SQL 1,048,576; provisioning 65,536. Unrelated
directory entries are inert and ignored; authority never comes from enumeration. Root/file symlinks, hardlinks,
non-regular files, group/world-writable modes, aliases, or descriptor-identity changes fail; each file is read once, checked and executed.

### 6. Define SQL authority and empty state

`0001` contains only the dedicated schema, default ACLs, ten domains with ten named checks, eight tables, sixty
table constraints and revocations. Eight PK plus sixteen unique constraints create the twenty-four
catalogue indexes; no `CREATE INDEX`. `0002` contains direct grants, thirty-eight policies and
ENABLE/FORCE RLS. Both are schema-qualified whole simple-query payloads; no splitter exists.

Static KATs allowlist only those constructs and reject every `forbiddenOwnedKinds` value, transaction
control, role/database/credential creation, dynamic SQL/psql, `IF NOT EXISTS`, `CREATE OR REPLACE`,
`ON CONFLICT`, `COPY`, `SECURITY DEFINER`, session authorization and `public` mutation. Callable
functions are only required `pg_catalog.octet_length`, `substring`, `scale`, and `pg_has_role` uses.
Seed SQL is fixed, separate and parameterized with base PostgreSQL types.

After provisioning preflight and the advisory lock, `empty` means the dedicated
schema is absent and the database-observable provisioning projection is exact.
External runtime-profile evidence is deliberately not a migration input. Any
dedicated-schema presence is `invalid` unless all exact-state conditions below
hold; a missing table, one ledger version, gap, future version, extra row, or
partial object set is never empty and is never repaired.

`exact` requires exactly two ledger rows. Versions 1 and 2 respectively equal
the manifest migration version and script digest, and both rows equal the
manifest catalogue and seed digests. The final catalogue and database
provisioning projection compare exactly. Exactly one configuration and state
row reconstruct the seed's stable identity byte-for-byte; no extra exists.
Mutable chain fields may be constraint-validly advanced. Only the empty
apply must replay `last=0`, `next=1`, `last_event=NULL` before commit. Exact
replay performs no DDL, seed, or ledger write.

The empty apply order is exactly `0001 -> seed -> 0002 -> ledger versions 1 and
2 -> owner-policy seed replay -> same-RW-transaction catalogue/provisioning
comparison -> COMMIT`. Ledger rows store each script digest and the shared
catalogue and seed digests; there is no provisioning-digest column. Both apply
and exact-no-op perform their complete under-lock classification proof before COMMIT.
Same-transaction migration comparison reuses pure normalizers but is not the
later readiness transaction. Readiness separately uses `SERIALIZABLE READ ONLY
DEFERRABLE`, external-profile evidence, and a separately frozen receipt after
commit; this runner never returns ready.

### 7. Use a closed, driver-independent terminal state machine

The live private entrypoint accepts only exact `{plan,store}`, both privately branded; the store is
composition-owned by the pinned PostgreSQL adapter. Malformed/unbranded input throws synchronously
before checkout. Structural stores exist only through a test factory excluded from the service artifact.
The store's sole `checkoutMigration` method is captured before await. Its client-free shell is exact
`{open,discardMalformed}`, captured before `open`; the session is exact
`{execute,release,destroy}`, captured before execute. Failed/malformed open uses only `discardMalformed`.

Capability records are intrinsic plain records with exact string data keys.
Accessors, symbols, proxies, exotic/subclassed record prototypes, and extras
fail without invocation; function values may be native async functions. No
driver type, path, classifier, verifier, repair/retry callback, or caller SQL
enters the API.

Execute accepts exact `{operation,text,values}`. `operation` is a primitive ASCII
string from the exhaustive private plan-step enum; `text` is a copied sealed
script or fixed statement; `values` is a fresh dense intrinsic array of only
null, boolean, safe integer, string, or copied intrinsic `Uint8Array`. The union is:

- `{kind:'command-complete',commandTag,rowCount,rows:[]}` with an exact per-step
  tag and `rowCount` of null or the exact safe integer;
- `{kind:'rows',commandTag:'SELECT',rowCount,columns,rows}` where columns is a
  dense array of exact literal aliases and rows is a dense array of intrinsic
  plain records with exactly those string data keys and only the base values
  above, all synchronously copied before a step-specific codec;
- `{kind:'script-complete',operation}` only for the whole simple-query
  `migration-0001` or `migration-0002` payload; and
- `{kind:'server-rejected',operation,sqlstate,transactionStatus}` with the exact
  in-flight operation, five-character code, and `idle|transaction|failed` state.

Every observation step fixes columns, `maximumRows`, per-alias ceilings and
`maximumResultValueBytes`; catalogue/provisioning/seed/baseline totals are at most
1,048,576/65,536/262,144/1,048,576 bytes, baseline rows 8,192, strings 196,608,
identifiers 63, and typed values their domain maximum.
Each variable expression is evaluated once into `(value,oversize)`: null is `(null,false)`, admitted
is `(value,false)`, excess `(null,true)` after `pg_catalog.octet_length`. A MATERIALIZED bounded-row
CTE returns one driver row with exact columns `payload,oversize`: admitted payload is bytea holding
the pinned PostgreSQL-16.15 UTF-8 JSON array `[aliases,denseRowArrays]` (bytea fields are lowercase
even hex), while any cell/row/aggregate excess returns only `(null,true)`. Data may lower but never
raise limits; truncation, hash substitution, missing/cross-wired sentinels, or payload with true fails.
Before copying/decoding, the bridge checks columns/types, intrinsic length and sentinel. No rejected
bytes or driver-side clone/slice/`Buffer.from`/encoding reaches the row codec; excess rolls back.

All records are structurally untrusted and exact-key/type checked. Each live result also has
private single-use WeakMap metadata containing its branded session, operation and ordinal.
The composition bridge emits it only after the exact CommandComplete/ErrorResponse sequence and
required ReadyForQuery state (`transaction` in-transaction; `idle` after COMMIT/ROLLBACK).
A rejection additionally requires the exact non-proxy/non-subclassed pinned-`pg` `DatabaseError`
caught from that awaited operation and matched to its ErrorResponse; `I|T|E` map only to
`idle|transaction|failed`. Plain/foreign/stale errors and caller code/status cannot obtain a brand.
Missing, replayed, cross-session, mismatched, or protocol-incomplete COMMIT evidence is uncertain. The
artifact excludes the disjoint test transcript brand/factory; no splitter or partial tag exists.

Preflight before `BEGIN` verifies exact `SESSION_USER=CURRENT_USER` migration
identity, role attributes, sole membership edge with grantor/options, and server
version. It issues `BEGIN ISOLATION LEVEL READ COMMITTED READ WRITE`, then these
statements in order:

1. `SET LOCAL search_path TO pg_catalog`;
2. `SET LOCAL row_security TO on`;
3. `SET LOCAL lock_timeout TO '5000ms'`;
4. `SET LOCAL statement_timeout TO '30000ms'`;
5. `SET LOCAL idle_in_transaction_session_timeout TO '30000ms'`;
6. `SET LOCAL synchronous_commit TO on`.

One fixed query reads `pg_catalog.pg_settings.setting/unit` for
`5000/30000/30000` and `ms`, and verifies READ COMMITTED, read-write,
`session_replication_role=origin`, `fsync=on`, and `full_page_writes=on`; the runner never sets the last three. It then
takes the fixed advisory transaction lock, issues
`SET LOCAL ROLE sf_supervisor_owner_v1`, and reverifies all settings plus
`SESSION_USER=sf_supervisor_migration_login_v1` and
`CURRENT_USER=sf_supervisor_owner_v1` before classification.

`release()`, `destroy()`, and `discardMalformed()` take no arguments and are
one-use. Their sole successes are primitive `released`, `destroyed`, and
`discarded`. A throw or any missing, accessor-bearing, widened, or wrong-literal
completion self-quarantines the client/shell; no second terminal is invoked.

| Phase/result | Terminal action | Runner result |
|---|---|---|
| checkout fails before a client exists | none | `rejected` |
| open fails or returns malformed | `discardMalformed` | `rejected`, or `rejected-cleanup-failed` without exact acknowledgement |
| known/malformed/thrown failure before BEGIN | `destroy` | `rejected`, or `rejected-cleanup-failed` without exact acknowledgement |
| known decoded failure after BEGIN | exact `ROLLBACK`, then `release` | `rejected`, or `rejected-cleanup-failed` if release is not acknowledged |
| post-BEGIN throw/malformed/unexpected status | `destroy` | `resolution-unknown`, regardless of destroy outcome |
| ROLLBACK server-rejected/thrown/malformed | `destroy` | `resolution-unknown`, regardless of destroy outcome |
| authenticated COMMIT rejection with `idle` | `release` | known `rejected` cleanup outcomes; never retry |
| authenticated COMMIT rejection with `failed` | exact `ROLLBACK`, then `release` | known `rejected` cleanup outcomes; never retry |
| COMMIT rejection with other status | `destroy` | `commit-resolution-unknown` |
| COMMIT thrown/malformed/transport-uncertain | `destroy` | `commit-resolution-unknown`, regardless of destroy outcome |
| COMMIT exact command-complete | `release` | `applied` or `exact-no-op`; `committed-cleanup-failed` if release is not acknowledged |

No same-invocation migration retry exists. A fresh operator invocation may try
to acquire the lock and reclassify; it fails closed on lock timeout and never
assumes a prior failed cleanup released the lock. Every result is deeply frozen,
contains `authority:none` and `readinessAuthorized:false`, and exposes no SQL,
seed, row, receipt, credential, or connection detail.

## Acceptance gates

This decision is implemented only when:

1. independent Node 20.0.0 and 24.14.1 KATs pin every JSON/SQL byte, length,
   digest, order, final LF, round trip, limit and private brand;
2. every malformed/reordered/over-limit path, buffer, PUBLIC/column/default-ACL/baseline, role,
   membership, seed-cross-wire and script mutant fails before checkout or rolls back by phase;
3. transcripts prove null/max/max+1/multibyte/payload-plus-true/missing/cross-wired cell and aggregate
   sentinels; rejected bytes never reach the driver. Brand replay, cross-session/wrong ordinal,
   missing/wrong ReadyForQuery and ErrorResponse/error mismatch fail closed;
4. PostgreSQL 16.15 proves empty/exact/concurrent apply, stock baseline, PUBLIC object/default ACL,
   value sentinels, acknowledged-versus-uncertain rollback/commit, pre-send termination,
   unknown-commit reclassification, no retry/partial repair, and every ADR-0044/0045 denial;
5. runtime source and sealed data are private build inputs; test fixtures and
   capture evidence are parent/harness protected but outside service `BUILD_INPUT_PATHS`.
   Source and tests remain under 500 lines, security/ADR/frozen-harness gates pass, and the public bundle SHA-256 remains
   `90e21e7c0e3a45b66da55f0e8cf9c0a23b3fb82e805223922d81096e097f7c3a`.

## Consequences

- **Positive:** Deployment bytes and uncertainty gain one replayable meaning;
  coherent substitution fails; disjoint authorities cannot silently replace
  one another; and the private supervisor gains migrations without public API
  or runtime-dependency changes.
- **Trade-off:** The initial profile is tied to one literal role layout and
  PostgreSQL 16.15. Changes require reviewed bytes, digests, tests, and live
  evidence; cleanup uncertainty requires operator reclassification, not retry.
- **Boundary:** This proposed contract is not an independently administered
  production deployment.

## Alternatives rejected

- **Let parser code choose schemas or literals** — makes implementation an unreviewed authority.
- **Trust a caller-supplied manifest and scripts together** — permits coherent
  substitution.
- **Store the deployment seed as a migration file** — conflates generic schema authority with deployment-specific trust material.
- **Treat any missing object as empty** — turns drift into unauthorized repair.
- **Reuse registration retries** — cannot safely resolve migration or COMMIT
  uncertainty.
- **Return success after uncertain cleanup** — overstates the terminal state; returning generic unknown after acknowledged COMMIT understates it.

## Links

[ADR-0038](ADR-0038-sota-application-completion-programme.md), [ADR-0039](ADR-0039-minimal-production-serving-artifact.md), [ADR-0042](ADR-0042-witnessed-single-use-capture-supervisor-protocol.md), [ADR-0043](ADR-0043-postgresql-supervisor-registration-state-and-dormant-adapter.md), [ADR-0044](ADR-0044-postgresql-supervisor-catalogue-contract.md), [ADR-0045](ADR-0045-canonical-postgresql-supervisor-catalogue-oracle-representation.md), and [ADR-0047](ADR-0047-canonical-postgresql-16-15-public-acl-baseline-projection.md).
