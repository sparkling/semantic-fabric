# SOTA — resource governance for a streaming SPARQL/OBDA endpoint over a live source DB

**Research key:** `obda-resource-governance`
**Date:** 2026-06-27 (round-2 deep-research)
**Scope:** governance for the virtualizer (Rust, tokio-postgres + deadpool) — stream results while protecting the source PostgreSQL/DuckDB/SQLite. Central tension: long-lived streaming cursors pin a pooled source-DB connection for the stream's whole life → pool exhaustion / head-of-line blocking.
**Decision recorded in:** ADR-0010 (decision update 2026-06-27).

## Bottom line

Run streaming cursors in a **separate, small, hard-bounded "stream lane" pool**, distinct from the point-query pool, so streams can never starve fast queries. Bound each stream's **total lifetime at the DB** with PostgreSQL 17 **`transaction_timeout`** — because **`statement_timeout` only bounds each individual `FETCH`, not the whole cursor**, so a slowloris client (one fast FETCH every 25 s) holds a connection for hours and never trips it. Shed (don't queue) excess streams via a small `deadpool` `wait` timeout + concurrency cap → HTTP 503. Add an app wall-clock watchdog (mandatory for DuckDB/SQLite, which lack server timeouts), a streaming row-count cutoff, a cost pre-check (EXPLAIN as a coarse gate), recursive-CTE depth + CYCLE bounds, and a circuit breaker keyed on source-DB health. Enforce the same limits again at the DB via a least-privilege read-only role.

## The cursor-vs-pool tension, precisely

- **Streaming pins a connection.** A `RowStream` keeps the connection dedicated until fully consumed or dropped (`RowStream::rows_affected()` returns `None` until exhausted; issue #840 = a follow-up query hanging on an undrained stream). **One in-flight stream == one pinned pool connection.**
- **`statement_timeout` does NOT save you.** A server-side cursor is read with repeated `FETCH`; `statement_timeout` is applied **per `FETCH`, not to the cursor lifetime** (PG-hackers thread; pgjdbc #2873). A slow client never trips it.
- **Naive pooling makes it worse.** deadpool `Timeouts` default to `None` = wait forever; default `max_size = cpu×4`; bb8 `connection_timeout` 30 s. One pool + N streams pins every slot → head-of-line blocking on point queries.

## Governance design (10 layers; 1+2+3 are the resolution)

1. **Two-lane pools** — `point_pool` (fast, larger) + `stream_pool` (small, strict; its `max_size` *is* the global concurrent-stream cap). Invariant: `point.max + stream.max + margin ≤ source max_connections` and ≤ the role's `CONNECTION LIMIT`.
2. **Admission = load-shedding, not queueing** — stream-lane `deadpool` `wait` ≈ 1–3 s → over-capacity fails fast → HTTP 503 + `Retry-After`.
3. **Bound total stream lifetime at the DB** — open the cursor in an explicit `READ ONLY`, `WITHOUT HOLD` transaction; set **`transaction_timeout` (PG17+)** = hard max (e.g. 300 s); it bounds total wall-clock incl. idle gaps (shortest-of-set wins). PG<17: `idle_in_transaction_session_timeout` + the app watchdog. **Never `WITH HOLD`** (materialises the entire result at COMMIT).
4. **App wall-clock + idle-consumer watchdog** — tokio deadline on the whole task; abort if the downstream client stops reading for T s. **Mandatory for DuckDB/SQLite.**
5. **Result-size cap as a streaming cutoff** — `LIMIT min(user, system_max)` pushed into SQL + count emitted rows + abort with a "truncated" trailer; never buffer to count. (Virtuoso `ResultSetMaxRows` rec. 100 000, hard ≤ 2²⁰.)
6. **Cost pre-check** — `EXPLAIN` (no ANALYZE) as a coarse admission gate (reject worst plans before execution). Caveat: planner estimates are unreliable for the wide UNION/many-join SQL OBDA emits → timeouts/size-caps are the real backstop. Theory: Ontop *Cost-Driven OBDA* (ISWC 2017).
7. **Recursive-CTE DoS bounds (P+/P*)** — PostgreSQL has **no** recursion-depth GUC (the "LIMIT trick" is "not recommended"); **always** emit a depth-counter column **and** a `CYCLE` clause (PG14+). DuckDB: explicit cycle detection / `USING KEY` + `memory_limit` (lower from the 80% default) + `max_expression_depth`.
8. **Circuit breaker** — trip on acquire-p95 / cancel-rate / error-rate; shed heavy/stream requests for a cooldown.
9. **Per-role DB caps** — read-only role with `statement_timeout`, `idle_in_transaction_session_timeout`, `transaction_timeout`, `CONNECTION LIMIT`.
10. **Engine-specific** — DuckDB/SQLite are in-process → "protection" is host CPU/mem: app watchdog + `duckdb_interrupt`/SQLite `progress_handler` + `busy_timeout` + cap DuckDB `threads`.

## Knobs (sane defaults)

| Knob | Default | Rationale |
|---|---|---|
| `statement_timeout` | 30 s | bounds each FETCH/point query (NOT the cursor) |
| `transaction_timeout` (PG17) | 300 s | **hard cap on total stream lifetime incl. idle** |
| `idle_in_transaction_session_timeout` | 60 s | kills stalled-client streams (pre-17 primary) |
| `lock_timeout` | 1–3 s | read-only executor never blocks on locks |
| `point_pool.max_size` | 16–32 | fast lane |
| `stream_pool.max_size` | 4–8 | each slot pinned for whole stream → keep small |
| stream-lane `wait` | 1–3 s | fail-fast → shed (503) |
| app `max_stream_duration` | ≈300 s | required for DuckDB/SQLite |
| result-row cap (LIMIT) | 100 000 | Virtuoso precedent |
| recursive depth cap | 25–50 hops | bound path explosion |
| recursive `CYCLE` | always on | cycle detection (PG14+) |

## Production precedent
Virtuoso **Anytime Queries** (`MaxQueryExecutionTime` overrides client; partial-results-on-timeout option; `ResultSetMaxRows`, `MaxQueryCostEstimationTime`, 2²⁰ HTTP cap); Stardog `query.timeout` 5 min + `query.memory.limit` (spill or terminate); GraphDB `query-timeout` + partial-vs-error toggle; QLever default LIMIT 100000; Ontop streams-by-default, delegates timeouts to the RDBMS. **Cross-cutting:** every endpoint pairs a hard server-enforced time bound (overriding client wishes) with a result-size cap + an explicit partial-vs-hard-error policy. Recommend **hard error / truncation trailer** for a write-protected source.

## Co-design with streaming (`virtualization-streaming.md`)
A continuous `RowStream` makes a slow-client stall count against `statement_timeout` (catchable, but can kill a legit long export); a cursor/`FETCH` makes each fetch short → needs `transaction_timeout`. **Choose the streaming mechanism and the timeout knob together.**

## Evidence grades
- statement_timeout per-FETCH; transaction_timeout total — **A** (PG docs, PG-hackers, pgjdbc #2873).
- WITH HOLD materialises at COMMIT — **A** (PG `DECLARE` docs).
- PG has no recursion-depth GUC; CYCLE since PG14 — **A** (PG `queries-with` docs).
- deadpool/bb8 defaults — **A** (docs.rs).
- two-lane pools + load-shedding + circuit breaker — **B** (established patterns + sound inference).

## Sources
- https://www.postgresql.org/docs/current/runtime-config-client.html · https://www.dbi-services.com/blog/postgresql-17-transaction_timeout/ · https://www.postgresql.org/docs/current/sql-declare.html · https://www.cybertec-postgresql.com/en/with-hold-cursors-and-transactions-in-postgresql/ · https://github.com/pgjdbc/pgjdbc/discussions/2873
- https://www.postgresql.org/docs/current/queries-with.html · https://duckdb.org/2025/05/23/using-key · https://duckdb.org/docs/current/operations_manual/limits
- https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.RowStream.html · https://github.com/sfackler/rust-postgres/issues/840 · https://docs.rs/deadpool/latest/deadpool/managed/struct.Timeouts.html
- https://arxiv.org/abs/1707.06974 (Cost-Driven OBDA, ISWC 2017)
- https://vos.openlinksw.com/owiki/wiki/VOS/VirtSPARQLEndpointProtection · https://docs.stardog.com/operating-stardog/database-administration/query-management · https://graphdb.ontotext.com/documentation/11.3/query-monitoring.html · https://docs.qlever.dev/ · https://ontop-vkg.org/guide/cli
