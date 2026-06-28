# SOTA — external-memory join for R2RML refObjectMaps (the PJTT OOM)

**Research key:** `external-memory-join`
**Date:** 2026-06-27 (round-2 deep-research)
**Scope:** R2RML referencing-object-map joins (child↔parent triples-map equi-joins via `rr:joinCondition`; possibly multiple ANDed conditions; same or different logical source) at scale, where an in-memory index of the whole parent table OOMs.
**Decision recorded in:** ADR-0015 (push computation into the source; DuckDB only for cross-source).

## Bottom line

**Push the join into the source SQL** whenever child and parent share one RDBMS source (the OntopM path): emit a single `SELECT child.*, parent.* FROM (<child>) JOIN (<parent>) ON c1=p1 AND …` and let the source execute it — bounded/indexed/spill-handled by the DB, **correct multi-condition + NULL semantics for free**, and it sidesteps the entire KROWN multi-join / N:M failure cliff. Keep the in-memory PJTT hash index **only for small parents**. For genuine **cross-source** joins (child/parent in different DBs, or a file source) join **in-engine with DuckDB** — because **DataFusion's hash join still does not spill** (June 2026).

## 1. Push-down vs in-engine (primary axis)

- **Push down (preferred)** when child+parent reference the same DB connection and the source is a target dialect. Multiple join conditions = `AND`-ed equalities (the source can use composite indexes). **NULL correctness:** `INNER JOIN ON a=b` never matches NULL (`NULL=NULL` is UNKNOWN) → child/parent rows with NULL join keys produce **no** triples = the intended R2RML result. semantic-fabric authors the SQL itself, so emitting the JOIN directly via `connector_arrow`/`tokio-postgres` needs no planner; `datafusion-table-providers`+`-federation` would auto-push same-source joins if a planner is wanted.
- **In-engine (must)** when child/parent are in different sources — no single source can run the JOIN.

## 2. The decisive maturity fact (June 2026)

- **DataFusion `HashJoinExec` does NOT spill** — open EPIC `apache/datafusion#17267`; Comet 0.15.0 release notes confirm; root cause = no cross-partition spill coordination. A large build side OOMs exactly like the PJTT. **Do not** treat DataFusion's default join as the bounded fallback.
- **DataFusion `SortMergeJoinExec` DOES spill** (#9359 → PR #11218): set `datafusion.optimizer.prefer_hash_join = false`, build a `RuntimeEnv` with a `FairSpillPool` + `DiskManager`. Caveat: SMJ buffers all rows sharing the current join-key value → an extreme single-hot-key N:M still pressures memory.
- **DuckDB has a mature, purpose-built out-of-core hash join** — VLDB 2025 "Saving Private Hash Join" + v1.2.0: adaptive external radix-partitioned hash join (4→4096 partitions until each fits), spills both sides into a unified buffer pool, runtime column compression, graceful degradation. Via `duckdb-rs`; spill automatic at `memory_limit` into `temp_directory`. **The most robust in-engine option.**

## 3. SOTA spilling-join algorithms (the fallback toolbox)
Grace hash join (partition both sides to disk, join pairs in memory); hybrid hash join (keep one partition resident); **radix-partitioned hash join** (cache-conscious; fastest in practice — Kim VLDB'09, Balkesen "Sort vs Hash Revisited" VLDB'14; DuckDB uses this); external sort-merge join (robust to N:M). **Build on the smaller side**; on a bad cardinality estimate, engines reverse roles / fall back to SMJ. Hot-key skew → repartition with more radix bits or SMJ fallback.

## 4. KROWN failure modes the design avoids
RMLMapper **times out at 5/10/15 join conditions**; RMLStreamer-CSV doesn't support multiple join conditions; SDM-RDFizer errors on multiple Graph Maps; "all systems fail/timeout the 15-named-graph dynamic case." These engines **materialise-then-join in memory** — a pushed-down indexed JOIN or a spilling engine join avoids the cliff. (See also FunMap — join elimination by pre-materialisation; and the 2025 *Algebraic Foundation for KGC* — relational-algebra mapping-plan rewrites incl. join pushdown — useful planner prior art.)

## 5. Tiered recommendation

- **Tier 0 — mapping-level elimination (free, always):** drop redundant self-joins (Morph-KGC/Ontop); a refObjectMap with **no** `rr:joinCondition` ⇒ use the parent's subject IRI (no join); parent==child on a PK ⇒ scan.
- **Tier 1 — PRIMARY: SQL push-down (same source).** One indexed `JOIN` at the source. Correct multi-condition + NULL natively.
- **Tier 2 — cross-source / large fallback: DuckDB external hash join** (graceful degradation) or DataFusion **SortMergeJoin + spill** (never DataFusion HashJoin for large parents). Pick build side from source stats.
- **Tier 3 — never for large parents: unbounded in-memory PJTT** (keep only below a row/byte threshold).

### Decision table
| Child vs parent source | Shape | Parent size | Strategy |
|---|---|---|---|
| Same DB | any (incl. N:M, many conditions) | any | Tier 1 push-down |
| parent==child on PK | self-join | any | Tier 0 eliminate |
| refObjectMap, no join condition | — | any | Tier 0 (parent subject IRI) |
| Different sources | 1:N / N:1 | small | in-memory PJTT |
| Different sources | any | large | Tier 2 DuckDB external hash join |
| Different sources | **N:M** | large | Tier 2 + hot-key skew mitigation |

## Evidence grades
- DataFusion HashJoin no-spill / SMJ spills — **A** (issues #17267/#9359 + Comet 0.15.0 + docs.rs).
- DuckDB external radix hash join — **A** (VLDB 2025 + v1.2.0).
- radix-hash superiority; build-smaller-side — **A** (Kim'09, Balkesen'14).
- KROWN join failures; Morph-KGC self-join-only; SDM-RDFizer PJTT — **A**.
- Ontop constraint-based self-join elimination detail — **B** (corroborated; primary PDF didn't render).

## Sources
- https://github.com/apache/datafusion/issues/17267 · .../issues/9359 · https://docs.rs/datafusion/latest/datafusion/physical_plan/joins/struct.SortMergeJoinExec.html · https://datafusion.apache.org/blog/output/2026/04/18/datafusion-comet-0.15.0/
- https://www.vldb.org/pvldb/vol18/p2748-kuiper.pdf · https://duckdb.org/2022/09/30/postgres-scanner · https://github.com/datafusion-contrib/datafusion-table-providers · https://github.com/datafusion-contrib/datafusion-federation · https://docs.rs/connector_arrow
- http://www.vldb.org/pvldb/vol7/p85-balkesen.pdf · https://15721.courses.cs.cmu.edu/spring2016/papers/kim-vldb2009.pdf
- https://github.com/kg-construct/KROWN · https://link.springer.com/chapter/10.1007/978-3-031-77847-6_2 · https://research.bcgl.fr/pdfs/ontop-iswc20.pdf · https://arxiv.org/pdf/1605.04263
- https://journals.sagepub.com/doi/10.3233/SW-223135 (Morph-KGC) · https://www.semantic-web-journal.net/system/files/swj3246.pdf (SDM-RDFizer PJTT) · https://arxiv.org/pdf/2503.10385 (Algebraic Foundation for KGC) · https://arxiv.org/pdf/2008.13482 (FunMap)
