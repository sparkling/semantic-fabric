# SOTA — external-memory deduplication (the PTT OOM)

**Research key:** `external-memory-dedup`
**Date:** 2026-06-27 (round-2 deep-research)
**Scope:** bounded-memory, correctness-critical triple `DISTINCT` for the R2RML→RDF materializer; the OOM risk at LOW duplicate rate where the dedup set grows ~ to the unique-triple count.
**Decision recorded in:** ADR-0015 (push computation into the source) + ADR-0006 (decision update).

## Bottom line

Exact `DISTINCT` provably needs Ω(distinct-count) state **somewhere**, so bounded RAM ⇒ **the authority must be allowed to spill** (hash-partition or sort). Probabilistic structures (Bloom/cuckoo/xor/quotient) are an *optimization layer*, **never** the authority.

**For semantic-fabric specifically (per ADR-0015): push `SELECT DISTINCT` into the source DB**, which spills natively (Postgres `work_mem`→temp; SQL Server `tempdb`; SQLite external-merge sorter; MySQL; DuckDB). The DataFusion-vs-bespoke question only arises for the **cross-source residual**, where the in-engine executor is **DuckDB** (mature out-of-core), not DataFusion. The tiers below are the fallback design for in-engine execution.

## Why "must spill" — and why filters can't substitute

A probabilistic filter answers "definitely-new" (no false negatives) or "maybe-seen". In a DISTINCT:
- definitely-new ⇒ emit + insert, skip the exact check (safe);
- maybe-seen ⇒ could be a true dup **or a false positive** → you **must** consult the exact set. **Dropping on the filter's word loses genuinely-new triples = silent data corruption.**

So a pre-filter saves CPU/IO, **not** the peak memory of the exact set. And the OOM trigger here is *low* dup rate, where the exact set ≈ the whole output (a filter can't shrink it) and dedup has low value anyway — at the OOM point the right move is **skip or spill**, not "add a Bloom filter" (filters shine in the opposite, high-dup regime where there is no OOM).

## Tiered in-engine design (cross-source / file-source fallback)

- **Tier 0 — don't dedup what can't duplicate.** Statically mark partitions provably dup-free (subject from a key + single-valued P/O over a source unique on the projected columns) and skip the set entirely. Exploits the Morph-KGC partition disjointness already computed. Biggest, cheapest win.
- **Tier 1 — in-memory fast path.** Per-predicate `FxHashSet` over a packed `u128` (or `u64`) key, bounded by a memory reservation (not unbounded).
- **Tier 2 — bespoke radix-partition + spill (the OOM fix).** On reservation overflow, hash `(s,o)` into K disk buckets each < budget → dedup each with an `FxHashSet` → recurse on any bucket still too big (skew). Bounded = one bucket + write buffers; rayon-parallel. **This is DuckDB's published algorithm** minus the engine overhead.
- **Tier 2-alt — sort-based DISTINCT** when sorted output is wanted (sorted N-Triples / bulk load): external-merge-sort the fixed-width keys, drop adjacent equals (rides DataFusion's *mature* sort spill if delegating).
- **Tier 3 — optional probabilistic accelerators** (binary-fuse/Bloom as an I/O negative-cache over finished spill-runs — the LSM/SSTable pattern), never authorities.

## Delegate vs bespoke

- **DataFusion (now v54, not ~47):** sort spill is **mature** (v50 multi-level merge); **hash-aggregate/DISTINCT spill is the fragile path** (issues #7858/#13831/#8003 — "ResourcesExhausted despite spilling"; spills a partition's state as one batch reloaded whole). If delegating, route DISTINCT through ORDER BY + dedup (the sort path), not hash GROUP BY.
- **DuckDB:** mature out-of-core hash aggregation (ICDE 2024 — small fixed hash tables → buffer-manager spill → radix-partition more than #threads → lazy var-data pointers → gradual degradation, no cliff). In-process via the `duckdb` crate. Sharp edges only for large *values* (not our case — keys are u64 pairs).
- **Sort vs hash are duals** (Graefe TODS 2022): same external-memory I/O complexity; hash wins when the result fits memory (high dup), sort wins when input+output both large / tight memory / sorted output wanted. Keep both; default hash-partition.

## RoaringBitmap
Exact (can be authoritative) but **in-memory only** — a ceiling-raiser (5–50× on dense ids), not a spill substitute. Map `(s,o)`→one dense id + test-and-set in a `RoaringTreemap`; wins on dense/clustered ids (per-(predicate, subject) object sets), niche on the sparse full Cartesian space.

## Convergent SOTA (what the engines do)
DuckDB (radix + buffer-manager spill, gradual degradation), ClickHouse (two-level hash + `max_bytes_before_external_group_by` spill), Polars (partitioned spillable sinks), DataFusion (mature sort spill, weaker aggregate spill). **Common denominator = radix/hash-partition into memory-sized chunks + buffer-managed spill (or sort+merge fallback).**

## Evidence grades
- Exact DISTINCT needs Ω(distinct) + filters can't be authority — **High** (information-theoretic + LSM Bloom basis).
- DataFusion sort-spill mature, aggregate-spill fragile — **High/B** (release blogs + issue tracker).
- DuckDB out-of-core aggregation — **High** (ICDE 2024 paper + blog).
- Sort/hash duality — **High** (Graefe TODS/TKDE).
- Memory math (FxHashSet ~19 B/elem u128; Bloom ~9.6 b/elem @1%) — **C** (my calculation; validate before setting thresholds).

## Sources
- https://duckdb.org/2024/03/29/external-aggregation · https://duckdb.org/pdf/ICDE2024-kuiper-boncz-muehleisen-out-of-core.pdf · https://duckdb.org/2024/07/09/memory-management
- https://datafusion.apache.org/blog/output/2026/06/12/datafusion-54.0.0/ · https://datafusion.apache.org/blog/2025/09/29/datafusion-50.0.0/ · https://github.com/apache/datafusion/issues/7858 · .../issues/13831 · .../issues/8003
- https://arxiv.org/abs/2010.00152 (Graefe, sorting/dup-removal/aggregation) · https://arxiv.org/html/2411.13245v2 (hash-vs-sort empirical)
- https://github.com/tomtomwombat/fastbloom · https://github.com/ayazhafiz/xorf · https://lib.rs/crates/qfilter · https://docs.rs/roaring/latest/roaring/
- https://clickhouse.com/docs/sql-reference/statements/select/group-by · https://docs.pola.rs/user-guide/concepts/streaming/
- https://www.semantic-web-journal.net/system/files/swj3246.pdf (SDM-RDFizer PTT/DT) · https://journals.sagepub.com/doi/10.3233/SW-223135 (Morph-KGC partitions) · https://arxiv.org/pdf/1812.07527 (LSM Bloom survey)
