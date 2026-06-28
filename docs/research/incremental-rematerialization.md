# Incremental KG re-materialization over relational sources

**Date:** 2026-06-27 · **Decision:** ADR-0016. **Evidence:** [A]/[B]/[C].

## Question
Source rows or T change → re-run only the affected partitions → apply a **delta** to the served Oxigraph store (no full reload). The reasoning-half (incremental closure) is ADR-0008; this is the materialization-pipeline half.

## Findings
- **No production R2RML/RML engine does incremental materialization** (Morph-KGC/SDM-RDFizer/RMLStreamer all full re-materialize). **IncRML** (2024) is the closest but **explicitly excludes triplestore delta-ingestion** — exactly the open half. Greenfield, but every building block is mature + Rust-native. [A]
- **CDC:** log-based (PostgreSQL `pgoutput` + **`REPLICA IDENTITY FULL`** for delete/update before-images) ≫ polling/triggers; persist the LSN watermark; monitor slot lag (a stalled consumer bloats WAL). Snapshot-diff (IncRML) is the non-log fallback. [A]
- **Affected partitions:** Morph-KGC partition **disjointness** makes a change provably partition-local; a `table.col → triples-map → partition` index selects what to recompute. [A]
- **Delta triples:** join-free = per-row mapping fn with derivation keys; join-bearing = IVM delta-join `Δ(P⋈C)=(ΔP⋈C)∪(P⋈ΔC)∪(ΔP⋈ΔC)` + **derivation counting** (a multiply-derived triple deletes only at count 0). [A]
- **Apply to Oxigraph:** one **atomic repeatable-read transaction** (`remove`/`insert`) — never the bulk loader (non-atomic, initial-load only); transport as **RDF Patch**. [A]
- **Unified Z-set circuit** (strategic): `dbsp`/Feldera or `differential-dataflow` (both Rust) maintain mapping-IVM **and** the OWL-RL closure as one incremental computation (deletions = negative weights). [A engine / B that it subsumes mapping]
- *Note:* the "~25× faster incremental vs remat" figure circulating for OWL-RL comes from a **retracted** paper — indicative only; the direction is corroborated by RDFox/Motik. [C]

## Decision (ADR-0016)
CDC → affected-partition delta → join-free/join-bearing delta triples (counting) → atomic Oxigraph delta txn → incremental closure (ADR-0008), unioned into the same commit. Strategic path: a unified Z-set circuit. Full-remat fallback (bulk-load + blue/green swap) on T-Box/mapping change, delta ratio above break-even, or CDC gap.

## Sources
- IncRML (SWJ 2024): https://www.semantic-web-journal.net/system/files/swj3674.pdf · Morph-KGC partitions: https://www.semantic-web-journal.net/system/files/swj3135.pdf
- CDC / Postgres logical decoding: https://debezium.io/blog/2018/07/19/advantages-of-log-based-change-data-capture/ · https://www.postgresql.org/docs/current/logical-replication-publication.html · slot WAL bloat: https://www.morling.dev/blog/mastering-postgres-replication-slots/
- IVM counting (SIGMOD 1993): https://dl.acm.org/doi/10.1145/170035.170066 · IVM-for-SPARQL counting (ACM 2025): https://dl.acm.org/doi/abs/10.1145/3796549 · DBToaster (VLDB 2012): https://arxiv.org/abs/1207.0137
- DBSP (arXiv 2203.16684): https://arxiv.org/abs/2203.16684 · `dbsp`/Feldera: https://github.com/feldera/feldera · differential-dataflow: https://crates.io/crates/differential-dataflow
- Oxigraph Store (atomic txn, no bulk-loader for deltas): https://docs.rs/oxigraph/latest/oxigraph/store/struct.Store.html · RDF Patch: https://afs.github.io/rdf-delta/
