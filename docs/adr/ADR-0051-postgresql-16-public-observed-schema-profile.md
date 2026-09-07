---
status: accepted
date: 2026-09-02
updated: 2026-09-07
tags: [postgresql, schema, identity, pg-catalog, runtime, observation]
supersedes: []
depends-on: [ADR-0006, ADR-0015, ADR-0038, ADR-0048, ADR-0050]
implements: [ADR-0050]
---

# PostgreSQL 16 public observed-schema profile

## Status boundary

This ADR is **accepted for observation-profile qualification only**. It freezes the first production-shaped observation profile required by ADR-0050 Phase 2.
The private, explicitly opt-in Rust adapter now covers one PostgreSQL 16 semantic catalogue contract and can return its branded, non-authorizing identity from the same repeatable-read transaction as the legacy projection. PostgreSQL 16.9
and 16.15 are the initial exact live qualification targets. The adapter implements the exact engine selector, profile
finalizer, guards, bounded typed row decoding, relation/column/type/default-collation normalization, live NOT NULL
emission, and raw PK/UNIQUE/FK adaptation. FK proof binds the ordered equality-operator arrays to the exact `conindid`
B-tree key/opclass/collation positions and strategy-3 `pg_catalog.=` signature. Index key prefixes are exact; only
resolved positive INCLUDE tails are admitted, while partial, expression, reordered and non-default-opclass indexes
reject. The four structural trigger roles are checked as a multiset, including self-reference; `O`/`A` are enforced and
`D`/`R` are valid but not enforced. NOT NULL and catalogue constraints share one 65,536-entry budget, and each bounded
collector polls at most the single overflow sentinel without decoding or retaining it.

Adversarial unit tests cover those laws. Untracked operator diagnostics against isolated PostgreSQL 16.9 and 16.15
produced byte-equal baseline digests across patches; disabling one FK trigger changed only the constraint digest,
dropping NOT NULL changed only the constraint digest, and reversing the referenced key or using `text_pattern_ops`
failed closed with a redacted unsupported-constraint error. No repository receipt backs those observations. They are
development diagnostics, not qualification: they did not execute the required twice-per-image networkless,
non-owner-role, immutable-image and byte-identical protocol.

The public Rust diagnostic collects a complete legacy projection and publishes either the branded rich identity or one closed, identifier-free `Unavailable` reason only after the outer transaction commits.
Clean pre-legacy mismatches remain pending unavailability; rich failures may downgrade only after successful savepoint recovery, while transaction, legacy or commit failures remain fatal.
The evidence executor publishes an ordered receipt pair atomically only after both exact-patch replays and the cross-patch laws pass. Until that valid tracked pair exists it fails closed; a valid pair promotes only this observation profile.
Existing legacy entry points are unchanged. For authored mappings, `sf-serve` carries the committed availability state bound to backend and `SourceId` only as a non-authorizing diagnostic.
A distinct private Direct-Mapping foundation consumes the rich in-transaction API as one step in an unforgeable PostgreSQL lease rather than promoting the observation itself.
It marks a pool member dirty before `BEGIN`, locks the exact public-table set before the first repeatable-read snapshot, and binds the identity, complete rich tables, database, role, session and policy context.
A primary-key-backed candidate is generated under that protection, rechecked, rolled back and stored only as an inseparable generation expectation.
For each internal verified request, one compiler permit is reserved before source I/O, retained across lease acquisition without requeue, and used for authoritative compilation after exact reobservation. The request reserves exactly 34 source-work units from typed executable inventories; 33 rejects before pool I/O.
Required-live evidence carries mapped SELECT, ASK and CONSTRUCT through request generation, retained-permit compilation and the same lease-owned connection. Final recheck plus acknowledged rollback is mandatory; any cancellation, error or drop that prevents rollback triggers bounded native cancel and dirty-member detachment.
The same required-live path now rejects any runtime login that is an owner, can inherit or change roles, carries elevated or mutation privileges, or lacks the exact CONNECT, `public`-schema USAGE, mapped-table SELECT and collation-probe execution capabilities; it also proves the permitted reads and denied `SET ROLE`, DDL and DML against a disposable database.
The PostgreSQL 16.15 gate also covers lock-before-snapshot, DDL barriers, old-generation coherence, successor drift, policy mutation, cancellation and dirty replacement.
The identity remains forgeable content equality and does not itself grant type, constraint, mapping, cache, readiness or execution authority; compiler facts remain `Unverified`.
Public startup still rejects every Direct Mapping selection before connector I/O. Qualification receipts do not alter startup and grant no reload lifecycle or backend admission.
Qualification never silently extends to another patch; SQLite and MySQL remain unavailable. Product code is Rust, while Node/MetaHarness remains development evidence with learning, evolution and promotion disabled.

## Context

ADR-0050 implemented a pure, bounded canonical hashing kernel but registered no database profile. PostgreSQL
startup already captures a legacy `Vec<TableSchema>` in one read-only repeatable-read transaction. That DTO is not
an identity input: it has unqualified names, lossy display types, no explicit ordinals or constraint states, and
mixes semantic facts with statistics. Its `information_schema` queries are unsuitable for durable identity:
`columns` is privilege-filtered and `table_constraints` omits PK/UNIQUE facts for a role holding only `SELECT`.
Phase 2 adds an independent `pg_catalog` projection in the same snapshot. Within the explicit resource ceiling,
legacy values and ordering remain unchanged; crossing the ceiling fails startup instead of allocating unboundedly.

## Decision

### 1. Register one closed profile

The repository owner reserves the
`io.github.sparkling.semantic-fabric.` profile prefix for this project. The
closed registry contains only `Postgres16PublicBaseV1`, bound to these immutable
Appendix-A profile IDs:

| Digest domain | Exact profile ID | UTF-8 bytes |
|---|---|---:|
| Structural | `io.github.sparkling.semantic-fabric.pg16-pb.structural-v1` | 57 |
| Type | `io.github.sparkling.semantic-fabric.pg16-pb.type-v1` | 51 |
| Constraint | `io.github.sparkling.semantic-fabric.pg16-pb.constraint-v1` | 57 |

The adapter selects this triple. A caller may invoke the pure `sf-core` builder
with the same ID bytes, but cannot select an adapter profile or construct its
runtime-accepted brand. The profile's initial closed candidate-engine set is
`{160009,160015}`. It becomes the qualified observation set only after both exact
receipts pass and this ADR's promotion is accepted. Another
16.x patch is `Unavailable(UnqualifiedEnginePatch)` until separately qualified;
another major is `Unavailable(ProfileNotImplemented)`. Adding a patch proven to
have the same law does not change profile IDs; any scope or normalization change
requires new affected IDs. Exact server version is a receipt fact, not a digest
input.
The public `ObservedSchemaIdentityV1::build` remains intentionally forgeable
content hashing. Runtime availability accepts only one opaque whole observation
constructed by the registered `sf-sql` adapter and published after commit. It
never accepts caller-supplied profile IDs, identities, individual digests,
legacy DTOs or availability booleans.

### 2. Freeze the session and atomic capture

Every identity catalogue reference is explicitly `pg_catalog`-qualified.
`information_schema`, `version()`, `format_type`, `pg_get_*`, `reg*::text`,
driver display strings, OIDs and object addresses never define identity bytes.

One outer `REPEATABLE READ READ ONLY` transaction performs this order:

1. set transaction-local deadlines, then query and decode one guard row;
2. abort on search-path, client-encoding or identifier-length mismatch; record
   other clean guard mismatches as pending unavailability, then collect legacy;
3. only when guards admit the profile, establish a savepoint, collect the rich
   projection, compare legacy/rich coordinates, normalize and build one identity;
4. on rich success, release the savepoint; on rich failure, roll back to and
   release it, retaining only a closed unavailable code;
5. commit the outer transaction; and only then publish availability or explicit
   unavailability with the complete legacy result.

The guards require:

- PostgreSQL `server_version_num` in the closed qualified-engine set;
- `server_encoding=UTF8` and `client_encoding=UTF8`;
- `max_identifier_length=63`, `max_index_keys=32`, `integer_datetimes=on`, and
  `session_replication_role=origin`;
- the exact runtime `search_path` setting `pg_catalog,public,pg_temp`;
- exactly one `pg_namespace` row named `public`; and
- exactly one `pg_database` row for `pg_catalog.current_database()`.

`statement_timeout=5s` and `lock_timeout=1s` are set immediately after begin and
bound guard, legacy and rich SQL. Because they belong to the outer transaction,
savepoint rollback retains them through the immediate commit. No retry combines
rows from different snapshots.
A rich query, decode, bound, unsupported-feature or kernel-build failure may
downgrade only after savepoint recovery and successful outer commit. This is
safe in Phase 2 because identity is non-authorizing and unused by compilation,
cache identity and readiness. The closed reason must be operator-visible at
startup. Outer begin/timeout setup, pre-savepoint guard SQL/decode, the three
fatal value mismatches, legacy, savepoint creation/release/recovery or commit
failure aborts startup. Another clean guard mismatch commits unavailability; an
unsupported major and unqualified 16.x use their dedicated variants. No partial,
stale or pre-commit state escapes. A later required phase rejects activation.

### 3. Freeze names and the public base-table scope

Relation names are:

```text
catalog = None
schema  = Some("public")
local   = exact pg_class.relname UTF-8
kind    = BaseTable
```
Native type names are `(None, Some("pg_catalog"), pg_type.typname)`. Database
identity remains separately bound through the mapping `SourceId`; cloning or
renaming a database does not change schema-content identity. Names undergo no
folding, normalization, trimming, quoting or search-path resolution. OIDs are
transaction-local join handles only.
The candidate probe enumerates all `public` `pg_class` rows with `relkind` in
`('r','p','f')`; it does not pre-filter unsupported table-like objects. A row is
admitted only when all are true:

```text
relkind='r'             relpersistence='p'
relisshared=false       relispartition=false
relrowsecurity=false    relforcerowsecurity=false
reloftype=0             relrewrite=0
relam joins pg_catalog.pg_am with amname='heap' and amtype='t'
no pg_inherits row names it as child or parent
```
Any partitioned table, partition, foreign table, unlogged/temporary table,
typed table, inheritance participant, RLS table, non-heap table or in-progress
rewrite makes the whole rich observation unavailable. Views and materialized
views are explicitly outside the relation set and ignored, as are indexes,
sequences, TOAST relations, routines and unused types. This profile therefore
cannot verify view, function or raw-SQL dependencies. An empty admitted scope is
valid; an admitted table with no visible user columns is unavailable.

### 4. Freeze columns and legacy agreement

For every admitted relation, the attribute query reads all `pg_attribute` rows with `attnum > 0`, including dropped
rows, in raw-`attnum` order. It uses left joins for type/collation facts so a dropped slot with `atttypid=0` remains
visible. Positive `attnum` values must be unique, contiguous through `pg_class.relnatts`, and bounded. A dropped row
requires `attisdropped=true` and `atttypid=0`; joined type facts may be NULL only there. Joined collation facts are NULL
for dropped or live `attcollation=0` rows and must be the exact default row for live character columns. Dropped rows are
excluded before name, type or collation normalization. Every live column requires `attislocal=true`,
`attinhcount=0`, and exact UTF-8 `attname`; each relation has at most 1,600 positive physical attributes. Each live
column receives a fresh dense one-based V1 ordinal in raw-`attnum` order. Constraint arrays use the same immutable
raw-attnum-to-`(dense ordinal, exact name)` map. Raw `attnum`, OIDs, storage, compression, statistics, defaults,
identity/generated expressions, ACLs and missing-value fields never enter a digest.
Before identity can be available, the legacy and rich projections must contain
the same relation-name set and, for each relation, the same visible column names
in dense ordinal order. Legacy types, constraint visibility and statistics are
deliberately not compared because their representations and privilege rules
differ. Coordinate disagreement yields `LegacyCoordinateMismatch`, so an
available identity never describes a larger or differently ordered compiler
schema.

### 5. Freeze type and typmod normalization

Every visible `atttypid` must resolve to the exact PostgreSQL-16 bootstrap
OID/name pair below, as a defined non-domain base type in `pg_catalog`, with no
base, element, relation-type or array dimension: `typbasetype=0`, `typelem=0`,
`typrelid=0`, and `attndims=0`. OIDs are guards and join handles only, never
digest inputs. Every unlisted type is unavailable, never `Opaque`.

| Native name (exact OID) | V1 family | Exact required non-collation facets |
|---|---|---|
| `bool` (16) | Boolean | none |
| `int2` (21), `int4` (23), `int8` (20) | SignedInteger | `bit-width=U64(16/32/64)` |
| `numeric` (1700) | ExactNumeric | optional pair below |
| `float4` (700), `float8` (701) | ApproximateNumeric | `bit-width=U64(32/64)` |
| `text` (25), `varchar` (1043), `bpchar` (1042) | Character | optional maximum plus section 6 |
| `bytea` (17) | Binary | none |
| `date` (1082) | Date | none |
| `time` (1083), `timetz` (1266) | Time | precision and `with-time-zone=Bool` |
| `timestamp` (1114), `timestamptz` (1184) | Timestamp | precision and `with-time-zone=Bool` |
| `json` (114), `jsonb` (3802) | Json | none; native name distinguishes them |
| `uuid` (2950) | Uuid | none |
Every admitted type except `numeric`, `varchar`, `bpchar`, `time`, `timetz`, `timestamp` and `timestamptz` requires
`atttypmod=-1`. Raw typmods never enter identity.
`varchar` and `bpchar` normalize as follows:

```text
-1 => character-maximum absent
>=5 => n=atttypmod-4; require 1<=n<=10,485,760; emit character-maximum=U64(n)
otherwise => unavailable
```

`numeric` uses `x=atttypmod-4`; `-1` emits neither numeric facet. Otherwise it
requires `atttypmod>=4`, computes
`p=(x>>16)&0xffff` and `s=((x&0x7ff)^0x400)-0x400`, requires
`1<=p<=1000` and `-1000<=s<=1000`, and requires exact re-encoding
`atttypmod=((p<<16)|(s&0x7ff))+4`. It then emits
`numeric-precision=U64(p)` and `numeric-scale=I64(s)`.
For `time`, `timetz`, `timestamp` and `timestamptz`, typmod `-1` normalizes to
`fractional-second-precision=U64(6)`; `0..=6` emits that value; anything else is
unavailable. The type also emits `with-time-zone=false` for `time`/`timestamp`
and `true` for `timetz`/`timestamptz`. No additional facet key is permitted.

### 6. Freeze character collation projection

Character columns require `attcollation=100`, and bootstrap OID 100 must resolve
to the single exact `pg_catalog.default` row with `collprovider='d'`,
`collencoding=-1`, and `collisdeterministic=true`. OID 100 is a guarded catalogue
coordinate only and never enters identity bytes. Its database provider must be
`c` or `i`. Every non-character admitted type requires `attcollation=0`.
Explicit, user-created and nondeterministic collations are unavailable, even if
a superuser created one in `pg_catalog`. Provider `c` requires NULL
`daticulocale` and `daticurules`; provider `i` requires non-NULL `daticulocale`
and permits optional `daticurules`.
Every character type emits:

```text
collation-name          = TypeName(None, Some("pg_catalog"), "default")
collation-provider      = Text("libc" | "icu")
collation-deterministic = Bool(true)
```

Provider and optional exact facets come from the matching `pg_database` row:
`datlocprovider`, `datcollate`, `datctype`, `daticulocale`, `daticurules`, and
`datcollversion` map to `collation-provider`, `collation-collate`,
`collation-ctype`, `collation-icu-locale`, `collation-icu-rules`, and
`collation-version`. SQL NULL means facet absence; an empty string remains
present.
The recorded version must be null-safely equal to
`pg_database_collation_actual_version(database_oid)`. The actual result is a
one-shot, volatile host guard and never a digest input or snapshot/lease claim.
The recorded version is intentional catalogue state: refreshing it after a
collation-library change may change type identity because comparison semantics
may have changed. This still grants no host-collation or execution authority.
A provider upgrade whose actual version differs null-safely from the recorded
value makes this profile unavailable until that value is deliberately refreshed.

### 7. Freeze constraints and state

Each visible `attnotnull=true` emits NOT NULL with
`{validated:true,enforced:true}`. PK, UNIQUE and FK facts come from
`pg_constraint`; names and OIDs never enter identity. All require local,
non-inherited, non-domain, non-parent constraints and reject deferrable or
initially deferred state. `conkey`/`confkey` arrays must be nonempty,
duplicate-free, resolvable and at most 32 members; equality arrays may repeat
OIDs but share that pre-copy bound.
PK and UNIQUE support indexes must be on the admitted relation, non-expression,
non-partial, immediate B-tree indexes whose key prefix exactly matches
`conkey`. Key attributes must use the built-in `pg_catalog` default B-tree
opclass and the column's collation. `opcmethod` joins `pg_catalog.pg_am` and
requires `amname='btree',amtype='i'`; the opclass belongs to `pg_catalog` and is
default. `opcintype` equals the column type except for the explicit
`varchar -> text` default-opclass case. `indclass` and `indcollation` must match
that opclass and the column. `indnkeyatts` must equal `conkey` arity, with exact
elementwise key-prefix equality; `indnatts>=indnkeyatts` and `indnatts<=32`, and
only attributes after `indnkeyatts` may be identity-invisible INCLUDE columns.
PK requires `indisprimary`; UNIQUE forbids it. Both require `indisunique`,
`convalidated`, `indisvalid`, `indisready`, and
`indislive`, then emit:

```text
validated = true
enforced  = true
```

PK preserves declared member order. UNIQUE member order is intentionally
canonicalized by Appendix A. UNIQUE null semantics comes exactly from
`indnullsnotdistinct`.
For an FK, child and parent must both be admitted relations. `conkey` and
`confkey` must have equal arity and resolve pairwise through their dense maps;
child, parent and pair duplicates reject. Corresponding source types and facets
must be equal. The `conindid` parent index must satisfy the same default B-tree,
collation and shape rules, with `indnkeyatts=confkey` arity and exact complete
key equality, and be unique, valid, ready and live. The
three equality-operator arrays must have the same arity. At each position all
three OIDs must be identical and equal the selected opfamily's search operator
where `amopstrategy=3`, `amoppurpose='s'`, and both operand types equal
`opcintype`. That operator must be binary `pg_catalog.=`, return `bool`, and have
the derived operand signature.

`confmatchtype` maps only `s` to Simple and `f` to Full. PostgreSQL 16 does not
implement MATCH PARTIAL, so `p` and any other code are unavailable. Update and
delete action codes must be one of `a`, `r`, `c`, `n`, or `d`; they are
intentionally identity-invisible because Appendix A cannot encode them.

An FK must own exactly these four internal, nondeferrable, nondeferred
`pg_trigger` rows, identified structurally rather than by name:

| Trigger relation | `tgtype` | Event |
|---|---:|---|
| child | 5 | AFTER ROW INSERT |
| child | 17 | AFTER ROW UPDATE |
| parent | 9 | AFTER ROW DELETE |
| parent | 17 | AFTER ROW UPDATE |

The child functions are exactly `pg_catalog.RI_FKey_check_ins()` and
`pg_catalog.RI_FKey_check_upd()`. Parent functions are exact:

| Action code | DELETE function | UPDATE function |
|---|---|---|
| `a` | `RI_FKey_noaction_del()` | `RI_FKey_noaction_upd()` |
| `r` | `RI_FKey_restrict_del()` | `RI_FKey_restrict_upd()` |
| `c` | `RI_FKey_cascade_del()` | `RI_FKey_cascade_upd()` |
| `n` | `RI_FKey_setnull_del()` | `RI_FKey_setnull_upd()` |
| `d` | `RI_FKey_setdefault_del()` | `RI_FKey_setdefault_upd()` |

Each resolved `pg_proc` row belongs to `pg_catalog`, takes zero arguments and
returns `trigger`.

For a self-referencing FK, all four rows attach to the same relation. They are
still one exact multiset of complete relation, `tgtype`, function and action
roles; no check assumes child and parent relation OIDs are distinct.
Each trigger requires `tgconstraint` equal to the FK, `tgconstrrelid` equal to
the opposite relation, `tgconstrindid=conindid`, `tgparentid=0`,
`tgisinternal=true`, `tgdeferrable=false`, `tginitdeferred=false`, `tgnargs=0`,
empty `tgattr`, NULL `tgqual`, and NULL transition-table names. Missing, extra
or malformed rows are unavailable. In guarded `session_replication_role=origin`,
`tgenabled` `O` or `A` counts as enabled and `D` or `R` as disabled; another
value is unavailable. FK state is:

```text
validated = convalidated
enforced  = all four triggers are O or A
```

CHECK/exclusion constraints, FK actions, defaults, generated expressions,
standalone indexes, included index columns and index ordering options are
explicit V1 blind spots: changing only them changes no digest and grants no
authority. Table-like objects that could be mistaken for admitted base tables
reject; facts outside Appendix A's admitted relation/column/key model are
explicitly identity-invisible. This is not a complete PostgreSQL DDL digest.

### 8. Bound every collection before public allocation

The rich path uses a fixed O(1) query set and no N+1 reads:

| Query | Server result bound | Adapter checks before retention |
|---|---:|---|
| guard | exactly 1 | exact session/profile values |
| table-like candidates | `MAX_RELATIONS_V1+1` | candidate/admitted relation cap |
| positive attributes + type/collation | `MAX_PHYSICAL_ATTRIBUTES_TOTAL_PG16_V1+1` | physical per-relation/total and live-column caps, bounded text |
| `p/u/f` + index/FK-trigger aggregates | remaining combined budget + 1 | streamed sentinel before decode; array arity before copy |

`MAX_PHYSICAL_ATTRIBUTES_TOTAL_PG16_V1` is 65,536, distinct from the equal-valued
live-column cap because dropped slots consume only the former. Queries use
server-side `LIMIT cap+1`, typed bound parameters, bounded string
projections and arrays that become NULL plus an overflow flag before oversized
payload transfer. FK triggers are fixed-size aggregates, not expanded result
rows; each correlated trigger input stops at five rows (four expected plus one
overflow sentinel) before aggregation.
Live NOT NULL facts are emitted first. Their count must not exceed `MAX_RAW_CONSTRAINTS_V1`; the catalogue query then
receives `remaining=MAX_RAW_CONSTRAINTS_V1-not_null_count` and `LIMIT remaining+1`. The evidence contract records NOT
NULL, catalogue and combined counts separately, plus each stream's cap, polled, decoded, retained-peak and overflow
values, so a cap-plus-one row cannot masquerade as decoded or retained input.

`MAX_LEGACY_RELATIONS_PG16_V1` is 4,096. It bounds both snapshot enumeration and caller-supplied table slices. Before
clone, allocation or SQL, each caller name must be at most 256 UTF-8 bytes and their checked cumulative length at most
1,048,576 bytes; oversized input rejects. `MAX_LEGACY_ROWS_PER_SET_PG16_V1` is 65,536 for
each columns, PK/UNIQUE-member, FK-member and statistics result. Each set query streams at most cap plus one. Every
text projection is length-checked and capped at 256 UTF-8 bytes server-side before transfer; overflow is fatal, not
truncated. The single-table entry point uses the same bounded collector. Crossing any legacy cap is fatal because a
partial schema is unsound; this rejects extreme schemas that previously attempted unbounded allocation while
preserving in-bound values.
The adapter enforces every Appendix-A production maximum, including cumulative
facet counts, semantic UTF-8 payload-occurrence bytes and conservative
pre-deduplication 64-MiB body accounting. It invokes the public kernel exactly
once after a complete valid projection, never after a guard, decode or bound
failure. Error precedence among simultaneous defects is not normative.

### 9. Keep runtime availability closed and non-authorizing

The Phase-2 adapter returns an opaque committed snapshot containing the complete legacy vector and either a branded
registered whole identity or a closed unavailable reason. The closed algebra has no free-form payload: `Display` and
`Debug` are at most 256 UTF-8 bytes, contain no identifiers, SQL, connection material, paths or values, and
`Error::source()` is `None`.
The top-level variants are `ProfileNotImplemented`, `UnqualifiedEnginePatch`,
`GuardUnsupported(GuardCodeV1)`, `LegacyCoordinateMismatch`, `CatalogQuery`,
`CatalogDecode`, `LimitExceeded(LimitCodeV1)`, `UnsupportedRelation`,
`UnsupportedType`, `UnsupportedCollation`, `UnsupportedConstraint`, and
`IdentityRejected`. `GuardCodeV1` is `{ServerEncoding, ClientEncoding, IdentifierLength, IndexKeyLimit,
IntegerDatetimes, ReplicationRole, SearchPath, PublicNamespace, CurrentDatabase}`. `LimitCodeV1` is `{RichRelations,
PhysicalAttributes, LiveColumns, RawConstraints, KeyMembers, Facets, TextBytes, CanonicalBody}`. Legacy-cap failure is
fatal, outside this unavailable algebra.
Both nested code types are identifier-free enums.
The target public snapshot API returns that opaque result. Existing legacy `sf-sql` functions `introspect_postgres`,
`introspect_postgres_all` and `introspect_postgres_public_snapshot` preserve their signatures; public `sf-serve`
`introspect_pg_all` preserves `Result<Vec<TableSchema>, String>`. They use the bounded legacy collector, and the first
two retain caller-supplied transaction semantics. `introspect_postgres_public_observed_snapshot` now returns committed
availability, startup calls it, and explicit callers may discard its observation through `into_legacy_tables`.
The `sf-serve` crate-private PostgreSQL opener consumes the opaque snapshot into an `IntrospectedSource` private
`observation: SourceSchemaObservationV1` field. That closed private enum is either `Unavailable` (retaining an optional
closed PostgreSQL reason) or carries the whole `Postgres16PublicObservedSchemaV1`; unchecked, compatibility, SQLite and
MySQL constructors can create only reason-free `Unavailable`.
`RuntimeBinding` has a private `schema_observation: BoundSourceSchemaObservationV1` field holding backend kind,
`mapping.source_id()` and that state. `RuntimeBinding::new` binds it before consuming the mapping; no public or
compatibility constructor accepts an identity, brand or availability argument. The sole production `into_parts`
consumer destructures and moves its fourth state; there is no identity-dropping production compatibility tuple.
Both states continue through `CompilerSchema::from_unverified_observation`.
For ordinary authored mappings, identity does not enter `CompileScope`, cache keys, admission, readiness, reload or execution. The private Direct-Mapping generation expectation retains and rechecks it as one exact fact, but authority comes only from the owned lease type-state and connection.
That use neither upgrades `CompilerSchema` nor makes the digest a capability. Equal identities in separate runtime bindings do not merge process-local binding authority.
Startup emits one bounded structural availability diagnostic through ADR-0011's product JSON subscriber and configured INFO ceiling; unavailable output contains only its closed reason and never a source error. Server-encoding mismatch remains non-authorizing unavailability; client encoding, identifier length and search path remain fatal because they can invalidate legacy decoding or name resolution.

## Required evidence

### Qualification receipt contract

Each qualified engine patch requires one canonical, replayable JSON receipt with
`receiptKind=postgresql-public-observation-qualification-v1`. It binds source commit/tree and Cargo lock; ADR/profile/
query/test/fixture/runner/protocol inputs; the complete immutable builder manifest/configuration, Rust/Cargo outputs and probe artefact; PostgreSQL OCI repository, platform, immutable manifest and configuration digests; exact server version and role preflight; separate relation, attribute, NOT NULL, catalogue and
combined-constraint counts; each stream's cap/polled/decoded/retained-peak/overflow/terminal state; the three identities; legacy
relation/column-coordinate comparison; required `null` error/failure sentinels; bounded output digests; distinct execution resources; cleanup; and
`replayStatus`. It contains no credentials, SQL payloads, OIDs, names or raw rows. Replay runs each pinned image twice in
fresh networkless containers and requires every stable candidate byte to agree. Its two axes are deliberately separate:
`observationProfileQualification=pass` reports only this exact observation law; any failed run emits no receipt, while
`runtimeAdmissionStatus=withheld-independent-gates`, `productionAdmission=false`, `verifiedLease=false`, `reload=false`,
and `directMapping=false` deny every wider runtime claim. Both patches must pass, bind the same qualification-critical
source/input closure, and agree cross-patch before the observation profile may be promoted.

- exact registry IDs, grammar, backend binding, guard/failure matrix, legacy Vec relation/column-coordinate compatibility, qualified/unqualified-patch tests, and a PostgreSQL-16 catalogue-column inventory comparison for 16.9 and 16.15;
- table-driven normalization/rejection for every type and exact typmod boundary: fixed `-1/other`, character 4/5/max/max+1, numeric minima/maxima/outside/noncanonical, temporal `-2/-1/0/6/7`, implicit/explicit precision-6 equality, overlong `daticurules`, each default-collation provider and each constraint state;
- instrumented exact-cap/cap-plus-one tests proving at most one sentinel is polled and no sentinel or oversized text/array is copied; oversized caller input issues zero SQL, keys test 32/33, FK aggregation 4/5, and dropped slots consume only physical capacity;
- transaction/redaction/kernel-call fault injection, including fatal legacy caps, failed savepoint recovery and commit;
- a live/dropped/live matrix proving dense ordinals and PK/UNIQUE/FK remapping; reject dropped/nonzero-type, live/NULL-type, character/NULL-collation, duplicate/gapped/out-of-range `attnum`, `relnatts` mismatch and keys naming a dropped slot, while accepting NULL collation facts for live `attcollation=0`;
- on both images, prove the comparison role is non-superuser/non-owner, cannot inherit, bypass, `SET ROLE`, CREATEROLE, CREATEDB, REPLICATION or DDL, and has only required CONNECT/USAGE/SELECT plus callable `pg_database_collation_actual_version(oid)`; then prove owner identity equality while role-visible legacy constraints differ;
- statistics/data/ACL/owner/OID/name-only noninterference, explicit blind-spot tests, and isolated mutations for each digest domain;
- unsupported relation/type/collation/constraint and malformed-catalogue cases produce closed unavailability, never partial identity;
- end-to-end and compile-fail tests prove only the committed opaque snapshot creates `Available`; carrier state and exact backend/`SourceId` survive `IntrospectedSource -> RuntimeBinding`; no production compatibility tuple drops state; unchecked/compatibility/SQLite/MySQL paths cannot inject it; compiler authorities remain `Unverified`; and equal identities in distinct bindings do not merge scopes;
- deterministic old-or-new DDL barriers without sleeps; and
- two fresh, ownership-labelled, `--network none` containers for each pinned PostgreSQL 16.9 and 16.15 digest, with a fixed database, Unix-socket execution, byte-equal replay summaries and verified cleanup.

The tracked pair also binds immutable builder/toolchain, fixture/runner bytes, image configuration, preflight/result, bounded stdout/
stderr, container/volume distinctness and cleanup. Its authority is observation-profile qualification only; it grants no
Direct Mapping, generation lease, reload, backend admission, production-readiness or release authority.

No test may connect to, mutate or use live product-mock. Its static source and semantic-builder gold may inform fixtures
but grant no runtime authority. Existing sealed evidence covers eleven PostgreSQL-16.9 databases and a `public` table/
column inventory, but permits partitions and omits this profile's type, collation, constraint and guard laws. It qualifies
no product-mock database; each needs isolated evidence. Multi-database composition is an extension, not a rewrite.
Node 20/24 MetaHarness protects inputs and replays receipts with development-only authority, evolution ineligible, and
learning/promotion disabled. Node remains outside the product closure.

## Consequences

- PostgreSQL gains repeatable rich identity without changing compiler authority or in-bound legacy projection values.
- Exact scope, type, collation, constraint and collection laws prevent catalogue implementation choices becoming an undocumented wire contract.
- Dense ordinals reconcile dropped PostgreSQL attributes with Appendix A.
- Initial availability is narrow; unsupported schemas retain serving behavior but expose a redacted unavailable status.
- Extreme legacy catalogues that exceed the new safety caps fail startup rather than consuming unbounded memory.
- Multi-schema observation and later authorizing phases extend the closed registry without reversing dependencies.

## Alternatives rejected

- **Hash `TableSchema`** — lossy, unqualified and statistics-sensitive.
- **Use `information_schema` for identity** — privilege-filtered and role-variant.
- **Derive the legacy DTO from rich rows now** — risks changing compiler display types and serving behavior during an observational phase.
- **Let callers attach a digest** — the pure builder is not provenance.
- **Use raw `attnum` as V1 ordinal** — dropped columns violate continuity.
- **Silently filter table-like candidates** — produces falsely complete scope.
- **Hash an actual-collation function result directly** — makes an ephemeral host probe a durable input; the recorded catalogue version is hashed instead.

## Rules

- **R1** — only the closed PostgreSQL adapter publishes availability; compatibility APIs never do.
- **R2** — identity uses bounded `pg_catalog` facts from the committed legacy repeatable-read snapshot.
- **R3** — transient PostgreSQL identifiers never enter digests; live columns use dense ordinals and database name is source binding only.
- **R4** — unsupported or incomplete observations never emit partial/stale identity or upgrade compiler authority.
- **R5** — product/runtime implementation is Rust; Node is evidence-only.
- **R6** — Phase-2 downgrade is permitted only while identity is non-authorizing; a requiring phase fails closed.

## Links

[ADR-0006](ADR-0006-crate-layout-and-performance-model.md),
[ADR-0015](ADR-0015-datatype-dialect-correctness.md),
[ADR-0038](ADR-0038-sota-application-completion-programme.md),
[ADR-0048](ADR-0048-rust-production-and-node-evidence-runtime-boundary.md), and [ADR-0050](ADR-0050-verified-source-generation-leases-schema-identity-and-atomic-runtime-activation.md).
