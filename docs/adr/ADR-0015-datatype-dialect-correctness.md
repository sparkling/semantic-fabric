---
status: accepted
date: 2026-06-27
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
* **Catalog-driven type determination + one Rust canonicalization chokepoint (chosen)** — determine the target XSD datatype from catalog metadata (per-dialect `DbTypeMap`), then produce the XSD canonical lexical form in Rust via `oxsdatatypes`, with a per-value SQLite storage-class branch for dynamic typing.
* **Strict SQL:2008 delimited-vs-regular identifier rejection** — rejected for identifier resolution: provenance is unrecoverable by a virtualiser and strict rejection is a net conformance loss against the predominantly-lenient W3C suite. Chosen instead: resolve every mapping column identifier against the live introspected schema (exact match, then unique ASCII-case-insensitive match).

## Decision Outcome

**Never trust the driver's rendering for a non-string value. Determine the target XSD datatype from catalog metadata, then produce the XSD canonical lexical form in Rust.** Two layers:

1. **Type determination — a per-dialect `DbTypeMap`** (the Ontop `DBTypeFactory` analogue): native source type → internal `XsdTypeCode`, read from the catalog (`information_schema`/`pg_catalog` for PostgreSQL; `PRAGMA table_info` for SQLite; `information_schema.COLUMN_TYPE` for MySQL, to disambiguate `tinyint(1)`).
2. **Value canonicalization — one Rust chokepoint** in `sf-core` term generation. Fetch each value in the most type-faithful driver form (binary/typed over text), parse into `oxsdatatypes`, and emit via its `Display`, which **is** the XSD canonical mapping — so literals round-trip through `oxttl`/`oxrdf` byte-identically using the same code path Oxigraph itself uses. `oxsdatatypes` does not cover `xsd:hexBinary`; a small uppercase-hex encoder handles it. Canonicalization is keyed on the value's **target XSD type**, never on the dialect's text. **Do not push canonicalization into SQL** (scientific notation, decimal trimming, hex casing are fragile across dialects and the cross-source read goes through other renderers anyway): SQL does set-work, Rust does lexical form.

> **Reconciliation note (2026-06-28, impl-verified).** "emit via its `Display`" is exact for every XSD type the engine canonicalizes **except `xsd:double` / `xsd:float`**: in `oxsdatatypes` 0.2.2 their `Display` delegates to Rust `f64`/`f32` formatting, which is **not** XSD-canonical (e.g. `1.0` → `1`, no mandatory `E`-notation — contradicting the "`E`-notation everywhere" requirement above). For those two types the single `sf-core` chokepoint still **parses/validates through `oxsdatatypes`** but emits the XSD-canonical scientific form itself (mantissa with ≥ 1 fractional digit, uppercase `E`, no leading-zero exponent; `INF`/`-INF`/`NaN`); all other types use `oxsdatatypes` `Display` directly as stated. This is a documentation correction only — the implemented output is XSD-canonical per §10 (verified by the `sf-core` canonical-double tests). Companion note in ADR-0006 §Term generation.

**SQLite — the special hazard (dynamic typing / type affinity).** A column's declared type is only a recommendation; values carry their own storage class. Policy: **branch on the per-value storage class** (`sqlite3_column_type()`), with a fast-path when the table is declared `STRICT` (3.37+). A documented, tested contract.

**`sqlparser` is SQL syntax only** — used for SQL emission and parsing `rr:sqlQuery`; it contributes nothing to type semantics, which is this separate subsystem. **NULL** in any referenced column ⇒ no RDF term (R2RML §11), enforced in Rust (not via SQL concat NULL-semantics).

### Identifier resolution — lenient against the live schema (decision 2026-06-28)

R2RML §5 mandates **SQL:2008 identifier comparison**: regular (undelimited) identifiers are case-insensitive; delimited identifiers are case-sensitive; an all-upper-case delimited identifier equals the undelimited form (`DEPTNO` = `"DEPTNO"`) but a mixed-case delimited one does not (`"Name"` ≠ regular `Name`). A strict processor therefore **rejects** a mapping that references a mixed-case delimited column with a regular identifier.

**Decision: resolve every mapping column identifier against the *live introspected schema* — exact match first (preserves a genuinely delimited/case-exact column), then a unique ASCII-case-insensitive match — rather than implement strict SQL:2008 delimited-vs-regular rejection.** Rationale:
* **Provenance is unrecoverable by a virtualiser.** SQL:2008 comparison needs the delimited-vs-regular status of the *actual column*. We learn columns by introspection (`information_schema` / `PRAGMA`), which returns bare name strings; SQLite (case-insensitive, no delimitation record) erases the distinction entirely, and PG only partially exposes it. Strict rejection would risk false-rejecting valid mappings.
* **The suite itself is predominantly lenient.** The W3C cases that exercise this pattern as **positive** cases (`R2RMLTC0002a`, `R2RMLTC0018a` / D018 — `rr:column "Name"` / `{Name}` against a delimited `"Name"`) *expect success*. Implementing strict rejection would fail those positives to satisfy the single **negative** case `R2RMLTC0002f` — a **net conformance loss**. Both the positive and negative cases here are W3C `test:reviewStatus test:unreviewed`.
* **Consistent with production OBDA.** Reference engines resolve against the catalog rather than re-deriving SQL identifier folding.

**Consequence — one documented deviation:** `R2RMLTC0002f` (a negative test expecting rejection) is not rejected by the engine. It is recorded in `sf_conformance::EXPECTED_DEVIATIONS`, reported truthfully as `earl:failed`, and **excluded from the regression gate** (`Report::unexpected_failures`) — so it cannot mask a real future regression while also not being a perpetual red bar. Revisit if a future need requires strict identifier validation (it would be a distinct, opt-in mapping-validation pass, not the resolution path).

### Consequences

* Good, because byte-identical RDF output across dialects by construction; reuses `oxsdatatypes` (already in-stack); the parity contract is directly testable.
* Bad, because a per-dialect type map plus the SQLite per-value branch are real surface to maintain.

### Confirmation

A `(SQL source type × dialect) → expected RDF literal` matrix, realised as **per-DBMS forked golden N-Triples fixtures** (the RML-community layout; ADR-0012) run against real PostgreSQL/SQLite, plus: **cross-dialect** byte-identity (the §10 consistency clause), Rust canonicalization unit tests over the raw dialect renderings, and SQLite affinity-violation + STRICT tests. The W3C RDB2RDF suite (ADR-0005) is the floor.

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
