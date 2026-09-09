---
status: accepted
date: 2026-06-27
updated: 2026-09-09
tags: [datatype, dialect, r2rml-section-10, canonicalization, oxsdatatypes, sqlite-affinity, correctness]
supersedes: []
depends-on:
  - ADR-0003
  - ADR-0004
  - ADR-0006
implements:
  - ADR-0001
---

# Datatype & dialect correctness — R2RML §10 canonicalization

## Context and Problem Statement

R2RML §10 defines the natural mapping from a SQL value to an RDF literal and mandates **consistency**: the same (target datatype, value) MUST yield the same lexical form — across modes and across source dialects. That consistency clause *is* the engine's parity contract. The hazard: a driver's default string rendering of a non-string value is frequently non-canonical or wrong for RDF (PostgreSQL booleans `t`/`f`, `bytea` `\x`+lowercase, hour-only tz offsets, space-separator timestamps; `xsd:double` requires `E`-notation everywhere; decimals return scale-padded), and **SQLite has no reliable per-column type at all**.

## Considered Options

* **Trust the driver's default string rendering of non-string values** — rejected: frequently non-canonical or wrong for RDF (PostgreSQL booleans `t`/`f`, `bytea` `\x`+lowercase, hour-only tz offsets, space-separator timestamps; `xsd:double` needs `E`-notation everywhere; decimals return scale-padded), and SQLite has no reliable per-column type at all.
* **Push canonicalization into SQL** — rejected: scientific notation, decimal trimming, and hex casing are fragile across dialects, and the cross-source read goes through other renderers anyway. SQL does set-work; Rust does lexical form.
* **Catalog-driven type determination + one Rust canonicalization chokepoint (chosen)** — determine the target XSD datatype from catalog metadata (per-dialect `DbTypeMap`), then produce the XSD canonical lexical form in Rust via `oxsdatatypes` and the decimal/scientific/hex refinements below, with a per-value SQLite storage-class branch for dynamic typing.
* **Strict SQL:2008 delimited-vs-regular identifier rejection** — rejected for identifier resolution: provenance is unrecoverable by a virtualiser and strict rejection is a net conformance loss against the predominantly-lenient W3C suite. Chosen instead: resolve every mapping column identifier against the live introspected schema (exact match, then unique ASCII-case-insensitive match).

## Decision Outcome

**Never trust the driver's rendering for a non-string value. Determine the target XSD datatype from catalog metadata, then produce the XSD canonical lexical form in Rust.** Two layers:

1. **Type determination — a per-dialect `DbTypeMap`** (the Ontop `DBTypeFactory` analogue): native source type → internal `XsdTypeCode`, read from the catalog (`information_schema`/`pg_catalog` for PostgreSQL; `PRAGMA table_info` for SQLite; MySQL catalog plus wire metadata). MySQL deliberately cannot recover whether an authored `BOOL` or `TINYINT(1)` produced the same server type identity.
2. **Value canonicalization — one Rust chokepoint** in `sf-core` term generation. Fetch each value in the most type-faithful driver form (binary/typed over text), then use `oxsdatatypes` parsing and canonical `Display` where its representation covers the source values. Full-range decimal construction validates and normalizes lexical slices; scientific formatting and uppercase hex have the dedicated refinements below. Canonicalization is keyed on the value's **target XSD type**, never on the dialect's text. **Do not push RDF construction into SQL**: SQL does separately qualified set-work; Rust emits the final lexical form.

> **Reconciliation note (2026-06-28, impl-verified).** "emit via its `Display`" is exact for every XSD type the engine canonicalizes **except `xsd:double` / `xsd:float`**: in `oxsdatatypes` 0.2.2 their `Display` delegates to Rust `f64`/`f32` formatting, which is **not** XSD-canonical (e.g. `1.0` → `1`, no mandatory `E`-notation — contradicting the "`E`-notation everywhere" requirement above). For those two types the single `sf-core` chokepoint still **parses/validates through `oxsdatatypes`** but emits the XSD-canonical scientific form itself (mantissa with ≥ 1 fractional digit, uppercase `E`, no leading-zero exponent; `INF`/`-INF`/`NaN`); all other types use `oxsdatatypes` `Display` directly as stated. This is a documentation correction only — the implemented output is XSD-canonical per §10 (verified by the `sf-core` canonical-double tests). Companion note in ADR-0006 §Term generation.

**SQLite — the special hazard (dynamic typing / type affinity).** A column's declared type is only a recommendation; values carry their own storage class. Policy: **branch on the per-value storage class** (`sqlite3_column_type()`), with a fast-path when the table is declared `STRICT` (3.37+). A documented, tested contract.

**MySQL — explicit profile boundary.** `MysqlTypeProfile::Native` is the product law: ambiguous `BOOL`/`TINYINT(1)` remains `xsd:integer`, including value `2`. Only the sealed W3C runner selects `MysqlTypeProfile::W3cSql2008`, whose versioned identity `mysql-w3c-sql-2008-v1` applies the suite's SQL-2008 logical-boolean convention. An explicit `rr:datatype` is authoritative in either profile. MySQL-only aliases remain outside dialect-neutral `natural_xsd`, so neither SQLite nor PostgreSQL admission changes. The selected profile participates in receipt outcome identity; this convention is not native-product type provenance.

**`sqlparser` is SQL syntax only** — used for SQL emission and parsing `rr:sqlQuery`; it contributes nothing to type semantics, which is this separate subsystem. **NULL** in any referenced column ⇒ no RDF term (R2RML §11). Rust reconstructs terms; SQL atom-local `IS NOT NULL` conditions prevent absent subjects, predicates, objects and referenced parent subjects from becoming query solutions before projection, ASK, aggregation or correlation. Class shortcuts guard their subject. Selected graph guards preserve OR semantics across graph alternatives; they never require every graph map to exist.

> **Path comparison refinement (2026-09-08).** Live text-decoder facts authorize
> exact comparison without replacing Rust RDF reconstruction. SQLite CHARACTER(n)
> uses its shared Rust lexical/padding function before joins and deduplication;
> PostgreSQL BPCHAR preserves wire padding through convert_from(bpcharsend(...),
> 'UTF8'), never a trimming ::text cast. MySQL nonbinary STRING follows its native
> session-sensitive text decoder and existing UTF8/NO PAD comparison, not RPAD.
> Numeric/date/binary/unknown families are not blanket-cast. The same-IR SQLite
> prepare-only twin omits comparison collation but retains decoder calls, so a
> compound relation cannot re-pad a CHAR(2) endpoint as CHAR(4). SQL/twin positions
> and parameters match; normalized outputs are text, raw SubPlans retain only
> agreed decoder facts. Required duplicate/connectivity, mixed-width, Unicode/NUL,
> HTTP correlation and pinned native tests pass. General mixed-type identity,
> ordinary-query collation and exact release remain open (ADR-0049).

> **NULL-query correction (2026-09-08).** Required `sf-sparql --test null_terms`
> checks cover subject column/template/blank-node absence, predicate absence,
> referenced parent subjects, inverse direction, class shortcuts, SELECT/ASK/COUNT,
> OPTIONAL/existence/anti-joins and graph alternatives across tree/flat/unoptimized
> translation. Authenticated SQLite HTTP and owned PostgreSQL16.15/MySQL8.4.11 TLS
> CLI checks exercise the admitted subject/object/class profiles. Dynamic predicates
> remain rejected by serving admission; raw compiler predicate tests do not widen
> that profile. General key equality, source bounds and release remain separate.

### PostgreSQL NUMERIC decoder correction (2026-09-09)

The binary digit count is unsigned 16-bit; its weight remains signed. The reader
now accepts 32,768 digit groups instead of rejecting a valid large value as a
negative count. It preserves arbitrary-precision lexical digits and display scale,
retains truncated-array checks, and still rejects NaN/infinities as unsupported.
The required owned PostgreSQL TLS query-profile aggregate includes authenticated
IRI reconstruction of a 131,072-digit NUMERIC with `.00` display scale. A synthetic
wire regression also covers the unsigned boundary and truncation. This repairs
the raw decoder; the range refinement below covers natural literals. The identity refinement closes
the separate qualified static-template comparison and deduplication slice.

### PostgreSQL NUMERIC identity refinement (2026-09-09)

Live NUMERIC metadata now authorizes a separate finite decoded lexical key, not
text output replacement or general numeric comparison. SQL set-work validates
`NUMERIC -> TEXT -> JSON -> TEXT` and compares under `C` collation; PostgreSQL's
[pinned JSON input/output](https://raw.githubusercontent.com/postgres/postgres/REL_16_15/src/backend/utils/adt/json.c)
preserves validated text. `JSONB` and `to_json` are not equivalent substitutes.
The returned payload remains NUMERIC, and Rust remains the RDF constructor.
Scale-distinct IRI substitutions (`1.0` / `1.00`) survive D1 and output DISTINCT;
known natural-only consumers keep their separate canonical numeric identity.

Required owned TLS CLI checks cover exact listed/fixed/`=`/`sameTerm` values,
signed/zero/fractional scale, duplicate and mixed-consumer bags, ordering/slicing,
native reference joins and authored SQL result expressions. Authored SQL literals
still require explicit datatype for semantic admission. Hidden NaN/infinity
terms fail terminally, including COUNT/ASK and unprojected OPTIONAL; portable
policy excludes denied invalid rows and cap-one requests recover. SQL validation
errors need not have the raw decoder's Unsupported classification. Natural-term
identity, other native families and general pooled identity remain unclosed.

### Natural decimal range refinement (2026-09-09)

The Rust chokepoint now validates the complete ASCII decimal lexical grammar and
normalizes sign/zero/digit slices without fixed-width arithmetic or intermediate
allocation. This supersedes direct `oxsdatatypes::Decimal` parsing for construction,
not arithmetic. Integral decimals retain `1`, not `1.0`, as required by the chosen
[XSD 1.1 canonical spelling](https://www.w3.org/TR/xmlschema11-2/#decimal); the
[R2RML consistency law](https://www.w3.org/TR/r2rml/#natural-mapping) is unchanged.
Core tests preserve prior representable spellings, reject malformed text, and
cover magnitude/scale extrema and idempotence. Owned PostgreSQL TLS CLI tests
reconstruct 131,072 integral digits and 16,383 fractional digits through SELECT,
DISTINCT and hidden COUNT/ASK; mixed raw-IRI/natural columns retain scale identity,
and an output-byte failure releases cap-one admission. Owned MySQL TLS SELECT and
DISTINCT also preserve the full `DECIMAL(65,30)` value. Raw driver data and IRI
construction are unchanged; normalization is linear, with output at most input
bytes plus one. Ordered/floating numeric comparison and AVG remain separate work;
the following refinement covers natural decimal identity and Eq/Ne. Existing explicit-datatype lexical behavior is unchanged:
the [same-natural-datatype case in R2RML §11.2](https://www.w3.org/TR/r2rml/#generated-rdf-term)
requires separate correction, not a claim that every explicit datatype overrides
natural construction.

### Natural decimal comparison refinement (2026-09-09)

Live PostgreSQL NUMERIC and MySQL NEWDECIMAL facts now authorize canonical
comparison keys, not replacement output columns. PostgreSQL uses
`pg_catalog.trim_scale` followed by the existing JSON finite-number validation;
MySQL normalizes its full decimal text, including ZEROFILL, sign and scale.
Rust remains the final RDF constructor. Fixed literals and `sameTerm` compare
lexical/datatype/language tuples exactly; query constants remain verbatim.
Raw NULL operands preserve expression errors under negation.

A separate finite natural-decimal Eq/Ne lane promotes base integer/decimal
constants using the full-range Rust normalizer. Integer grammar is validated
independently; malformed typed lexicals yield expression errors, not matches
under NOT. No bounded numeric cast or float conversion is introduced. This
repairs the public PostgreSQL numeric-versus-text binding failure without
canonicalizing `sameTerm` operands or granting ordered/float comparison authority.

Natural provenance follows raw Projection/Ref/SubPlan positions. Mixed or
coercing arms retain an incompatible marker: MySQL multi-arm DECIMAL is not
range-safe merely because every arm is decimal. PostgreSQL policy predicates
dominate fallible literal keys, including direct EXISTS bodies. Owned TLS CLI
checks cover both providers' fixed/sameTerm/value matches, noncanonical and
wrong-datatype constants, NULL/OPTIONAL, invalid-lexical negation, ZEROFILL, and
PostgreSQL denied non-finite values with cap-one recovery. General natural BGP
unification, same-natural explicit datatype construction, arithmetic breadth
and full native/release qualification remain required, unclosed work.

### Natural temporal identity refinement (2026-09-09)

MySQL natural DATE/DATETIME now retains native payload and datatype while decoder-qualified identity keys match canonical Rust output; query constants remain verbatim. Required owned TLS CLI checks cover canonical/noncanonical fixed and sameTerm matches, DATE/leap/year-zero/extrema and DATETIME fractions, duplicate bags, nested projection, mixed literal/IRI joins, NULL/negation, invalid hidden SELECT/COUNT/ASK terms, cap-one recovery and denied-invalid-row policy/existential/OPTIONAL isolation. Coercing mixed temporal SubPlans retain a rejection marker instead of falling back to raw equality. Wider natural/native identity and native-consumer copies remain open.

Rust remains the final natural-literal canonicalizer; source DATE/DATETIME fields
and their datatype codes are not text-replaced. SQL set-work compares only a
separately qualified canonical identity key: calendar validity (including accepted
year zero) and trimmed DATETIME fractional zeros. Explicit datatype/language and
IRI construction retain decoder lexicals; sameTerm constants are never canonicalized.
Original natural-term guards survive hidden projection/COUNT/ASK. Missing or
incompatible temporal provenance is not xsd:string or native-equality authority.
These are existing ADR-0007/0034 comparison refinements, not a second RDF generator.

### Identifier resolution — lenient against the live schema (decision 2026-06-28)

**Literal comparison refinement (2026-09-09).** RDF identity is the decoded
lexical/datatype/language tuple, separate from numeric FILTER value promotion.
SQLite compiler-owned predicates use shared Rust decoding and natural-literal
canonicalization, not SQL casts; explicit datatype/language literals preserve
their original lexical form. `sf-core::numeric_compare` validates the original
four base numeric lexical spaces before integer/decimal/float/double promotion,
avoids decimal-to-float double rounding and keeps NaN out of total-order rules.
Invalid lexical operands produce expression error; the pinned numeric library's
representation limits fail closed rather than misclassify valid XSD values.
Callbacks charge work before parsing and release request state at teardown.
Numeric VALUES compare in Rust on every dialect; existing nonnumeric VALUES
variable-pair equality is retained. Different plain-column RDF kinds compare
false when bound but preserve expression errors when unbound, including NOT.
Datatype/language identifiers use byte-exact comparison, not native collation.
The public signed-zero, datatype/language, large-integer/decimal and OPTIONAL
checks cover this SQLite slice. Native/natural/mixed-descriptor generalization
remains open under ADR-0034; missing metadata is never proof of xsd:string.

**Column-IRI refinement (2026-09-09).** SQLite identity predicates retain each
operand's processor output base and generate decoded IRIs through the same
`sf-core::term::column_iri` function as RDF reconstruction. Absolute spellings
remain verbatim. **Normative correction:** [R2RML §4/§11.2](https://www.w3.org/TR/r2rml/#generated-rdf-term) requires simple base/value concatenation, not RFC3986 resolution; leading slashes and dot segments remain. The earlier resolver and non-concatenation wording were incorrect.
The query-owned callback charges value/base bytes before UTF-8 validation and
resolution, preserves NULL/data errors and releases request state. D1 and native
reference atoms retain raw values/descriptors while partitioning on every
required decoded/resolved RDF key. Column blank labels retain separate graph
identity. Missing decoder authority and native IRI-resolution operations reject;
Independent processor/document bases now flow through `R2rmlOptions`; serving exposes
per-source fixed `--mapping-base` / `--mapping-base-2`, with bounded validation,
layered precedence, compiler identity and reload retention. Turtle `@base` changes
mapping syntax only. PostgreSQL/MySQL resolved-key support remains required
follow-up; the late-template slice below qualifies only its stated paths (ADR-0034).

**IRI substitution alphabet correction (2026-09-09).** R2RML §7.3 permits
RFC3987 `iunreserved`, not arbitrary Unicode. One shared `ucschar` range table
now governs Rust reconstruction, fixed RDF-star ID components and all three SQL
template encoders. C1 controls,
private-use and excluded noncharacters become uppercase percent-encoded UTF-8
bytes; allowed CJK/supplementary characters remain unchanged. Literals are not
encoded and existing percent signs are encoded once as input, not interpreted.
Byte-oriented SQLite/MySQL rendering preserves NUL and rejects malformed UTF-8
instead of silently repairing it; full windows, continuation bytes and binary
comparisons establish membership. PostgreSQL classifies code points numerically
and escapes their UTF-8 bytes. Existing MySQL packet/aggregate limits remain.
Core boundary cases, SQLite SQL/HTTP identity and COUNT, and authenticated owned
PostgreSQL16.15/MySQL8.4.11 different-shape template equality checks pass. This
removes an unsafe assumption before late-base classification; general template
identity remains open outside the following qualified slice.

**Late-template refinement (2026-09-09).** Template substitutions are encoded
once, then the expanded string is used if it is a valid absolute IRI; otherwise
the processor base is prepended verbatim and the result validated. Dynamic
schemes, partial schemes, authority/port slots and split fixed percent escapes
retain `TermSpec.base` through reconstruction and typed SQL identity. A static
fast path requires a grammar proof: a fixed scheme, committed path/query/fragment,
valid empty-slot skeleton and complete fixed percent escapes. Statically relative
recipes may bake the base only when every expansion is non-absolute and the
prefixed recipe passes that proof. Full-literal recipes fold at parse time.
Required core/parser tests cover both base branches, empty/NULL substitutions,
escaping and the pinned parser's Unicode/host boundaries.

Authenticated SQLite queries now compare and deduplicate finalized template IRIs
using live per-part decoders and the existing work-charged Rust IRI callback.
Invalid values remain data errors under COUNT; no column is required to register
the callback. Original template guards survive constant binding and hidden terms.
The qualified atom has one original table/query and IRI-template/constant keys;
mixed natural/literal-column keys, native resolved-template SQL, reference atoms,
recursive endpoints, late RDF-star proposition components and wider pooling
remain explicitly unsupported pending their own exact lowering. Serving still
rejects dynamic predicates; raw direct matching is constrained, with nontrivial
entailment alternatives rejected. No backend/admission flag changes.

R2RML §5 mandates **SQL:2008 identifier comparison**: regular (undelimited) identifiers are case-insensitive; delimited identifiers are case-sensitive; an all-upper-case delimited identifier equals the undelimited form (`DEPTNO` = `"DEPTNO"`) but a mixed-case delimited one does not (`"Name"` ≠ regular `Name`). A strict processor therefore **rejects** a mapping that references a mixed-case delimited column with a regular identifier.

**Decision: resolve every mapping column identifier against the *live introspected schema* — exact match first (preserves a genuinely delimited/case-exact column), then a unique ASCII-case-insensitive match — rather than implement strict SQL:2008 delimited-vs-regular rejection.** Rationale:
* **Provenance is unrecoverable by a virtualiser.** SQL:2008 comparison needs the delimited-vs-regular status of the *actual column*. We learn columns by introspection (`information_schema` / `PRAGMA`), which returns bare name strings; SQLite (case-insensitive, no delimitation record) erases the distinction entirely, and PG only partially exposes it. Strict rejection would risk false-rejecting valid mappings.
* **The suite itself is predominantly lenient.** The W3C cases that exercise this pattern as **positive** cases (`R2RMLTC0002a`, `R2RMLTC0018a` / D018 — `rr:column "Name"` / `{Name}` against a delimited `"Name"`) *expect success*. On a dialect whose identifiers are case-insensitive or whose catalog erases delimited-vs-regular provenance (SQLite; MySQL semantics), implementing strict rejection would fail those positives to satisfy the single **negative** case `R2RMLTC0002f` — a **net conformance loss**. Both the positive and negative cases here are W3C `test:reviewStatus test:unreviewed`. *(Corrected 2026-07-20: this trade-off argument is per-dialect, not universal — see the correction below.)*
* **Consistent with production OBDA.** Reference engines resolve against the catalog rather than re-deriving SQL identifier folding.

**Consequence — one documented deviation:** `R2RMLTC0002f` (a negative test expecting rejection) is not rejected by the engine. It is recorded in `sf_conformance::EXPECTED_DEVIATIONS`, reported truthfully as `earl:failed`, and **excluded from the regression gate** (`Report::unexpected_failures`) — so it cannot mask a real future regression while also not being a perpetual red bar. Revisit if a future need requires strict identifier validation (it would be a distinct, opt-in mapping-validation pass, not the resolution path).

> **Correction (2026-07-20) — the rejection-is-impossible argument is per-dialect,
> not universal.** This ADR originally implied strict `0002f` rejection would
> break the positive twins on any backend. Verified against the maintained
> [R2RML implementation report](https://rml.io/r2rml-implementation-report)
> (Ontop v4.1.0, tested 2021-03-11): **Ontop passes `R2RMLTC0002f` alongside
> `R2RMLTC0002a` and `R2RMLTC0018a` on PostgreSQL** — and fails `0002f` on
> MySQL, exactly like this engine. The honest split: **SQLite/MySQL semantics**
> — identifiers case-insensitive (SQLite regardless of quoting), no
> delimited-vs-regular provenance; accepting is *correct for the database* and
> the net-conformance-loss argument stands. **PostgreSQL** — unquoted
> identifiers case-fold, so `{Name}` genuinely does not match a delimited
> `"Name"`; rejection is achievable without breaking the positives, and our
> case-insensitive fallback erasing that distinction is a **resolver-design
> choice**. Closing it (a delimited-aware PG resolver) is an **open parity
> item** (README §9), deliberately not yet built; until then `0002f` remains a
> documented deviation on both dialects. The `EXPECTED_DEVIATIONS` rationale
> and both W3C suite headers carry this same correction.

### Consequences

* Good, because byte-identical RDF output across dialects by construction; reuses `oxsdatatypes` (already in-stack); the parity contract is directly testable.
* Bad, because a per-dialect type map plus the SQLite per-value branch are real surface to maintain.

### Confirmation

A `(SQL source type × dialect) → expected RDF literal` matrix, realised as **per-DBMS forked golden N-Triples fixtures** (the RML-community layout; ADR-0012) run against real PostgreSQL/SQLite, plus: **cross-dialect** byte-identity (the §10 consistency clause), Rust canonicalization unit tests over the raw dialect renderings, and SQLite affinity-violation + STRICT tests. The W3C RDB2RDF suite (ADR-0005) is the floor.

> **Implementation boundary (2026-09-06).** The current exact RDB2RDF inventory
> contains only the canonical fixture names; it has no per-DBMS fork files yet.
> SQLite, required-live PostgreSQL, and required-live MySQL execute only bytes
> captured by that seal and record them in backend-aware v5 outcome receipts.
> MySQL records 74 pass, one documented `R2RMLTC0002f` deviation, and 12 exact
> typed Direct Mapping unsupported outcomes under `RequirePrimaryKey`; its
> conformance-only type-profile identifier is part of outcome identity. Receipt
> provider image/toolchain provenance remains explicitly unbound, so pinned-image
> CI/live evidence qualifies only this mapping baseline—not native product type
> provenance, backend admission, or full Query/Protocol conformance. A future
> dialect fork must enter the inventory before execution; runtime discovery or
> rereading after the sealing barrier is prohibited. This closes snapshot
> immutability for the current baseline, not the full per-dialect golden matrix.

> **Amendment (2026-07-16, impl-verified).** This ADR's Confirmation clause calls
> for "Rust canonicalization unit tests over the raw dialect renderings" per
> dialect. SQL Server's had none: `marshal_column_data` (`sf-sql`) converts
> `tiberius`'s typed `ColumnData` into a lexical string *before* anything reaches
> this ADR's chokepoint — and that per-driver decode step is exactly where a
> real bug lived, undetected, because it produces a syntactically-valid-but-
> factually-wrong string (e.g. `"1970-01-01"` for a stored `0001-01-01`) that
> `oxsdatatypes` has no way to catch: it validates lexical *form*, not whether
> the driver decoded the value correctly. `date_from_proleptic()`'s epoch
> arithmetic was wrong (fed a Rata Die day-count into an algorithm expecting
> days-since-1970-01-01), compounded by an independently-wrong constant in the
> 1900 epoch path. Fixed, and now covered by 8 new direct unit tests plus a live
> round-trip test against a real SQL Server container — see ADR-0026 for the
> full account. Worth remembering: this class of bug is invisible to the
> chokepoint's own correctness guarantees precisely because it happens
> upstream of it, in per-dialect driver decoding — any *new* per-dialect decode
> step (a future backend's own date/time marshaling) needs its own such tests,
> not just trust that the shared chokepoint will catch it.

## More Information
* **Term-generation home:** `sf-core` (ADR-0003 R3). **Execution:** ADR-0006. **Conformance:** ADR-0005. **Test strategy:** ADR-0012.
* **Research:** `docs/research/` — `dialect-correctness`, `r2rml-spec-tests`.
