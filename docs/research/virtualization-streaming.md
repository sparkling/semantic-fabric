# SOTA — streaming large PostgreSQL result sets to HTTP in async Rust

**Research key:** `virtualization-streaming`
**Date:** 2026-06-27 (round-2 deep-research)
**Scope:** the virtualizer read path — SPARQL→SQL → live Postgres via `tokio-postgres`/`deadpool` → row→binding map → `sparesults` → axum HTTP body. Target: bounded memory, low TTFB, genuine pull-based backpressure, result sets in the millions of rows.
**Decision recorded in:** ADR-0010 (decision update 2026-06-27), refining ADR-0007 stage 6.

## Bottom line

1. **Use `Client::query_raw()` → `RowStream`; never `query()`** (which collects `Vec<Row>` → OOM). `RowStream` **already bounds client memory and already propagates TCP backpressure to the Postgres server** — you do **not** need a cursor/portal merely for bounded memory.
2. **Wrap the pipeline in one `async-stream::try_stream!` generator that owns the pooled connection**, builds the `RowStream` inside it (sidesteps the self-referential borrow), maps row → `QuerySolution`, serializes per-solution with `sparesults`, **coalesces ~16–64 KiB chunks**, into `axum::body::Body::from_stream`.
3. **`prepare()` the SQL before returning `200`** — parse/plan/permission errors then surface as a clean `4xx`; only rare runtime errors truncate the stream after headers.
4. **Escalate to a portal (`bind` + `query_portal_raw(&p, max_rows)`) or `DECLARE CURSOR`/`FETCH` only for server-side row-count checkpoints** — chiefly to interplay with `statement_timeout`, or for non-PG backends (DuckDB/SQLite). `max_rows` does **not** buy client-memory boundedness (you already have it).
5. **Pool size = max concurrent streams** (a stream pins its connection for its life). `CancelToken` on drop; **discard, don't recycle** a possibly-undrained connection.

## The decisive mechanism (verified from tokio-postgres source, v0.7.18)

- Request channel (Client→Connection) is **unbounded**; the per-request response channel (Connection→`RowStream`) is **bounded, `mpsc::channel(1)`**.
- When the consumer stops polling, that capacity-1 channel fills → the Connection **stops reading the socket** (`connection.rs`: pushes to `pending_responses` and `return Ok(None)`) → kernel RX window closes → the Postgres backend blocks on its socket write (`ClientWrite`). **Genuine pull-based, end-to-end backpressure; total in-flight client buffering is bounded independent of result-set size.**
- (A wrong answer was initially produced from a model-summarized GitHub *rendered* page claiming "no backpressure"; the **raw source** contradicts it — graded against the primary artifact. Re-grep against the pinned tag at build time.)

## Wire-protocol semantics

- Simple query / `Execute max_rows=0`: backend sends the **entire** result (TCP-throttled only). `Execute max_rows=N`: returns N rows then `PortalSuspended` — the only protocol way to make the server stop at a row boundary. A **portal** is the protocol-level cursor (`Bind`), txn-scoped; `DECLARE CURSOR`/`FETCH` is its SQL equivalent.
- **libpq contrast:** default `PQexec` buffers the whole result; `PQsetSingleRowMode`/`PQsetChunkedRowsMode` change **client-side processing only — "the server still sends all rows."** To *limit what the server produces* you need a cursor/portal with FETCH. So `RowStream` ≈ libpq single-row mode **plus** the TCP backpressure single-row mode lacks.
- Server memory: a streaming plan does **not** materialise the full result server-side (it blocks on `ClientWrite`). Caveat: buffering plan nodes (`Sort` without LIMIT, `Materialize`, hash builds, CTE materialisation) still spool to `work_mem`/disk regardless of fetch strategy — backpressure throttles transmission, not those operators.

## `RowStream` vs `max_rows` / cursor

| | Bounds client mem | Server row-count checkpoints | TTFB | Round-trips | Explicit txn |
|---|---|---|---|---|---|
| `query()` | ❌ Vec<Row> | ❌ | high | 1 | no |
| `query_raw()`→`RowStream` (`max_rows=0`) | ✅ chan(1)+TCP | ❌ (one continuous Execute) | **lowest** | 1 | no |
| portal `query_portal_raw(&p,N)` loop | ✅ | ✅ every N | low | 1/batch | **yes** |
| `DECLARE CURSOR`+`FETCH FORWARD N` | ✅ | ✅ | low | 1/batch | **yes** |

`max_rows`/cursor value: (a) small server-side window; (b) **plays well with `statement_timeout`** — a continuous `query_raw` counts slow-client stall against the timeout (killable, but may also kill a legit long export), whereas each `FETCH` is short so the slow-client wait happens *between* statements (guard with `idle_in_transaction_session_timeout` / `transaction_timeout`; see `obda-resource-governance.md`); (c) tunable framing batch.

## deadpool & pipeline

- A `RowStream`/portal **pins one pooled connection for the whole stream**; portals additionally need an explicit `Transaction` on that connection. So **max concurrent streams ≤ pool size.** Own the `deadpool::Object` inside the `async-stream` generator (the `Object→Txn→Portal→RowStream` chain is self-referential — SeaORM #2350 documents the same problem; the generator owning the chain is the canonical fix).
- Recycling: prefer `Verified` over default `Fast` for a streaming endpoint, or pair with explicit cancel-and-discard. **Cancellation:** dropping the stream does **not** send `CancelRequest` — capture `Client::cancel_token()` and call `cancel_query()` on drop, or discard the connection.
- **Pull chain (every link is `poll_next`):** client TCP/HTTP-2 window → hyper polls body only when writable → axum `Body::from_stream` → `try_stream!` → `sparesults` serialize → row→binding → `RowStream.poll_next` → tokio-postgres chan(1) → Connection stops socket read → server `ClientWrite`. The only ways to break it: collecting into a `Vec`, or inserting an **unbounded** channel/spawn.

## Prior art
- **Oxigraph HTTP server** — `ReadForWrite` pull state-machine over the lazy `QueryResults::Solutions` iterator + `sparesults` (the direct blueprint).
- **rdf-fusion** — DataFusion `SendableRecordBatchStream` (columnar batch streaming).
- **Ontop** — streams by default (`--no-streaming` "not recommended"); JDBC `fetchSize` + cursor.
- **QLever** — chunked transfer, lazy export, default cap 100000.

## Evidence grades
- query() buffers; query_raw→RowStream incremental — **High** (docs.rs).
- chan(1) backpressure mechanism — **High** (raw `connection.rs`/`client.rs`).
- libpq single-row is client-side only; cursor/portal needed to limit server — **High** (PG docs).
- Oxigraph ReadForWrite pattern — **High** (raw `cli/src/main.rs`).
- Cancel/drain-on-drop detail — **A** for CancelToken / **B-C** for drain (verify empirically against the pinned version — a unit test, not a gate).

## Sources
- https://docs.rs/tokio-postgres/latest/tokio_postgres/struct.Client.html · .../struct.RowStream.html · .../struct.Transaction.html · .../struct.CancelToken.html
- https://raw.githubusercontent.com/sfackler/rust-postgres/master/tokio-postgres/src/connection.rs · .../client.rs · issues #840, #917
- https://www.postgresql.org/docs/current/protocol-flow.html · .../libpq-single-row-mode.html
- https://docs.rs/sparesults/latest/sparesults/ · https://raw.githubusercontent.com/oxigraph/oxigraph/main/cli/src/main.rs
- https://docs.rs/axum/latest/axum/body/struct.Body.html · https://blog.cloudflare.com/hyper-bug/ · https://docs.rs/async-stream/ · https://github.com/SeaQL/sea-orm/discussions/2350
- https://docs.rs/deadpool-postgres/latest/deadpool_postgres/enum.RecyclingMethod.html · https://ontop-vkg.org/guide/cli · https://github.com/ad-freiburg/qlever
