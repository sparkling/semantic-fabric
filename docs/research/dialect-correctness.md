# SQL dialect correctness for R2RML §10 datatype mapping

**Date:** 2026-06-27 · **Decision:** ADR-0015. **Evidence:** [H]igh / [M]ed / [L]ow.

## Question
How to produce **byte-identical** RDF literals from SQL values across modes (materialize/virtualize) and dialects (PostgreSQL, SQLite, MySQL), honouring R2RML §10's *consistency* clause.

## Findings
- **§10 natural mapping** is normative: CHAR/VARCHAR→plain; INTEGER→`xsd:integer`; DECIMAL/NUMERIC→`xsd:decimal`; FLOAT/DOUBLE→`xsd:double`; BOOLEAN→lowercase `true`/`false`; DATE/TIME/TIMESTAMP→`xsd:date`/`time`/`dateTime` (space→`T`); BINARY→`xsd:hexBinary`; INTERVAL undefined. The consistency clause requires `(type,value)` → one lexical form, regardless of source type/dialect. [H]
- **Driver text rendering is non-canonical or wrong for ≥1 core type in every candidate DB:** PostgreSQL booleans `t`/`f`, `bytea` `\x`+lowercase, hour-only tz offsets, space-separator timestamps; **`xsd:double` needs `E`-notation (`3.14E0`) everywhere**; decimals return scale-padded. [H]
- **SQLite = the hazard: dynamic typing / type affinity.** A column's declared type is only a recommendation; values carry their own storage class (NULL/INTEGER/REAL/TEXT/BLOB); no `information_schema` (use `PRAGMA table_info` + per-value `sqlite3_column_type()`); STRICT tables (3.37+) restore rigidity. [H]
- **Ontop's two-layer architecture is the reference:** `DBTypeFactory` (native-type → semantic type map, with lexical overrides e.g. `getDBTrueLexicalValue`) + `DBFunctionSymbolFactory`. [H]
- **`sqlparser-rs` is syntax/AST only** ("avoids applying SQL semantics") — good for emission, contributes nothing to type mapping. [H]
- **`oxsdatatypes` `Display` is the XSD canonical mapping** and is already in-stack (Oxigraph), so literals round-trip through `oxttl`/`oxrdf` byte-identically; it lacks `hexBinary` (write a small uppercase-hex encoder). [H]
- MySQL (deferred): `BOOLEAN`=`TINYINT(1)`→`0/1` (outside §10.2 → interop divergence, `rml-core#100`); `||` is OR unless `PIPES_AS_CONCAT`. [H/M]

## Decision (ADR-0015)
Never trust driver rendering. **Layer 1:** per-dialect `DbTypeMap` determines the target XSD type from catalog metadata. **Layer 2:** canonicalize in a single Rust chokepoint (`sf-core`) via `oxsdatatypes` + hexBinary encoder, keyed on the target XSD type. SQLite: branch on per-value `sqlite3_column_type()` + STRICT fast-path. Don't push canonicalization into SQL. Test: per-DBMS forked golden N-Triples + cross-dialect + cross-mode byte-identity + canonicalization unit tests + SQLite affinity/STRICT tests (ADR-0012); W3C RDB2RDF suite is the floor (ADR-0005).

## Sources
- R2RML §10: https://www.w3.org/TR/r2rml/#natural-mapping · SQL→XSD wiki: https://www.w3.org/2001/sw/rdb2rdf/wiki/SQL_to_XSD_Type_Mappings.html · XSD 1.1 pt2: https://www.w3.org/TR/xmlschema11-2/
- SQLite affinity: https://sqlite.org/datatype3.html · PG binary/datetime: https://www.postgresql.org/docs/current/datatype-binary.html , .../datatype-datetime.html · Npgsql type reps: https://www.npgsql.org/doc/dev/type-representations.html
- Ontop DB adapter: https://ontop-vkg.org/dev/db-adapter.html · sqlparser-rs: https://docs.rs/sqlparser/latest/sqlparser/dialect/ · oxsdatatypes: https://docs.rs/oxsdatatypes
- RML per-DBMS test cases: https://github.com/kg-construct/rml-test-cases · rml-core#100: https://github.com/kg-construct/rml-core/issues/100
