# ADR-0050 Appendix A: Observed Schema Identity V1 contract

**Normative authority:** This document is Appendix A of
[ADR-0050](../adr/ADR-0050-verified-source-generation-leases-schema-identity-and-atomic-runtime-activation.md).
It has no independent status; it is proposed, accepted, superseded or rejected
with ADR-0050. [Appendix B](./ADR-0050-observed-schema-identity-v1-known-answer-vectors.md)
contains the inseparable known-answer vectors.

This appendix is normative. “Must,” “reject” and “exact” are conformance
requirements.

## A.1 Scope and authority

Observed Schema Identity V1 is a pure, deterministic `sf-core` content-identity
kernel. It performs no I/O and has no database-driver dependency. Its result is:

```rust
pub struct ObservedSchemaIdentityV1 {
    structural: StructuralSchemaDigestV1,
    types: TypeSchemaDigestV1,
    constraints: ConstraintSchemaDigestV1,
}
```

Each digest is an opaque newtype over `[u8; 32]`. Construction is private to the
canonical encoder; accessors may expose bytes and lowercase hexadecimal text.

The identity:

- grants no type, constraint, mapping, execution, lease, readiness or backend
  authority;
- never constructs or upgrades `ConstraintAuthority`, `ColumnTypeAuthority` or
  `VerifiedGenerationLease`;
- is insufficient by itself for plan-cache reuse or Direct-Mapping admission;
  and
- remains `Unverified` regardless of who supplied the input.

There is no `From<TableSchema>` or equivalent conversion. `TableSchema` lacks
qualified identity, explicit ordinals, lossless type semantics and constraint
state. `sf-sql` adapters construct the new input from richer bounded catalogue
rows.

## A.2 Public value model

Public input values are validated, but the admitted observation has private
fields and no mutable access.

```rust
pub struct SchemaObservationInputV1 {
    pub profiles: SchemaProfilesV1,
    pub relations: Vec<RelationInputV1>,
    pub constraints: Vec<ConstraintInputV1>,
}
pub struct SchemaProfilesV1 {
    pub structural: ProfileIdV1,
    pub types: ProfileIdV1,
    pub constraints: ProfileIdV1,
}
pub struct RelationInputV1 {
    pub name: QualifiedNameV1,
    pub kind: RelationKindV1,
    pub columns: Vec<ColumnInputV1>,
}
pub struct ColumnInputV1 {
    pub ordinal: NonZeroU32,
    pub name: IdentifierV1,
    pub source_type: SourceTypeV1,
}
pub struct SourceTypeV1 {
    pub native_name: QualifiedNameV1,
    pub family: TypeFamilyV1,
    pub facets: Vec<TypeFacetV1>,
}
pub struct TypeFacetV1 {
    pub key: TokenV1,
    pub value: TypeFacetValueV1,
}
pub enum TypeFacetValueV1 {
    Bool(bool), U64(u64), I64(i64), Text(TextValueV1),
    TypeName(QualifiedNameV1), TextList(Vec<TextValueV1>),
    TypeNameList(Vec<QualifiedNameV1>), Digest32([u8; 32]),
}
```

Constraint inputs are:

```rust
pub enum ConstraintInputV1 {
    NotNull { column: ColumnRefV1, state: ConstraintStateV1 },
    PrimaryKey {
        relation: RelationRefV1,
        state: ConstraintStateV1,
        columns: Vec<ColumnKeyV1>,
    },
    UniqueKey {
        relation: RelationRefV1,
        state: ConstraintStateV1,
        nulls: UniqueNullSemanticsV1,
        columns: Vec<ColumnKeyV1>,
    },
    ForeignKey {
        child: RelationRefV1,
        parent: RelationRefV1,
        state: ConstraintStateV1,
        match_kind: ForeignKeyMatchV1,
        pairs: Vec<(ColumnKeyV1, ColumnKeyV1)>,
    },
}
pub struct ConstraintStateV1 { pub validated: bool, pub enforced: bool }
pub struct RelationRefV1 { pub name: QualifiedNameV1 }
pub struct ColumnRefV1 { pub relation: RelationRefV1, pub column: ColumnKeyV1 }
pub struct ColumnKeyV1 { pub ordinal: NonZeroU32, pub name: IdentifierV1 }
```

`QualifiedNameV1` contains `catalog: Option<IdentifierV1>`,
`schema: Option<IdentifierV1>` and mandatory `local: IdentifierV1`. A relation
reference resolves by qualified name to exactly one admitted relation; its kind
is taken from that relation. V1 admits only `RelationKindV1::BaseTable`. Other
relation kinds require a later grammar version.

Resolution is exact, never search-path or partial-name matching. All three
`QualifiedNameV1` components, including `None` versus `Some`, compare by their
exact bytes. A `ColumnKeyV1` resolves only when both its ordinal and name match
the same admitted column in that exact relation; matching either field alone
rejects.

## A.3 Hard limits

The input and resulting canonical bodies must satisfy every limit:

| Limit | Exact maximum |
|---|---:|
| Profile/token bytes | 64 |
| Identifier bytes | 1,024 |
| Facet text-value bytes | 16,384 |
| Relations | 4,096 |
| Columns per relation | 4,096 |
| Columns in the observation | 65,536 |
| Raw constraint records | 65,536 |
| Canonical constraint records | 65,536 |
| Members in one PK, UNIQUE or FK | 256 |
| Facets per column type | 64 |
| Facets in the observation | 262,144 |
| Items in one facet list | 1,024 |
| Facet-list items in the observation | 1,048,576 |
| Cumulative UTF-8 payload-occurrence bytes | 33,554,432 |
| Canonical body bytes per digest | 67,108,864 |

An empty observation is valid. An admitted relation has at least one column.
The UTF-8 cap is the checked sum of the byte length of every semantic string
occurrence in the submitted value, including repeated names in references and
every list item; allocation sharing or interning cannot change it. The builder
precomputes each exact canonical body length with the same grammar and checked
arithmetic before allocating a body or initializing a digest. It then streams
exactly that many body octets. Excess, mismatch, overflow or an unrepresentable
`u32` length rejects without returning a partial identity. Any future adapter
must additionally apply equivalent cap-plus-one collection bounds before it
constructs this input; that adapter work is outside the Phase 1 kernel.

The public builder returns `Result<ObservedSchemaIdentityV1,
SchemaIdentityErrorV1>`. Its closed error variants are `Invalid { code,
location }`, `LimitExceeded { limit, observed, maximum }`, and
`ArithmeticOverflow { operation }`. Closed invalid codes distinguish token,
identifier/text, relation kind, empty relation/key, ordinal continuity,
duplicate relation/column/facet/key member, duplicate FK child/parent,
dangling relation/column, and multiple-primary-key failures. Locations contain
only bounded `u32` relation, column, facet, constraint or member indexes.
Detection precedence for multiply invalid input is not a V1 guarantee; callers
match the typed variant and code. `Display` and `Debug` are at most 256 UTF-8
bytes, expose only code/index/count data, and never identifiers, type text, SQL,
paths or source configuration; `source()` returns `None`.

`RelationKind` is a reserved error code for a future untyped adapter boundary.
Direct Phase 1 input uses the closed `RelationKindV1` enum and therefore cannot
construct an unsupported relation kind; this reservation grants no new V1 kind.

Changing a cap, primitive encoding, tag meaning or record grammar requires V2.
Adding a separately specified profile under this grammar does not.

## A.4 Identifier, token and name law

`IdentifierV1` and `TextValueV1` are valid UTF-8 and contain no U+0000.
Identifiers are non-empty; text values may be empty. Identifier bytes are the
exact bytes supplied by the catalogue profile: there is no Unicode
normalization, case folding, whitespace trimming or SQL reinterpretation.
Equality and ordering are unsigned bytewise. Composed and decomposed Unicode
spellings are therefore distinct. Exact duplicate qualified relation names in
one observation reject; exact duplicate column names within one relation
reject. The same column name in different relations is valid. Case-distinct
names remain representable. Mapping identifier resolution remains a separate
ADR-0015 concern.

`ProfileIdV1` and `TokenV1` are 1–64 ASCII bytes. The first and last byte are
`[a-z0-9]`; interior bytes are `[a-z0-9._-]`. The Phase 1 kernel treats a
profile ID only as an opaque domain-separation token and performs no
profile-specific normalization. V1 registers no production profile. All six
profile IDs in Appendix B are reserved for those tests and must not be emitted
by product integration. Before a later adapter may emit an identity, a closed
registry must bind its globally unique immutable profile ID to one backend
family, catalogue scope, normalization law, required/forbidden facets and
version; reuse of an ID for another definition rejects.

Relation, column and native type names are semantic and enter their respective
digests. Constraint names do not. Renaming a constraint without changing its
admitted facts leaves the constraint digest unchanged. Database-local transient
identifiers such as PostgreSQL OIDs, connection IDs, transaction IDs and
physical row IDs do not enter these durable digests; they may exist only in a
backend lease.

## A.5 Primitive byte grammar

All integers are big-endian. No native-width integer or `usize` is encoded.

```text
U8(x)       = one octet x
BOOL(false) = 00
BOOL(true)  = 01
U32(x)      = four-octet unsigned big-endian x
U64(x)      = eight-octet unsigned big-endian x
I64(x)      = eight-octet two's-complement big-endian x
TXT(s)      = U32(byte_length(UTF8(s))) || UTF8(s)
OPT(None)   = 00
OPT(Some x) = 01 || x
VEC(xs)     = U32(count(xs)) || encoding(xs[0]) || ... || encoding(xs[n-1])
```

No padding, terminator, BOM, alignment, host-endian value, JSON, CBOR, Serde
representation, debug string or driver display representation is present.

```text
QNAME(q) = OPT(TXT(q.catalog)) || OPT(TXT(q.schema)) || TXT(q.local)
RELATION_COORD(r) = QNAME(r.name) || RELATION_KIND(r.kind)
COLUMN_KEY(c) = U32(c.ordinal) || TXT(c.name)
COLUMN_COORD(r,c) = RELATION_COORD(r) || COLUMN_KEY(c)
```

Column ordinals are contiguous `1..=column_count`. Gaps, zero, repeated ordinals
or repeated exact names reject.

## A.6 Domains and tags

The SHA-256 preimage is `DOMAIN || U64(body_byte_length) || BODY`. Domains
include their final NUL octet:

```text
Structural: b"semantic-fabric:observed-schema:structural:v1\0"  // 46 octets
Type:       b"semantic-fabric:observed-schema:type:v1\0"        // 40 octets
Constraint: b"semantic-fabric:observed-schema:constraint:v1\0"  // 46 octets
```

| Tag | Meaning | Tag | Meaning |
|---:|---|---:|---|
| `11` | Structural relation | `12` | Structural column |
| `21` | Column type | `22` | Type facet |
| `31` | NOT NULL | `32` | Primary key |
| `33` | Unique key | `34` | Foreign key |

`RelationKindV1` has one tag: `01` Base table.

| Type-family tag | Meaning | Type-family tag | Meaning |
|---:|---|---:|---|
| `01` | Boolean | `02` | Signed integer |
| `03` | Unsigned integer | `04` | Exact numeric |
| `05` | Approximate numeric | `06` | Character |
| `07` | Binary | `08` | Date |
| `09` | Time | `0a` | Timestamp |
| `0b` | Interval | `0c` | JSON |
| `0d` | UUID | `0e` | Enum |
| `0f` | Array | `10` | Domain |
| `11` | Dynamic per-value type | `7f` | Opaque observational type |

| Facet-value tag | Payload | Facet-value tag | Payload |
|---:|---|---:|---|
| `01` | `BOOL` | `02` | `U64` |
| `03` | `I64` | `04` | `TXT` |
| `05` | `QNAME` | `06` | `VEC(TXT)` |
| `07` | `VEC(QNAME)` | `08` | exactly 32 digest octets |

Unique-null semantics are `01` NULLs distinct, `02` NULLs not distinct and
`03` observed but unknown. FK-match semantics are `01` SIMPLE, `02` FULL, `03`
PARTIAL and `04` observed but unknown. Every literal tag in this appendix is one
octet and the tables give its hexadecimal value. Formally,
`RELATION_KIND(x)`, `TYPE_FAMILY(x)`, `UNIQUE_NULL_SEMANTICS(x)` and
`FOREIGN_KEY_MATCH(x)` are `U8(the listed tag)`, while each
`FACET_VALUE(x)` is `U8(the listed tag) || the listed payload`.

V1 exposes typed input and an encoder, not a byte decoder. Closed Rust enums
make unknown/reserved tags unconstructible; V1 therefore makes no
unknown/trailing-input-byte rejection claim. A future decoder requires its own
canonicality contract. `Opaque`, unknown-null and unknown-match values remain
observational and cannot authorize compilation.

## A.7 Structural body

```text
STRUCTURAL_BODY = TXT(structural_profile) || U32(relation_count)
               || STRUCTURAL_RELATION*
STRUCTURAL_RELATION = 11 || RELATION_COORD(relation) || U32(column_count)
                    || STRUCTURAL_COLUMN*
STRUCTURAL_COLUMN = 12 || COLUMN_KEY(column)
```

Relations sort by unsigned lexicographic comparison of complete encoded
`RELATION_COORD`; input order is irrelevant. Columns encode in ordinal order.
Only qualified relation identity, relation kind, column identity and ordinal
enter this digest.

## A.8 Type body

```text
TYPE_BODY = TXT(type_profile) || U32(total_column_count) || TYPE_RECORD*
TYPE_RECORD = 21 || COLUMN_COORD(relation,column)
            || QNAME(column.source_type.native_name)
            || TYPE_FAMILY(column.source_type.family)
            || U32(facet_count) || TYPE_FACET*
TYPE_FACET = 22 || TXT(facet.key) || FACET_VALUE(facet.value)
```

There is exactly one type record for every structural column and no record for
another coordinate. Type records sort by complete encoded `COLUMN_COORD`.
Facets intentionally sort by unsigned bytewise raw `facet.key`, not by the
length-prefixed `TXT(facet.key)` encoding; duplicate facet keys reject.
Facet-list order is semantic and preserved. Tests include keys such as `b` and
`aa`, whose raw-byte and length-prefixed orders differ. A registered profile
needing set semantics defines its own unique canonical list order.

A later registered type profile defines and validates required and forbidden
facets for its families, including length, precision, scale, timezone, declared
type, affinity, STRICT table, collation provider/version/determinism, domain
base, enum-label order, array element and definition-digest facts. The Phase 1
kernel validates only this appendix's generic syntax and bounds. Changing a
registered profile law requires a new profile ID.

## A.9 Constraint body

```text
CONSTRAINT_BODY = TXT(constraint_profile)
                || U32(canonical_constraint_count) || CONSTRAINT_RECORD*
NOT_NULL = 31 || COLUMN_COORD(column)
         || BOOL(validated) || BOOL(enforced)
PRIMARY_KEY = 32 || RELATION_COORD(relation)
            || BOOL(validated) || BOOL(enforced)
            || U32(column_count) || COLUMN_KEY*             // declared order
UNIQUE_KEY = 33 || RELATION_COORD(relation)
           || BOOL(validated) || BOOL(enforced) || UNIQUE_NULL_SEMANTICS
           || U32(column_count) || COLUMN_KEY*              // canonical order
FOREIGN_KEY = 34 || RELATION_COORD(child_relation)
            || RELATION_COORD(parent_relation)
            || BOOL(validated) || BOOL(enforced) || FOREIGN_KEY_MATCH
            || U32(pair_count)
            || (COLUMN_KEY(child) || COLUMN_KEY(parent))*   // declared order
```

PK, UNIQUE and FK lists are non-empty and bounded by the key-arity cap. Every
reference follows A.2's exact qualified-name and `(ordinal, name)` match. A PK
or UNIQUE list rejects a repeated exact column key before normalization. An FK
rejects a repeated exact pair, repeated child key, or repeated parent key.
After exact duplicate constraint records collapse, more than one distinct PK
for a relation rejects.

UNIQUE member order is relationally irrelevant and normalizes by encoded
`COLUMN_KEY` order. PK order is retained because it changes Direct-Mapping row
IRIs. FK pair order is retained because it changes the Direct-Mapping reference
predicate. Validation resolves references and rejects repeated members first;
UNIQUE members then normalize, complete records encode, exact encoded records
deduplicate, remaining records sort by unsigned lexicographic comparison, and
the post-deduplication count encodes.

V1 admits only NOT NULL, PK, UNIQUE and FK facts. Constraint names,
backing-index names, defaults, checks, exclusion constraints, update/delete
actions, deferrability, functional-dependency annotations, statistics,
estimates and collection times are absent and cannot grant authority.

## A.10 Digest independence

The three profiles occur only in their corresponding bodies. Consequently:

- a type descriptor or type-profile change affects only the type digest;
- a constraint fact or constraint-profile change affects only the constraint
  digest;
- statistics and estimates affect none;
- structural changes affect structural identity and any type or constraint
  record carrying that coordinate;
- a constraint-name-only change affects none;
- relation, constraint, facet-record and UNIQUE-member input order affects none;
  and
- PK-member, FK-pair and facet-list order is semantic and can change a digest.

The runtime binds the three-digest tuple with its separate `SourceId`, mapping,
ontology, capability, policy and activation identities. No individual digest is
a complete runtime generation.

## A.11 Rust organization

Phase 1 remains within `sf-core`, existing crate boundaries and the repository
file-size rule:

```text
sf-core/src/schema_identity/mod.rs
sf-core/src/schema_identity/model.rs
sf-core/src/schema_identity/validate.rs
sf-core/src/schema_identity/encode.rs
sf-core/src/schema_identity/tests/mod.rs
sf-core/src/schema_identity/tests/vectors.rs
sf-core/src/schema_identity/tests/mutations.rs
sf-core/src/schema_identity/tests/bounds.rs
```

`model.rs` owns public values and caps, `validate.rs` owns cross-reference
normalization, and `encode.rs` owns the only canonical encoder.
`sha2.workspace = true` is the only new `sf-core` dependency. No Node code
participates. Phase 1 adds no adapter collection, source-type fidelity work,
runtime binding propagation, cache identity, lease, reload or mapping code.
