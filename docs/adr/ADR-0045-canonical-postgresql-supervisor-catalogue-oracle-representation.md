---
status: proposed
date: 2026-08-30
updated: 2026-08-30
tags: [postgresql, supervisor, catalogue, canonical-json, verification]
supersedes: []
depends-on: [ADR-0038, ADR-0039, ADR-0042, ADR-0043, ADR-0044]
implements: [ADR-0044]
---

# Canonical PostgreSQL supervisor catalogue oracle representation

## Status boundary

This ADR is **proposed**. It closes representation and authority ambiguities in
ADR-0044 so its dormant catalogue artefact can be implemented. It does not
accept ADR-0042 through ADR-0044, activate the supervisor, create SQL, provision
a database or credential, contact PostgreSQL, or grant migration, readiness,
network, signer, publication, repair, or release authority.

The catalogue remains reviewed repository input. Model-assisted source edits
are permitted, but no edit becomes authority before explicit independent
review and commit. Observed state, fixtures, generators, verifiers, and
unreviewed automation may never self-authorize, refresh, or replace it.

## Context

Independent architecture, adversarial, security, and test-gap reviews agreed
that ADR-0044 fits the existing sealed supervisor architecture but found three
blocking ambiguities:

1. `catalog-contract-v1.json` had no exact wire grammar, root order, record
   schemas, collection comparators, or duplicate identities;
2. catalogue and provisioning expectations overlapped, so a verifier could
   silently choose which artefact was authoritative; and
3. prose constraints and the 27 aliases did not seal exact constraint tuples,
   implicit PostgreSQL objects, or the query join topology.

The reviews also found two fail-closed requirements: OIDs and raw ACL order
cannot be identities, and resource limits must apply before allocation or
recursive role traversal. Reusing the general build JSON reader would be
unsafe because it does not perform fatal UTF-8 decoding.

## Decision

### 1. Separate four authorities

The four artefacts have disjoint normative responsibilities:

| Artefact | Sole authority |
|---|---|
| `catalog-contract-v1.json` | Dedicated-schema objects, columns, named constraints, indexes, generated FK-trigger semantics, RLS, policies, schema/object/column/default ACL states, implicit-object closure, and exact-result query topology. |
| `provisioning-contract-v1.json` | Server/database identity, database name and owner, `public` schema owner/ACL, bootstrap grantor, nine role attributes/settings, seven membership edges/options, database/parameter/tablespace privileges, effective CONNECT, forbidden predefined-role membership, service ownership/inbound grants outside the dedicated schema, and runtime-credential absence attestation. |
| `manifest-v1.json` | Exact raw digests and lengths of both contracts, ordered migration scripts, seed, server version, and advisory-lock identity. |
| readiness receipt | Digests of the three sealed inputs plus separately normalized catalogue and provisioning observations; never new authority. |

The catalogue may name a role where an owner, policy target, ACL grantor, or ACL
grantee is a schema-local fact. It does not duplicate that role's attributes,
settings, membership, bootstrap grantor, database access, or credential facts.
The provisioning contract owns those facts. Combined readiness resolves every
catalogue role reference exactly once against provisioning and fails if either
projection is absent, extra, or inconsistent.

The literal dedicated schema is `sf_supervisor_v1` and its literal owner
reference is `sf_supervisor_owner_v1`. These two cross-contract identity
constants are equal by a combined-verifier invariant; neither contract may
substitute the other's missing facts.

### 2. Define exact catalogue bytes

The catalogue domain is
`semantic-fabric/programme-capture/supervisor-postgresql-catalogue-oracle-v1`,
and `schemaVersion` is the JSON number `1`. PostgreSQL version is absent here:
the manifest alone owns `server_version_num = 160015` and binds this digest.

The file uses **root-line canonical JSON plus LF**, not RFC 8785 and not a
generic pretty-printer:

1. bytes are fatal UTF-8 with no BOM;
2. the first bytes are `{\n` and the final bytes are `}\n`;
3. every root member occupies one physical line in the fixed order below;
4. each line is two spaces, a JSON-string key, `: `, the compact
   `JSON.stringify` representation of its normalized value, an optional comma,
   and LF;
5. nested object keys use their record-schema order below; arrays are dense;
6. every nullable field is present as explicit `null`;
7. no insignificant whitespace, CR, trailing space, alternate escape, unsafe
   number, negative zero, non-finite number, or unpaired surrogate is admitted;
8. duplicate object keys are rejected by a bounded token scan before
   `JSON.parse`; and
9. parsing succeeds only when schema reconstruction and re-encoding reproduce
   every input byte.

The exact root order is:

```text
domain
schemaVersion
schemaName
ownerRole
limits
schemas
domains
relations
columns
constraints
indexes
foreignKeyTriggers
policies
objectAcls
columnAcls
defaultAcls
implicitObjects
exactQueries
```

`limits` has keys `maximumBytes`, `maximumDepth`, `maximumNodes`,
`maximumRecords`, `maximumCollectionWidth`, `maximumObjectKeys`,
`maximumStringBytes`, and `maximumIdentifierBytes`, with exact values
`1048576`, `16`, `16384`, `4096`, `1024`, `32`, `196608`, and `63`.
Those values are both encoded authority and hard parser ceilings; a file cannot
raise its parser limits.
The lower `8192` proposal was rejected when the complete explicit M0 inventory measured `9125` nodes; no required fact was removed to fit a guessed ceiling.

The token scanner defines root depth as one; each nested array or object adds
one. A node is any JSON object, array, or primitive value; object keys are not
nodes. A record is any non-root object. Collection width is an array's element
count; object width is its own-key count. String bytes are fatal-UTF-8 bytes of
each decoded key or value, before normalization. Limits are tested at exact
maximum and maximum plus one.

The raw catalogue SHA-256 is calculated over the exact file bytes and stored
only in the manifest. The catalogue contains no self-digest.

### 3. Close every record schema

Unknown, missing, reordered, accessor, symbol, sparse, exotic-prototype, and
proxy-bearing values fail. Identifiers are literal 1..63-byte ASCII PostgreSQL
identifiers. Qualified names are never packed into a single string.

Every set-like collection is strictly increasing by its identity tuple and
rejects duplicate identity. Ordered tuple fields preserve their declared
order. The total comparator is type-aware: strings use unsigned-ASCII-byte lexicographic
order with a proper prefix first, numbers ascend, `null` precedes strings, and `false`
precedes `true`; tuples compare left-to-right. Locale, coercion, serialized JSON, and insertion order never compare.

| Collection | Exact record-key order | Identity/order |
|---|---|---|
| `schemas` | `name, owner, aclState, privileges` | `name` |
| `domains` | `schema, name, owner, baseTypeSchema, baseTypeName, typeModifier, collationSchema, collationName, notNull, defaultTemplate, defaultExpression, checks` | `schema, name` |
| domain `checks` | `name, template, expression` | `name` |
| `relations` | `schema, name, kind, persistence, owner, accessMethod, rowSecurityEnabled, rowSecurityForced, replicaIdentity, relOptions, toastState` | `schema, name` |
| `columns` | `schema, relation, ordinal, name, typeSchema, typeName, baseProjectionType, notNull, defaultTemplate, defaultExpression, identityKind, generatedKind, collationSchema, collationName` | `schema, relation, ordinal` |
| `constraints` | `schema, relation, name, kind, columns, referencedSchema, referencedRelation, referencedColumns, matchType, updateAction, deleteAction, deferrable, initiallyDeferred, validated, definition, checkTemplate, expression` | `schema, relation, name` |
| `indexes` | `schema, name, relationSchema, relation, constraintName, accessMethod, unique, primary, immediate, nullsNotDistinct, clustered, replicaIdentity, valid, ready, live, keys, includedColumns, predicateTemplate, predicateExpression` | `schema, name` |
| index `keys` | `position, column, expression, collationSchema, collationName, opclassSchema, opclassName, direction, nulls` | ascending `position` |
| `foreignKeyTriggers` | `constraintSchema, constraintName, side, event, timing, orientation, triggerType, internal, functionSchema, functionName, deferrable, initiallyDeferred, enabled` | `constraintSchema, constraintName, side, event` |
| `policies` | `schema, relation, name, permissive, command, roles, usingTemplate, usingArguments, usingExpression, withCheckTemplate, withCheckArguments, withCheckExpression` | `schema, relation, name` |
| `objectAcls` | `objectKind, schema, object, owner, aclState, privileges` | `objectKind, schema, object` |
| `columnAcls` | `schema, relation, column, aclState, privileges` | `schema, relation, column` |
| `defaultAcls` | `owner, schema, objectClass, rowState, privileges` | `owner, schema, objectClass` |
| ACL `privileges` | `grantorRole, granteeKind, granteeRole, privilege, grantable` | all fields with `null` before strings |
| `exactQueries` | `name, root, parameters, joins, predicates, projection, maximumRows` | `name` |

Tagged policy/principal references use keys `kind, name`. Exact-query nested
records are also closed:

| Query value | Exact record-key order |
|---|---|
| `root` | `schema, relation, alias` |
| `parameters` item | `position, name, baseType` |
| `joins` item | `kind, leftAlias, rightSchema, rightRelation, rightAlias, columnPairs` |
| join `columnPairs` item | `leftColumn, rightColumn` |
| `predicates` item | `sourceAlias, column, operator, operandKind, operand` |
| `projection` item | `sourceAlias, column, cast, outputAlias` |

Each policy argument record has keys `scopeRole, capabilityRole, sessionLogin,
ownerRole`; fields are explicit identifier-or-null. `scope-equality-v1` requires
only scope, `scope-capability-v1` requires scope plus capability, and
`migration-session-owner-v1` requires scope, session login, and owner. A null
template requires null arguments and expression.

`implicitObjects.rules` has keys `arrayTypes, compositeRowTypes,
constraintIndexes, foreignKeyTriggers, toastObjects` and exact values
`raw-null-effective-element-acl-v1`, `explicit-owner-only-acl-v1`, `enumerated-constraint-index-v1`,
`four-internal-ri-triggers-v1`, and `parent-linked-unnamed-v1`. `allowedDerivedKinds` is exactly
`[array-type, composite-row-type, constraint-index, foreign-key-trigger, toast-index, toast-relation]`;
`forbiddenOwnedKinds` is exactly `[aggregate, cast, collation, conversion, extension, foreign-table,
function, materialized-view, operator, partition, procedure, publication, rule, sequence,
subscription, text-search-object, user-trigger, view]`. These are sorted literal enums.

All string leaves are ASCII. Scalars and enums are closed as follows:

| Family | Exact grammar |
|---|---|
| relation | `kind=table`, `persistence=permanent`, `accessMethod=heap`, `replicaIdentity=default`, `toastState=absent|linked`; options are sorted ASCII |
| domain/column | base/type names are qualified identifiers; projection is `bytea|text|boolean|integer`; PostgreSQL typmod `-1` maps to JSON null and every other modifier is a nonnegative safe integer; identity/generated kinds are empty; collation is both-null or qualified |
| constraint | `primary-key|unique|foreign-key|check`; non-FK reference/action fields are null; FK is `simple/restrict/restrict`; definition is ASCII, check template/expression are both-null or both-string |
| index | `btree`; immediate is true; nulls-not-distinct/clustered/replica-identity are false; a key has exactly one of column/expression; `asc/nulls-last`; collation is both-null or qualified; opclass is qualified; M0 predicate template/expression are null |
| trigger | `after/row`, internal true, function schema `pg_catalog`, enabled `origin`; referencing insert/update are `foreign-key-check`, use `RI_FKey_check_ins|RI_FKey_check_upd`, and mirror owning-FK deferral; referenced delete/update are `foreign-key-action`, use `RI_FKey_restrict_del|RI_FKey_restrict_upd`, and are never deferrable/initially deferred |
| policy | command `select|insert|update`; tagged role set; template/expression pairs are both-null or both-string |
| ACL | object kind `table|domain|type`; class `function|type|table|sequence|schema`; default-ACL schema is identifier-or-null and is null for all five M0 global expectations (two explicit, three absent); privilege `USAGE|CREATE|SELECT|INSERT|UPDATE|DELETE|TRUNCATE|REFERENCES|TRIGGER|EXECUTE`; booleans are JSON booleans |
| query | join `left`; operator `equals`; operand `parameter|literal`; base type/cast `bytea|text`; identifiers only |

Generator templates are fixed code enums, never interpolated SQL. Domain and
CHECK templates are the literal reviewed constraint names; policy templates are
`scope-equality-v1`, `scope-capability-v1`, or
`migration-session-owner-v1`. Default and predicate templates are null in M0.
The generator emits from structured values plus its exhaustive template switch;
stored deparse strings are comparison facts only.

`granteeKind` is exactly `role` or `public`. A role grantee has a non-null
`granteeRole`; PUBLIC has a null one. This tagged representation cannot confuse
PostgreSQL's OID-zero PUBLIC identity with a real role textually named
`PUBLIC`. Policy roles use the same closed tagged representation.

ACL state is explicit:

- `aclState` is `null` or `explicit` and always carries the normalized effective
  privilege atoms;
- a default-ACL `rowState` is `absent` or `explicit`; and
- an explicit empty ACL, a null ACL with effective defaults, and an absent
  default-ACL row are distinct and never normalized together.

`relOptions`, `includedColumns`, policy roles, and privilege atoms are sorted
sets. Constraint columns, referenced columns, index keys, query parameters,
joins, predicates, and projections are ordered semantic tuples.
`schemas` contains exactly the root `schemaName` and solely owns its ACL;
`objectAcls` cannot duplicate it.

### 4. Freeze normalization without trusting OIDs

Catalogue queries may use OIDs only as transaction-local join keys. Every OID
reference must resolve exactly once through a non-dropping join before a stable
qualified name enters an observation. An unresolved, multiply resolved, or
relinked OID fails; renumbering with unchanged stable identities does not.

The normalized oracle excludes the volatile facts already listed by ADR-0044.
It additionally applies these exact rules:

- constraint column arrays and index-key order remain semantic;
- `pg_get_expr(..., false)` and `pg_get_constraintdef(..., false)` bytes under
  `search_path=pg_catalog` are compared exactly on PostgreSQL 16.15;
- equivalent predicates with reordered conjunctions, alternate casts,
  redundant wrappers, or different referenced identities fail;
- raw ACL and policy-role array order is discarded only after expansion to
  stable tagged atoms;
- membership transitivity is never inferred into the catalogue; it belongs to
  provisioning;
- generated FK trigger names and OIDs are discarded, but each `tgconstraint` resolves the owning FK, `tgrelid`/`tgconstrrelid` resolve the exact side/counterpart relations, `tgconstrindid` resolves that FK's supporting PK/UQ index, and `tgparentid=0`; side, event, timing, orientation, type, function, internal/deferral flags, and `tgenabled='O'` remain exact; raw `tgtype` has exactly one event plus AFTER/ROW, `tgattr`/`tgargs` are empty, argument count is zero, and qualifier/transition-table fields are null; and
- raw password, physical, activity, timestamp, statistics, freeze, tuple, and
  transaction state never enters either contract.

Observed row codecs, the contract-byte parser, the contract-to-SQL generator,
and the comparator are separate pure private modules. The generator reads only
reviewed contracts. It has no observation input and no code path that writes a
contract. The comparator cannot repair either side.

### 5. Close explicit and implicit object identity

The schema inventory is exactly ten domains and eight relations containing 88
ordered columns. The relation set is:

```text
schema_migrations
authority_configurations
authority_state
semantic_events
semantic_receipts
registration_runs
registration_results
publication_outbox
```

The committed JSON is the normative data appendix for every literal column,
constraint/check/index/trigger/policy/ACL record and expression. It must receive
independent architecture, security, and data review before parser or generator
implementation begins. Code validates the reviewed data; it never supplies a
missing name, tuple, enum, expression, or default.

The project scope tuple `S` is
`(project_authority_digest, project_scope_role)`. Exact key tuples are:

| Relation | Required primary/unique tuples |
|---|---|
| `schema_migrations` | PK `(migration_version)` |
| `authority_configurations` | PK `(S, configuration_epoch)`; UQ `(S, configuration_digest)`; UQ `(S, configuration_epoch, configuration_digest, genesis_authority_head_digest)`; UQ `(S, configuration_epoch, configuration_digest, genesis_semantic_receipt_digest)` |
| `authority_state` | PK `(S)`; database-wide UQ `(singleton_key)` |
| `semantic_events` | PK `(S, event_digest)`; UQ `(S, global_sequence)`; UQ `(S, global_sequence, event_digest)`; UQ `(S, run_id, run_sequence)`; UQ `(S, run_id, run_sequence, global_sequence, event_digest, resulting_controller_state_head_digest)`; UQ `(S, semantic_request_digest, run_id, event_digest)` |
| `semantic_receipts` | PK `(S, event_digest)`; UQ `(S, semantic_receipt_digest)`; UQ `(S, event_digest, semantic_receipt_digest)` |
| `registration_runs` | PK `(S, run_id)`; UQ `(S, run_id, original_registration_request_digest, original_registration_request_sha256, original_registration_event_digest)` |
| `registration_results` | PK `(S, semantic_request_digest)`; UQ `(S, current_event_digest)`; UQ `(S, run_id, semantic_request_digest)`; UQ `(S, run_id, response_status)` |
| `publication_outbox` | PK `(S, event_digest)`; UQ `(S, public_commitment_digest)` |

The exact foreign-key tuple inventory is:

| Source | Target | Timing |
|---|---|---|
| each of `semantic_events`, `semantic_receipts`, `registration_runs`, `registration_results`, `publication_outbox`: `(S)` | `authority_state(S)` | immediate |
| `authority_state(S, active_configuration_epoch, active_configuration_digest, authority_head_digest)` | `authority_configurations(S, configuration_epoch, configuration_digest, genesis_authority_head_digest)` | immediate |
| `authority_state(S, last_global_sequence, last_event_digest)` | `semantic_events(S, global_sequence, event_digest)` | deferred |
| `semantic_events(S, authority_configuration_epoch, authority_configuration_digest, authority_head_digest)` | `authority_configurations(S, configuration_epoch, configuration_digest, genesis_authority_head_digest)` | deferred |
| `semantic_events(S, run_id)` | `registration_runs(S, run_id)` | deferred |
| `semantic_events(S, previous_global_sequence, previous_global_event_digest)` | `semantic_events(S, global_sequence, event_digest)` | deferred |
| `semantic_events(S, run_id, previous_run_sequence, previous_run_global_sequence, previous_run_event_digest, prior_controller_state_head_digest)` | `semantic_events(S, run_id, run_sequence, global_sequence, event_digest, resulting_controller_state_head_digest)` | deferred |
| `semantic_events(S, previous_global_genesis_configuration_epoch, previous_global_genesis_configuration_digest, previous_global_genesis_receipt_digest)` | `authority_configurations(S, configuration_epoch, configuration_digest, genesis_semantic_receipt_digest)` | deferred |
| `semantic_events(S, previous_global_event_digest, previous_global_event_receipt_digest)` | `semantic_receipts(S, event_digest, semantic_receipt_digest)` | deferred |
| `semantic_receipts(S, event_digest)` | `semantic_events(S, event_digest)` | immediate |
| `registration_runs(S, original_registration_request_digest, run_id, original_registration_event_digest)` | `semantic_events(S, semantic_request_digest, run_id, event_digest)` | deferred |
| `registration_runs(S, run_id, last_run_sequence, last_run_global_sequence, last_run_event_digest, current_controller_state_head_digest)` | `semantic_events(S, run_id, run_sequence, global_sequence, event_digest, resulting_controller_state_head_digest)` | deferred |
| `registration_runs(S, run_id, original_registration_request_digest)` | `registration_results(S, run_id, semantic_request_digest)` | deferred |
| `registration_runs(S, run_id, first_changed_replay_request_digest)` | `registration_results(S, run_id, semantic_request_digest)` | deferred |
| `registration_results(S, run_id, original_registration_request_digest, original_registration_request_sha256, original_registration_event_digest)` | `registration_runs(S, run_id, original_registration_request_digest, original_registration_request_sha256, original_registration_event_digest)` | deferred |
| `registration_results(S, semantic_request_digest, run_id, current_event_digest)` | `semantic_events(S, semantic_request_digest, run_id, event_digest)` | deferred |
| `publication_outbox(S, event_digest)` | `semantic_events(S, event_digest)` | immediate |

All project primary, unique, and foreign keys include `S`. The sole exception is
the database-wide singleton unique constraint. Every FK uses `MATCH SIMPLE`,
`ON UPDATE RESTRICT`, `ON DELETE RESTRICT`, and is validated.

Scope, receipt-to-event, and outbox-to-event FKs are immediate. State-last-event,
every non-scope FK originating at `semantic_events`, and every circular run/result
provenance FK are `DEFERRABLE INITIALLY DEFERRED`, preserving ADR-0044. Literal
catalogue names, tuples, and flags are normative; naming conventions fill
nothing.

Run sequence one references run sequence zero and a strictly smaller prior
global sequence. It does **not** require the prior global sequence to equal the
current global sequence minus one because other runs may interleave. Global
event ancestry separately requires global adjacency.

PostgreSQL cannot recompute these application SHA-256 values without an
additional function or extension, both forbidden in M0. Database checks enforce
relational equality, status/kind/nullability/sequence laws, bounds, and literal
values. The materializer and exact-row codec retain cryptographic byte/hash
verification. No CHECK constraint may claim to recompute a digest.

`implicitObjects` has keys `allowedDerivedKinds`, `forbiddenOwnedKinds`, and
`rules`. It permits only:

1. one composite row type and its array type for each table: the composite has
   explicit owner-only ACL state, while its array has raw-null ACL state and
   owner-only effective atoms inherited from that exact composite;
2. one array type for each explicit owner-only domain, likewise raw-null with
   effective atoms inherited from that exact domain;
3. each exact constraint-backed index already represented in `indexes`;
4. TOAST relations/indexes linked to an enumerated eligible table, represented
   by that table's `toastState` rather than volatile names; and
5. the normalized generated trigger records for enumerated FKs.

PostgreSQL 16.15 rejects direct array-type ACL changes. Observation therefore
requires raw array `typacl` null and resolves effective atoms through the exact
element type; applying `acldefault` directly to an array is a comparison error.

Every other owner-created table, view, materialized view, sequence, partition,
foreign table, function, procedure, aggregate, operator, cast, collation,
conversion, text-search object, rule, user trigger, publication, subscription,
extension, or schema object is forbidden.

### 6. Seal the exact-result query topology

The catalogue contains one query named `registration-exact-result-v1`.
Its root is `registration_results AS result`. It has three ordered `LEFT JOIN`
edges:

1. `result.current_event_digest` to
   `semantic_events AS current_event.event_digest`;
2. `result.run_id` to `registration_runs AS run.run_id`; and
3. `run.original_registration_event_digest` to
   `semantic_events AS original_event.event_digest`.

Every edge also equates both `S` columns. The root predicates equal the pinned
project-authority digest parameter, literal project-scope role, and semantic
request digest parameter. Parameters are base `bytea` values, never private
domain OIDs. The maximum returned rows is `2`: zero means absent, one is decoded,
and two fails indeterminate.

The 27 projections are ordered and literal. Every domain is cast to its base
type; uint64 and response status are cast to text. The output aliases must equal
the private `POSTGRES_EXACT_RESULT_RAW_ROW_KEYS_V1` tuple byte-for-byte. The
tuple is exported only from its private module, never from `src/index.ts`.

Using each nested record's field order from section 3, the complete normalized
query fragment is:

```text
root = (sf_supervisor_v1, registration_results, result)
parameters = [(1, project_authority_digest, bytea), (2, semantic_request_digest, bytea)]
joins = [(left, result, sf_supervisor_v1, semantic_events, current_event, [(project_authority_digest, project_authority_digest), (project_scope_role, project_scope_role), (current_event_digest, event_digest)]), (left, result, sf_supervisor_v1, registration_runs, run, [(project_authority_digest, project_authority_digest), (project_scope_role, project_scope_role), (run_id, run_id)]), (left, run, sf_supervisor_v1, semantic_events, original_event, [(project_authority_digest, project_authority_digest), (project_scope_role, project_scope_role), (original_registration_event_digest, event_digest)])]
predicates = [(result, project_authority_digest, equals, parameter, 1), (result, project_scope_role, equals, literal, sf_supervisor_project_scope_v1), (result, semantic_request_digest, equals, parameter, 2)]
projection = [(result, project_authority_digest, bytea, result_project_authority_digest), (result, semantic_request_digest, bytea, result_semantic_request_digest), (result, run_id, text, result_run_id), (result, serialized_request, bytea, result_serialized_request), (result, serialized_request_sha256, bytea, result_serialized_request_sha256), (result, response_status, text, result_response_status_text), (result, response_content_type, text, result_response_content_type), (result, serialized_response, bytea, result_serialized_response), (result, serialized_response_sha256, bytea, result_serialized_response_sha256), (result, current_event_digest, bytea, result_current_event_digest), (current_event, project_authority_digest, bytea, current_event_project_authority_digest), (current_event, event_digest, bytea, current_event_digest), (current_event, event_kind, text, current_event_kind), (current_event, semantic_request_digest, bytea, current_event_semantic_request_digest), (current_event, prior_controller_state_head_digest, bytea, current_event_prior_controller_state_head_digest), (current_event, serialized_envelope, bytea, current_event_serialized_envelope), (current_event, serialized_envelope_sha256, bytea, current_event_serialized_envelope_sha256), (run, project_authority_digest, bytea, run_project_authority_digest), (run, run_id, text, run_run_id), (run, original_registration_request_digest, bytea, run_original_registration_request_digest), (run, original_registration_event_digest, bytea, run_original_registration_event_digest), (original_event, project_authority_digest, bytea, original_event_project_authority_digest), (original_event, event_digest, bytea, original_event_digest), (original_event, semantic_request_digest, bytea, original_event_semantic_request_digest), (original_event, global_sequence, text, original_event_global_sequence_text), (original_event, serialized_envelope, bytea, original_event_serialized_envelope), (original_event, serialized_envelope_sha256, bytea, original_event_serialized_envelope_sha256)]
maximumRows = 2
```

The topology, sources, predicates, join kinds/order, scope pairs, casts,
projection order, aliases, and row ceiling are all contract data. A later
deterministic generator emits the schema-qualified SQL and a KAT pins its raw
digest. Matching aliases with a weakened join is insufficient.

### 7. Bound parsing and observation before allocation

The byte parser accepts only a non-proxy intrinsic `Uint8Array`, snapshots it
synchronously, checks the hard byte ceiling before decoding, performs fatal
UTF-8 decoding and byte-identical re-encoding, scans JSON within the fixed
depth/node/key/string/collection ceilings, parses, reconstructs the closed
schema, validates cross-references and ordering, re-encodes, hashes, and deep
freezes independent storage.

All catalogue/provisioning observation queries use a server-side
`expected cardinality + 1` limit before result allocation. Total observation
rows are bounded by the contract maxima. Each driver value is synchronously
snapshotted before another await.

Role traversal is provisioning-only, cycle-safe, and bounded to the nine named
roles plus one excess sentinel. It records direct incident edges first and
computes the forbidden transitive closure locally over that bounded graph.
Unexpected cycles, nodes, edges, or truncation fail.

No parser error includes raw contract bytes, credentials, SQL parameters, or
connection strings.

## Acceptance gates

This decision is implemented only when:

1. independent KATs pin exact byte length, raw SHA-256, root order, final LF,
   round-trip bytes, and deep immutability on Node 20.0.0 and 24.14.1;
2. invalid UTF-8, BOM/CRLF, duplicate/reordered/unknown keys, unsafe numbers,
   over-limit graphs, sparse arrays, proxies, accessors, symbols, exotic
   prototypes, and count-neutral substitutions fail without invoking traps;
3. the independently reviewed JSON proves ten domains, eight relations, 88
   columns, 19 principal/command admissions, 38 policies, and every literal
   key/FK/index/trigger/expression/ACL fact without code-supplied defaults;
4. coherent OID renumbering with unchanged resolved identities, generated-`tgname`-only churn, and raw ACL reorder mutants normalize identically, while unresolved OID,
   PUBLIC/role, null/empty/default ACL, grantor/grantee/grantable, ordered tuple,
   expression, RLS, policy, and immediate/deferred trigger linkage/parent/type/internal/function/deferral/bitmask/filter/argument/qualifier/transition-table/`tgenabled` (`O` to `D`, `R`, or `A`) mutants fail;
5. the query topology and 27 aliases match an independent literal and the
   private row decoder, while weakened joins, scope predicates, casts, order,
   or cardinality fail;
6. the JSON and parser are sealed private build inputs and protected paths,
   while the public exports, public bundle bytes, empty runtime dependencies,
   and all false authority flags remain unchanged; and
7. the manifest alone selects PostgreSQL 16.15 and binds both digests; later
   evidence compares catalogue and provisioning separately and proves no
   observed-state write path exists.

## Consequences

### Positive

- The catalogue digest now has one replayable byte meaning.
- Deployment-specific authority cannot drift into schema truth or vice versa.
- OID churn and raw ACL order do not create false drift, while semantic
  substitutions still fail.
- Exact query recovery cannot be weakened behind a matching alias list.
- Hostile contracts and catalogue observations are bounded before allocation.

### Negative

- The contract is deliberately verbose and tied to PostgreSQL 16.15 deparsing.
- A schema change requires coordinated catalogue, SQL, manifest, KAT, and live
  evidence updates.
- The compact root-line representation optimizes deterministic review and the
  repository's 500-line rule rather than ordinary pretty-JSON ergonomics.

### Neutral

- This completes ADR-0044's representation; it does not change the product
  architecture, application goals, public API, or activation state.
- Provisioning remains the next separate contract slice.

## Alternatives rejected

- **Generic pretty JSON** — the 88-column inventory exceeds the file-line
  policy and leaves key-order reconstruction underspecified.
- **RFC 8785 by label only** — claiming a standard without implementing every
  number and string rule would create false interoperability.
- **One combined catalogue/provisioning oracle** — makes database deployment
  identity and schema evolution inseparable.
- **Store raw OIDs or ACL arrays** — treats volatile order and installation
  identity as semantics.
- **Aliases without query topology** — permits weakened joins to look valid.
- **Generate or refresh from live PostgreSQL** — lets observed state rewrite
  authority.
- **Hash enforcement inside PostgreSQL** — requires a forbidden extension or
  owner function and duplicates the materializer's byte authority.

## Links

- [ADR-0038](ADR-0038-sota-application-completion-programme.md)
- [ADR-0039](ADR-0039-minimal-production-serving-artifact.md)
- [ADR-0042](ADR-0042-witnessed-single-use-capture-supervisor-protocol.md)
- [ADR-0043](ADR-0043-postgresql-supervisor-registration-state-and-dormant-adapter.md)
- [ADR-0044](ADR-0044-postgresql-supervisor-catalogue-contract.md)
- [PostgreSQL 16 system catalogues](https://www.postgresql.org/docs/16/catalogs.html)
- [`pg_constraint`](https://www.postgresql.org/docs/16/catalog-pg-constraint.html)
- [`pg_policy`](https://www.postgresql.org/docs/16/catalog-pg-policy.html)
- [`pg_auth_members`](https://www.postgresql.org/docs/16/catalog-pg-auth-members.html)
- [PostgreSQL system information functions](https://www.postgresql.org/docs/16/functions-info.html)
