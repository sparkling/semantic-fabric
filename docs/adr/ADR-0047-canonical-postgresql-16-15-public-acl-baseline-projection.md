---
status: proposed
date: 2026-08-30
updated: 2026-08-31
tags: [postgresql, supervisor, privileges, canonical-json, verification]
supersedes: []
depends-on: [ADR-0038, ADR-0039, ADR-0042, ADR-0043, ADR-0044, ADR-0045, ADR-0046]
implements: [ADR-0046]
---

# Canonical PostgreSQL 16.15 PUBLIC ACL baseline projection

## Status boundary

This ADR is **proposed**. It fixes the closed vocabulary, catalogue projection,
ordering, capture profile and candidate replay evidence for ADR-0046's stock PUBLIC-ACL
baseline. It does not accept ADR-0042 through ADR-0046, provision PostgreSQL,
authorize migrations or readiness, refresh a pin, or make a deployment.

The fixture is a protected test oracle, not a deployed input. Any admitted
runtime implementation will own the independently reviewed projection but must
never embed, learn or supply the expected baseline count, length or digest.
Those values enter only through the reviewed provisioning contract selected by
the independently pinned manifest.
The current receipts are test-fixture evidence only. Receipt V1 remains an
immutable, non-attesting predecessor; additive receipt V2 binds its exact bytes
before adding the OID/attribute-aware candidate matrix and effective-privilege
witness. They are replay-checkable against the owned fresh-initialization
procedure, but neither authenticates its historical run event nor admits a
runtime fixture.

## Context

ADR-0046 fixes eight record keys and a clean PostgreSQL profile, but not the
field nullability, class/kind vocabulary, true-array privilege rule, canonical
sort, or a replayable capture procedure. A digest over an unspecified
projection would let two reasonable implementations produce different bytes.

PostgreSQL also treats a true array's type privileges as its element type's
privileges. Expanding the array row's own `typacl` happens to match a stock
cluster, but becomes unsound after a grant or revoke. The baseline must follow
PostgreSQL 16.15's access-control semantics, not only reproduce one fixture.

## Decision

### 1. Keep projection, expectation and provenance disjoint

The three authorities are:

| Authority | Owns | Must not own |
|---|---|---|
| this projection and its protected implementation | which database facts become ordered records | expected count, byte length or digest |
| sealed provisioning contract | baseline profile plus expected count, byte length and raw SHA-256 | projection SQL or capture provenance |
| independently pinned manifest and sealed source | which provisioning bytes are admissible | a replacement projection or learned baseline |

A semantic parser brand proves only that bytes satisfy this record contract. A
separate sealed-source/plan brand proves reviewed provenance. Coherently
substituted records, fixture and provisioning bytes may parse, but cannot obtain
the latter brand or reach checkout.

Three module-private brands are pairwise disjoint and have no conversion API:

1. a fixture-semantic handle exists only in test code and is absent from the
   service build graph;
2. a live-observation handle is minted only by ADR-0046's authenticated
   session/operation/ordinal and protocol bridge after checkout; and
3. a sealed `Plan` is minted only by the fixed no-caller-bytes loader after the
   compiled manifest pin admits its provisioning bytes.

The runtime normalizer accepts only an already admitted `Plan` plus its live
observation. It accepts no raw bytes, records, fixture handle, digest tuple,
query, parser handle, callback or structural substitute. Coherent fixture,
provisioning and manifest replacements may all be semantically valid but must
fail the compiled pin with zero `checkoutMigration` property access. A forged
or substituted live observation can exist only after checkout and must cause
rollback before COMMIT; it is not misreported as a pre-checkout rejection.

### 2. Define the clean capture profile

The capture source pin is the OCI Linux/amd64 platform manifest
`postgres@sha256:485935f94cc7165afa896978809c37b592dc07f0a37d2c8f645f12412d0212c8`
with media type `application/vnd.oci.image.manifest.v1+json`; its configuration is
`sha256:80f4c7a5e91618546dce5b4fe60cf03b14c0f9efa7e40157278d122772ced8d2`.
The current receipt claims no separate OCI image-index identity.
It reports PostgreSQL `16.15 (Debian 16.15-1.pgdg13+2)` and
`server_version_num=160015`.

`initdb` uses `--locale=C --encoding=UTF8`. An isolated, unexposed container
creates `sf_public_baseline` from `template0` with UTF8, libc locale provider,
`LC_COLLATE=C` and `LC_CTYPE=C`. Capture starts immediately, before any user
DDL, role, grant or extension command. The database must have:

- schemas `information_schema`, `pg_catalog`, `pg_toast`, and empty `public`;
- exactly one non-relocatable `plpgsql` extension version `1.0` in
  `pg_catalog`, null `extconfig`/`extcondition`, and exactly extension members
  language `plpgsql` plus routines `plpgsql_call_handler()`,
  `plpgsql_inline_handler(internal)`, and `plpgsql_validator(oid)`;
- no physical `pg_default_acl` row, foreign-data wrapper, foreign server, user
  mapping or large object; and
- no schema named `sf_supervisor_v1`.

The capture session uses `BEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY
DEFERRABLE`, `SET LOCAL search_path TO pg_catalog`, `SET LOCAL row_security TO
on`, `SET LOCAL quote_all_identifiers TO off`, and `SET LOCAL client_encoding
TO 'UTF8'`. One closed guard row in that same transaction proves these GUCs,
server/client encoding, database locale/provider, schema/extension sets,
public-namespace dependency closure, supported kind/handler mappings and every
negative profile count before any row is admitted. False, missing, extra or
malformed guard data is an explicit invalid result, never an empty projection.
The transaction commits no state.
Authentication settings and container credentials are transport-only and are
not part of the baseline preimage.

### 3. Define one closed record schema

Every record is an intrinsic plain object with exactly these keys in order:

```text
objectClass
schemaName
objectName
subobjectName
objectKind
routineIdentityArguments
privilege
grantable
```

`objectName` and `objectKind` are non-empty strings. `schemaName`,
`subobjectName`, and `routineIdentityArguments` are explicit string or null.
Names come from PostgreSQL `name` values and are at most 63 UTF-8 bytes;
routine identity arguments are the literal one-argument
`pg_catalog.pg_get_function_identity_arguments(p.oid)` result under the fixed
GUCs, with no post-normalization; zero arguments are the permitted empty
string. They may be at most 196,608 UTF-8 bytes. NUL, invalid UTF-8 and any
unpaired surrogate in a decoded key or value fail codecs.
`grantable` is the JSON boolean `false`; PostgreSQL does not permit grant
options to PUBLIC.

The closed class and kind contract is:

| `objectClass` | `schemaName` | `subobjectName` | `objectKind` | identity arguments | allowed privilege |
|---|---|---|---|---|---|
| `schema` | null | null | `schema` | null | `CREATE`, `USAGE` |
| `relation` | string | null | `table`, `partitioned-table`, `view`, `materialized-view`, `foreign-table`, `sequence` | null | kind-appropriate table or sequence privilege |
| `column` | string | string | parent kind except `sequence` | null | `INSERT`, `SELECT`, `UPDATE`, `REFERENCES` |
| `routine` | string | null | `function`, `procedure`, `aggregate`, `window-function` | string | `EXECUTE` |
| `type` | string | null | `base`, `composite`, `domain`, `enum`, `pseudo`, `range`, `multirange`, `array` | null | `USAGE` |
| `language` | null | null | `language` | null | `USAGE` |
| `foreign-data-wrapper` | null | null | `foreign-data-wrapper` | null | `USAGE` |
| `foreign-server` | null | null | `foreign-server` | null | `USAGE` |

Table-like privileges are `SELECT`, `INSERT`, `UPDATE`, `DELETE`, `TRUNCATE`,
`REFERENCES`, and `TRIGGER`; sequence privileges are `SELECT`, `UPDATE`, and
`USAGE`. The table fixes both admissible values and their spelling; no future
PostgreSQL kind is silently accepted under this versioned profile.

Schema records put the schema name in `objectName`. Schema-bound objects put
their owning namespace in `schemaName`. Column records put the parent relation
in `objectName` and column in `subobjectName`. Global objects put their catalog
name in `objectName` and use null for all inapplicable identity fields.

Large objects deliberately have no record representation because their OID is
not a stable cross-cluster identity. Any PUBLIC atom in
`pg_largeobject_metadata.lomacl` is a separate fatal profile violation; zero
large objects alone is not a substitute for that ACL check.

“Effective” in this ADR means null expansion of the selected catalogue ACL
slot, not ownership, membership, predefined-role effects or relation-to-column
implication. True-array element mapping is the sole exception. Relation PUBLIC
atoms are not duplicated into column records.

### 4. Project PostgreSQL's effective PUBLIC atoms exactly

For every supported object, expand the selected ACL with
`pg_catalog.aclexplode`, retain `grantee=0`, copy `privilege_type` and
`is_grantable`, and discard grantor. Grantor omission is deliberate: the
baseline describes effective PUBLIC atoms and avoids binding the cluster's
bootstrap-superuser name. If different grantors yield the same record identity,
the preserved duplicate makes the projection invalid rather than being
silently deduplicated.

Within each explicitly enumerated governed source/scope, a positive branch
materializes every `aclexplode` atom and then filters only on `grantee=0`;
class, kind, privilege, grantability, identity and duplicate validity are
checked afterwards. Branch composition is `UNION ALL`. `UNION`,
`DISTINCT`, `DISTINCT ON`, `GROUP BY`, set aggregation and any other
deduplication before client duplicate validation are forbidden.

The source and hard-wired default for each class are:

| Class | Source | Selected ACL when raw ACL is null |
|---|---|---|
| schema | `pg_namespace.nspacl/nspowner` | `acldefault('n', nspowner)` |
| table-like relation | `pg_class.relacl/relowner`, relkind `r,p,v,m,f` | `acldefault('r', relowner)` |
| sequence | `pg_class.relacl/relowner`, relkind `S` | `acldefault('s', relowner)` |
| column | `pg_attribute.attacl` plus parent owner, positive/non-dropped columns of `r,p,v,m,f` | `acldefault('c', relowner)` |
| routine | `pg_proc.proacl/proowner`, prokind `f,p,a,w` | `acldefault('f', proowner)` |
| type | effective type row described below | `acldefault('T', effective owner)` |
| language | `pg_language.lanacl/lanowner` | `acldefault('l', lanowner)` |
| foreign-data wrapper | `pg_foreign_data_wrapper.fdwacl/fdwowner` | `acldefault('F', fdwowner)` |
| foreign server | `pg_foreign_server.srvacl/srvowner` | `acldefault('S', srvowner)` |

For type records, `typtype b,c,d,e,p,r,m` maps respectively to
`base,composite,domain,enum,pseudo,range,multirange`. A **true array** is instead
kind `array` when `typelem` is nonzero and `typsubscript` is the exact
`pg_catalog.array_subscript_handler` procedure. Matching PostgreSQL 16.15's
`IsTrueArrayType`/`pg_type_aclmask`, its effective owner and ACL come from the
referenced element row. Its own `typacl` must be raw null: PostgreSQL does not
consult that field, so accepting a non-null value would hide otherwise
observable drift behind an unchanged effective digest. A non-null array ACL,
missing element or unexpected type/procedure mapping fails the projection.

Base ACL atoms are selected before identity resolution. Every namespace,
parent relation, true-array element and subscript-handler OID needed by a
candidate atom must resolve exactly once through non-dropping joins. A missing,
multiply resolved, cross-wired or relinked identity sets the invalid sentinel;
an optional join can never silently remove an atom. The same-snapshot negative
guard also rejects unsupported PostgreSQL-16.15 `relkind`, `prokind`,
`typtype`/element-handler cases carrying candidate PUBLIC authority, and PUBLIC
column ACLs on dropped, system or unsupported-parent columns. Separately
governed database, tablespace, parameter, default-ACL, large-object, `public`
and dedicated-schema scopes have explicit guard outputs rather than filtering.

`public` and `sf_supervisor_v1` schemas and their contained objects are excluded
because ADR-0046 verifies them separately and requires `public` to contain no
objects. Physical `pg_default_acl` never substitutes for a null current-object
ACL: `acldefault` is the hard-wired default. The capture includes stock
`pg_catalog` and `information_schema` objects and does not use `pg_init_privs`
as a replacement for current ACL state.

Raw null and explicit empty are intentionally equivalent in this PUBLIC-only
projection when the hard-wired default has no PUBLIC atom: schemas,
table-like relations, sequences, columns, FDWs and servers. They differ for
routines, types and languages, whose hard-wired null default grants PUBLIC
`EXECUTE` or `USAGE`; explicit empty emits nothing. Physical
`pg_default_acl` affects future creation only and never changes this matrix.

### 5. Canonicalize records and bytes

Records sort by all eight fields, left to right. The final SQL `ORDER BY` is
text `COLLATE pg_catalog."C" ASC`, with `NULLS FIRST` on the three nullable
strings, followed by boolean `ASC`; every `name` is cast to text first. The
client compares adjacent rows as received using unsigned UTF-8 bytes, null
before string and false before true. It never sorts untrusted rows before
hashing. Two equal adjacent records are forbidden, including two preserved
atoms whose different grantors disappear from the record.

The fixture is exactly `JSON.stringify(records) + "\n"`: one dense JSON array,
compact member encoding, the key order above, no BOM/CR/trailing data or
alternate escaping, and one final LF. It is stored only at:

```text
coding-harness/supervisor-service/__tests__/fixtures/postgresql-16.15-clean-template0-public-object-acl-v1.json
```

The hostile reader enforces at most 1,048,576 input bytes including final LF,
8,192 non-root objects, 65,536 JSON nodes, eight decoded own keys, root-array
width 8,192, depth three and 196,608 decoded Unicode-scalar UTF-8 bytes per key
or value before semantic replay. Root depth is one; objects, arrays and
primitives are nodes, while keys are not. These are independent pre-schema
scanner ceilings, not a promise that 8,192 valid records fit every other
ceiling: `N` valid eight-field records cost `1 + 9N` nodes and at least
`168N + 2` bytes. Exact maximum and maximum-plus-one KATs isolate every guard.

The reader snapshots an intrinsic non-proxy `Uint8Array`, performs fatal UTF-8
decode/re-encode, scans duplicate decoded keys and limits before its sole
`JSON.parse`, rejects escaped or literal lone surrogates and NUL, reconstructs
every record, replays exact bytes, and stores mutable bytes only behind a
private brand. Literal/escaped equivalent duplicate keys fail before
`JSON.parse`; a valid surrogate pair and raw astral scalar remain canonical.

Isolated scanner KATs use a root array of 8,192 empty objects, then nest one
additional object inside one element for record maximum/+1;
`1 + 8,192 + 57,343` nodes then one more scalar for node
maximum/+1; six strings containing `5 × 196,608 + 65,516` bytes then one more
byte for exact input maximum/+1; 8,192/+1 root primitives for width; 8/+1
decoded keys for object width; and one nested object then one deeper for depth.

The recorded reproduction establishes candidate test-fixture values only. The
tracked replay orchestrator owns two fresh networkless containers, anonymous
data volumes and `template0` databases, then binds the projection and independent
raw-catalogue cross-check for each run. Matching projection runs or client
reserializations prove determinism, never completeness. The candidate must not
enter provisioning or manifest bytes until Section 7's explicitly enumerated,
fail-closed pre-sealing predicate passes. That predicate contains no
post-bundle gate; item 6 cannot be a prerequisite for constructing the bundle
that it tests.

### 6. Make runtime comparison bounded and independent

The later live verifier executes the same semantic projection in one
`SERIALIZABLE READ ONLY DEFERRABLE` snapshot. Its fixed query returns only a
bounded ADR-0046 `[aliases,denseRowArrays]` UTF-8 payload plus an
oversize/invalid sentinel; the driver bridge checks the exact
row/column/protocol brand before copying or decoding it. PostgreSQL returns no
final fixture JSON, JSONB record aggregate, digest or set-aggregated authority.
The client reconstructs records, validates received order and multiplicity,
then hashes exactly `JSON.stringify(records) + "\n"`, including the LF.

The runtime normalizer independently validates record shape, ordering,
duplicates, absence of any PUBLIC large-object ACL atom and limits, then computes observation count,
exact canonical byte length and raw SHA-256. It compares those three values to
the already admitted provisioning handle. It neither reads the fixture nor
contains its expected digest. A matching digest does not replace the separate
public-schema, service-ownership, direct-grant, default-ACL, parameter,
tablespace or database checks in ADR-0046.

### 7. Require independent replay and hostile evidence

Future runtime admission requires all of:

The pre-sealing predicate is exactly: items 1–5 in full; item 7's candidate
fixture/capture-pin and fixture/capture-KAT half; item 8 in full; and item 9's
fixture/capture protection and service-build-exclusion half. No evidence can be
treated as implicitly applicable. Item 6, item 7's bundle/runtime-pin and
live-observation-KAT half, and item 9's private implementation/sealed-data
build-input half run only after ADR-0046 constructs the immutable bundle. Each
pre-sealing proof is then rerun unchanged and independently under its original
authority. A separate bundle-consistency check compares its result with the
sealed bundle; neither the bundle nor observed database state supplies an
oracle to those proofs. This split prevents a circular requirement without
weakening runtime admission.

1. two newly initialized containers from the exact pinned platform manifest produce identical raw
   transcripts, ordered records, count, bytes and digest without a shared data
   volume;
2. a separately pinned completeness oracle uses ten independent COPY statements
   (nine raw-catalogue sections plus one control record), retaining every source row and ACL atom, including
   raw-null/explicit-empty inventory, every grantee, grantor and ordinality. It
   consumes neither projection stream nor fixture, reuses neither its `UNION`
   nor predicates, derives defaults, true arrays, vocabulary and ordering
   independently, and compares the ordered multiset in both directions;
3. independent direct-ACL expansion is the column completeness witness. A
   fresh no-membership role's `has_*_privilege` calls corroborate every positive
   stock atom, including true arrays; for a column they equal relation atom OR
   column-local atom and cannot prove column-atom absence;
4. mutants cover every class/kind/privilege/nullability, row and key order,
   duplicate, grantable bit, raw-null/explicit-empty ACL, default-ACL
   non-retroactivity, true-array own/element ACL, unresolved OID, unsupported
   kind, dropped/system column, public-schema object and large-object atom;
5. deleting any branch/predicate, returning zero, omitting/adding an atom,
   changing `UNION ALL`, swapping array/element ACLs, reordering, or a
   count-neutral substitution is caught by the independent multiset oracle;
6. coherent fixture + provisioning + manifest substitution parses semantically
   but fails compiled-pin `Plan` creation with zero checkout access; fixture as
   live observation, structural/proxy/accessor brands and stale/cross-session/
   wrong-ordinal observations fail, with post-checkout failures rolling back;
7. before sealing, Node 20.0.0 and 24.14.1 produce identical candidate
   fixture/capture pins and pass the fixture/capture exact/max/+1,
   literal/escaped/multibyte, hostile-carrier, private-brand and copy-alias
   KATs; after bundling, those versions produce identical bundle/runtime pins
   and pass the corresponding live-observation private-brand and copy-alias
   KATs;
8. capture receipts bind the OCI Linux/amd64 platform manifest/config, initdb and database
   creation facts, projection and independent-oracle source digests, distinct
   volume identities, raw multiset transcript and final pins; and
9. fixture and capture tests are parent/harness protected inputs but remain
   outside service `BUILD_INPUT_PATHS`; implementation source and sealed
   provisioning/manifest data are private build inputs and never public exports.

Owned fresh replay orchestration and the no-membership `has_*` witness are
implemented and have reproduced the candidate locally. A protected test-only
reader now snapshots an intrinsic non-resizable byte carrier, enforces the
specified decoded-key and allocation ceilings before its sole `JSON.parse`,
replays exact canonical bytes, and retains a private copy behind an opaque
`authority=none` handle. Its 92 hostile/limit/private-brand KATs plus five
exact-fixture baseline tests pass on exact Node 20.0.0 and 24.14.1. Unpublished
developer-local runs report both V1 and V2 live replay, 602/602 supervisor
tests, and 892 parent-harness tests with two intentional skips.

The parent harness uses explicit non-recursive root and discovered-directory
watches, requires setup/cooperative/settled metadata digests to agree, and
fails closed at 8,192 watchers while closing every partially opened watcher.
This avoids Node 20's recursive-watcher teardown race without changing digest bytes.

An additive protected pure mutator, extracted fail-closed replay support and 17
focused KATs identify the exact eight top-level `UNION ALL` class branches,
construct one deletion per branch, and construct four scanner-positioned
record-set mutants without editing the pinned projection. The latter return
zero rows, omit the first canonical atom, add one typed canonically last
sentinel atom, or replace the first atom with that sentinel while preserving
the count. Held-root and held-file `O_NOFOLLOW` descriptors plus pre/post
identity checks reject absolute/parent, symlink, hardlink, directory and
oversized sources. Exact frozen process arguments and mutated container
snapshots make identifier, image, environment, network, port and mount
predicates load-bearing before PostgreSQL execution.

The test-only live verifier uses two fresh networkless anonymous-volume
containers. Within one serializable rollback-only transaction per run it seeds
PUBLIC `USAGE` on one foreign-data wrapper and server, proves all eight classes
are positive, and requires original and explicit-column normalized projections
to equal the independently derived raw-catalogue bag. Every mutant executes
and parses. A branch deletion must preserve the exact seven-class survivor bag;
zero, omission, addition and substitution must equal their mutation-specific
exact output, with the sentinel independently pinned by a KAT. The full oracle
then rejects the first eleven mutants
with `ORACLE_RECORD_BAG_KEYS_MISMATCH` and the count-neutral substitution with
`ORACLE_RECORD_BAG_MULTIPLICITY_MISMATCH`. Postflight proves the seed absent and
owned cleanup is checked. Exact Node 20.0.0 and 24.14.1 produce the same
12,584,275-byte deterministic transcript and kill all 12 mutants in both runs.
The 60-second session and 15-second probe/inspection ceilings consume at most
120 seconds beneath the 300-second parent, leaving 180 seconds for bounded
local analysis and termination; CI allows 120 minutes for the conservative
110-minute sum of all six isolated replay and cleanup ceilings.

This closes only item 5's delete-any-top-level-class-branch, return-zero,
single-atom omission/addition and count-neutral-substitution subsets. Item 4
and the remaining item 5 predicate, value, nullability, order, duplicate,
array/element and `UNION ALL` replacement mutations remain open, as do item
6's sealed runtime parser/compiled-pin `Plan` and live-observation brands. The
test-only reader, mutator, replay support and verifier remain outside service
build inputs and exports, so they create no runtime or migration dependency.
These developer runs are not bound by a versioned test-run receipt, and a
passing GitHub-hosted live matrix remains open. The current evidence therefore
does not satisfy runtime admission.

### 8. Record the reproduced test-only candidate

The frozen V1 test-only receipt records two orchestrator-owned runs. Each uses the
exact platform manifest/configuration, default command/environment plus the two
pinned PostgreSQL variables, network mode `none`, no published ports and exactly
one fresh anonymous `PGDATA` volume. The runner proves the database absent before
creating `sf_public_baseline` from `template0` with UTF8/libc/C/C, binds the full
profile and result to each run, and ownership-checks cleanup. It records profile SHA-256
`15d6ff996e0cf5cec2fd269898c6ec470f35d2b8e25da6f2535daa95324f92c7`,
raw-oracle transcript SHA-256
`e1f9f698c9778f3e80eec44346e5f76305831783c8f28cd7d465cb5a5065b463`
and the same projection. The independent oracle source is 16,037 bytes with
SHA-256 `6a1cf204ca8c5a3aa7a70da4f5c8c46cd15998b745d2eb648b77568e6c912722`;
the projection source is 6,859 bytes with SHA-256
`0e3ad724f4ce85191564c245c51dd7665b6d9aa704c355067a0056cdbfe95232`.

Additive receipt V2 is 8,816 bytes with SHA-256
`48d54b635ff6bafc6bdb4ffcb1bb9d74c8357e932e22f7b6453bb54cb0d698e8`.
It first pins the 4,835-byte V1 receipt at
`14fbd3ff2d2b50d3a8adbe0b51dc921eb926cd644a4a765183723518ec4fd08b`,
then binds an OID/attribute-aware candidate matrix and an effective-privilege
`has_*` witness over the closed eight-class matrix. V1's raw oracle independently
expands the direct ACL atoms. A fresh no-membership role performs 13,603 checks
across its six populated classes:
5,958 true, 7,645 false and zero true grant-option results. It corroborates all
4,059 projected atoms, including 16 column-local atoms and 294 true-array atoms,
and binds the inventory, observation, raw-oracle, session and per-run transcript
digests plus every witness/replay source. FDW and server remain explicit
zero-check/zero-atom class counts under this exact clean profile rather than
omitted classes.

The reproduced candidate fixture is **4,059 records**, **36,532 JSON nodes** and **860,988
bytes**, with raw SHA-256
`a108e05f9cfd6d6485a86fe198a87b3800e21986b5c62e6251519de6577d05be`.
These values may become provisioning inputs only after the Section 7
pre-sealing gates pass and ADR-0046's later sealed bundle pins them. Item 6 then
tests that bundle before admission; presence here or in test evidence grants no
runtime authority.

## Consequences

- **Positive:** The candidate baseline digest has one independently implementable
  meaning, including the PostgreSQL-specific array rule and negative classes.
- **Trade-off:** The contract is intentionally tied to PostgreSQL 16.15 and one
  amd64 platform manifest. A patch, image or vocabulary change needs a new reviewed
  profile and fresh independent capture; it cannot refresh in place.
- **Boundary:** This closes a representation gap only. It supplies no database,
  migration, readiness, deployment or release authority.

## Alternatives rejected

- **Commit the first observed digest** — reproduces an implementation without
  defining its meaning.
- **Use `information_schema` alone** — omits PostgreSQL-specific classes,
  object kinds, column details and true-array behavior.
- **Use `has_*_privilege` as the sole projection** — loses raw-null/default and
  grantable structure and can include membership/predefined-role effects.
- **Expand each array row's `typacl`** — disagrees with PostgreSQL access checks
  after element grants or revokes.
- **Include bootstrap grantor or OIDs** — introduces cross-cluster identities
  that the effective PUBLIC baseline does not need.
- **Load fixture bytes in production** — turns test evidence into deployable
  authority and permits fixture substitution to affect runtime behavior.

## Links

[ADR-0038](ADR-0038-sota-application-completion-programme.md),
[ADR-0039](ADR-0039-minimal-production-serving-artifact.md),
[ADR-0042](ADR-0042-witnessed-single-use-capture-supervisor-protocol.md),
[ADR-0043](ADR-0043-postgresql-supervisor-registration-state-and-dormant-adapter.md),
[ADR-0044](ADR-0044-postgresql-supervisor-catalogue-contract.md),
[ADR-0045](ADR-0045-canonical-postgresql-supervisor-catalogue-oracle-representation.md),
[ADR-0046](ADR-0046-sealed-postgresql-supervisor-migration-authority-bundle.md),
[capture receipt V1](../../coding-harness/supervisor-service/__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v1.json),
[capture receipt V2](../../coding-harness/supervisor-service/__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v2.json),
[PostgreSQL 16 privileges](https://www.postgresql.org/docs/16/ddl-priv.html), and
[PostgreSQL 16.15 `pg_type_aclmask`](https://github.com/postgres/postgres/blob/REL_16_15/src/backend/catalog/aclchk.c#L3483-L3552).
