# DuckDB as the heterogeneous (non-relational file) reader

**Date:** 2026-06-27 · **Decision:** ADR-0002 (3-tier heterogeneous scope), ADR-0006 (file-reader role). **Evidence:** [A]/[B]/[C].

## Scope note
This concerns **non-relational file sources only** (CSV/JSON/Parquet/XML). DuckDB is **not** an intermediary on the relational/OLTP path — relational execution is native-driver push-down + semi-join reduction (ADR-0006, `federation.md`). This report assesses DuckDB as the *file* reader for the deferred full-RML story.

## Findings
- **Proven pattern:** Morph-KGC (SOTA RML engine) already uses DuckDB to evaluate RML Views over tabular/JSON sources. [A]
- Each RML reference-formulation → a DuckDB read strategy: `rml:CSV`→`read_csv`; `rml:JSONPath`→`read_json` + `json_extract` + **`UNNEST` (= the RML iterator)**; Parquet/Excel→`read_*`; RML-LV→a DuckDB `VIEW`. DuckDB JSON path is a *subset* (no filter predicates → relocate to SQL `WHERE`). [A]
- **XML:** no native support; the **`webbed` community extension** adds `read_xml` + XPath **1.0**. Caveats: XPath 1.0 only (RML/Morph-KGC use 3.0), namespace `local-name()` workarounds, **unsigned community extension** (supply-chain/governance flag for an embedded Rust engine) — even Morph-KGC keeps XML *outside* DuckDB on an XPath-3.0 path. [B]
- **Out of scope for any SQL reader:** Façade-X, code-walked sources (e.g. Roslyn C#), streaming (Kafka/MQTT). [A]

## Decision (ADR-0002)
Re-scope the heterogeneous deferral into 3 tiers: (a) DuckDB-native CSV/JSON/Parquet/Excel = near-term cheap win (a thin compiler from reference-formulation IRIs to DuckDB SQL; the IR is already open-IRI-shaped); (b) XML = bounded decision (`webbed` vs pre-transform); (c) Façade-X/code/streaming = true deferral. DuckDB appears only here, off the relational and runtime paths.

## Sources
- DuckDB JSON: https://duckdb.org/docs/current/data/json/loading_json , .../json_functions · multi-format/`read_*`: https://duckdb.org/docs/lts/core_extensions/excel
- `webbed` ext: https://duckdb.org/community_extensions/extensions/webbed · https://github.com/teaguesterling/duckdb_webbed
- Morph-KGC RML Views (DuckDB): https://morph-kgc.readthedocs.io/en/latest/documentation/ · ESWC 2023 "Boosting KG Generation from Tabular Data with RML Views": https://2023.eswc-conferences.org/wp-content/uploads/2023/05/paper_Arenas-Guerrero_2023_Boosting.pdf
- RML-IO / RML-LV specs: https://kg-construct.github.io/rml-io/spec/docs/ · https://kg-construct.github.io/rml-lv/spec/docs/
