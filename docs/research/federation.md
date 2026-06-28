# Cross-source federation (relational, post-DataFusion)

**Date:** 2026-06-27 · **Decision:** ADR-0006 (execution). **Evidence:** [A]/[B]/[C].

## Correction this report establishes
The claim that DuckDB `postgres_scanner`/`ATTACH` "pushes filters, aggregates **and joins**" is **false** for the direction we use (DuckDB reaching out to a source). That capability is **`pg_duckdb`** (DuckDB embedded *inside* Postgres — the opposite direction). The `ATTACH`/scanner path pushes down **filter + projection + LIMIT only; joins and aggregates are pulled into DuckDB.** [A] (MotherDuck postgres-duckdb-options; Crunchy; DuckDB postgres docs; duckdb-postgres DeepWiki.)

## Findings
- Consequence: a **same-source** join must never go through ATTACH-and-join (DuckDB would pull both tables and join locally, losing the source index). Push it via the source's native driver, or the **`postgres_query()`/`mysql_query()`/`sqlite_query()` pass-through** (ships raw sub-SQL to the source — the linchpin for co-located sub-plans). [A]
- DuckDB's CBO **lacks reliable statistics for ATTACHed foreign tables** → do not let it choose the cross-source side/order; the engine must compute cardinalities (`pg_class.reltuples`, `information_schema`, Parquet footer, `sqlite_stat1`) and construct the plan. [A/B]
- **SOTA cross-source technique = semi-join reduction**, not pulling both sides into a join engine: **Teiid dependent join** (3 modes: semi-join IN-list / key pushdown / data-movement), **Trino dynamic filtering** (runtime build→probe semi-join), **Predicate Transfer** (CIDR 2024, Yannakakis + Bloom, ~3.1× over Bloom-join). Near-perfect fit for R2RML refObjectMap joins (parent side = a thin key + subject-template projection). [A]
- Ontop is single-endpoint and **delegates** federation to an external SQL federator; semantic-fabric chooses to *be* that layer — inheriting side-selection/stats/semi-join. [A]

## Decision (ADR-0006)
Relational cross-source joins use **semi-join reduction / data-movement via the source drivers**, with cardinality-driven side selection — **no pulled-in OLAP engine** (no DuckDB/DataFusion on the relational path). A Rust spilling hash join is the last-resort fallback for a pathological large N:M only.

## Sources
- DuckDB postgres ext: https://duckdb.org/docs/current/core_extensions/postgres/overview · scanner pushdown limits (DeepWiki): https://deepwiki.com/duckdb/duckdb-postgres/4.1-query-optimization · postgres_scanner vs pg_duckdb: https://motherduck.com/blog/postgres-duckdb-options/ · pg_duckdb: https://github.com/duckdb/pg_duckdb
- DuckDB join-order w/o stats (thesis): https://blobs.duckdb.org/papers/tom-ebergen-msc-thesis-join-order-optimization-with-almost-no-statistics.pdf
- Teiid dependent join: http://teiid.github.io/teiid-documents/master/content/dev/Dependent_Join_Pushdown.html · Trino dynamic filtering: https://trino.io/blog/2019/06/30/dynamic-filtering.html · Predicate Transfer (CIDR 2024): https://www.cidrdb.org/cidr2024/papers/p22-yang.pdf · Denodo data-movement: https://community.denodo.com/kb/en/view/document/Denodo%20Query%20Optimizations%20for%20the%20Logical%20Data%20Warehouse
- Ontop federation: https://ontop-vkg.org/tutorial/federation/dremio/
