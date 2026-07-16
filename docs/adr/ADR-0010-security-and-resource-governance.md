---
status: accepted
date: 2026-06-27
tags: [security, resource-governance, injection-safety, dos, recursive-cte, result-streaming, query-limits, production]
supersedes: []
depends-on:
  - ADR-0006
  - ADR-0007
  - ADR-0008
implements:
  - ADR-0001
---

# Security & resource governance for the SPARQL→SQL path

## Context and Problem Statement

The virtualiser (ADR-0007) is a security boundary: untrusted SPARQL is translated into SQL and executed against a live source database. Three concerns are intrinsic to the rewriter/executor and cannot be retrofitted at a gateway (which never sees the generated SQL): **injection**, **denial of service**, and **result streaming** (a SPARQL `SELECT` may return millions of rows). This ADR fixes the controls the **engine** owns. Authorization (authN/Z, row-level security, multi-tenancy, sensitivity) is **ADR-0018**; deployment-edge operations (TLS, secrets store, rate-limiting, audit transport) are **ADR-0014**.

## Considered Options

* **Engine-owned controls (chosen)** — build injection-safety, DoS governance, and result streaming into the rewriter/executor itself, since these concerns are intrinsic to the SPARQL→SQL path.
* **Retrofit at a deployment gateway/edge** — rejected: a gateway never sees the generated SQL, so injection, denial of service, and result streaming (intrinsic to the rewriter/executor) cannot be addressed there.

## Decision Outcome

### A. Injection-safety by construction
* Values originating from the SPARQL (FILTER constants, VALUES, bound terms) become **bound SQL parameters**, never string-concatenated; SQL is built as a `sqlparser` **AST**, not assembled from strings.
* **The mapping is the reachability allow-list:** generated SQL can reference only the tables/columns the R2RML mapping IR exposes; identifiers come from the *trusted mapping*, never user input — so neither table/column injection nor access to un-mapped data is expressible. *This bounds what is reachable; it is not authorization (ADR-0018).*

### B. Resource governance (DoS controls)
* **Bounded recursion:** every `P+`/`P*` recursive CTE carries a depth limit **and** cycle detection (`CYCLE` on PG14+ / `USING KEY` on a DuckDB source).
* **Statement timeout + result-size cap + pre-execution cost check + admission control** on every generated query — the source DB is never taken down.

### C. Result streaming (bounded memory + backpressure)
* Results stream via `tokio-postgres` `query_raw()` → `RowStream` (never `query()`, which buffers a `Vec<Row>`); `RowStream` already bounds client memory **and** propagates TCP backpressure to the backend. Serialise per-solution with `sparesults`, coalesce ~32 KiB chunks, into an `axum` streaming body (the Oxigraph `ReadForWrite` pattern). `prepare()` the SQL before the `200` (clean `4xx`); on stream drop, **cancel the query and discard the connection** (never recycle a possibly-undrained one).
* **Stream lifetime is bounded at the DB:** `statement_timeout` is per-`FETCH`, not per-cursor, so a slow client would otherwise pin a connection indefinitely — bound total lifetime with PostgreSQL 17 `transaction_timeout` (pre-17: `idle_in_transaction_session_timeout` + an app wall-clock watchdog; the watchdog is mandatory for DuckDB/SQLite sources). Run streams in a small, hard-capped **stream-lane connection pool** distinct from the point-query pool; shed overflow as HTTP `503` + `Retry-After` rather than queue; **never** `WITH HOLD` cursors (they materialise the full result at COMMIT).

### D. Delegated
* **Authorization / RLS / tenancy / sensitivity → ADR-0018.** **TLS, secrets store, rate-limiting, audit transport, deployment packaging → ADR-0014.** The engine consumes DB credentials via secret injection only (never logged; ADR-0011) and emits governance + access-decision events to observability (ADR-0011).

### Consequences
* Good, because neither table/column injection nor access to un-mapped data is expressible (user values are bound parameters; identifiers derive only from the trusted mapping IR).
* Good, because the source DB is never taken down (statement timeout, result-size cap, pre-execution cost check, admission control on every generated query).
* Good, because bounded recursion makes `P+`/`P*` terminate within a depth bound with cycle detection rather than OOM/hang.
* Good, because client memory is bounded and TCP backpressure propagates to the backend (`RowStream`), and slow/abandoned clients cannot pin a connection indefinitely (DB-bounded stream lifetime, stream-lane pool, cancel-on-drop).
* Neutral, because authorization / RLS / tenancy / sensitivity is delegated to ADR-0018 and TLS / secrets store / rate-limiting / audit transport / deployment packaging to ADR-0014.

### Confirmation
* Fuzzing the rewriter (ADR-0012) surfaces no injection (always parameterised; identifiers always from the mapping).
* A `P+` query over a cyclic fixture terminates within the depth bound; a pathological query hits cost-reject or timeout — not OOM/hang.
* A million-row `SELECT` streams with bounded memory; a slow/abandoned client is bounded by `transaction_timeout` and does not exhaust the stream-lane pool.

> **Status correction (2026-07-16, measured, `ADR-0027`).** The "stream-lane
> connection pool" / "shed overflow as `503` + `Retry-After`" clause above
> describes design intent this ADR presented as decided, but it was never
> built: `grep` for `stream_lane`/`Retry-After`/`503` across `sf-serve`'s
> source and tests returns zero matches, and PostgreSQL is served over a
> single `tokio_postgres::Client`, not a pool. Live load testing (`ADR-0027`)
> confirmed the practical consequence: under concurrent overload, requests do
> **not** crash, hang, or corrupt data (the existing per-request `timeout` and
> `max_query_len` both hold correctly under concurrency, verified directly)
> — but there is no fast, honest overload signal either. Concurrent clients
> simply share the one connection's throughput unevenly and each waits out
> its own full timeout before getting a truncated response, worse UX than
> this clause describes, though not unsafe. Treat this clause as **accepted,
> not implemented** until the pool/shedding is actually built or is formally
> descoped — do not read the rest of this ADR's "accepted" status as implying
> this specific piece shipped.

## More Information
* **Rewriter / `P+`:** ADR-0007. **Exec / pooling:** ADR-0006. **Closure backstop:** ADR-0008. **Authorization:** ADR-0018. **Observability / secrets:** ADR-0011. **Fuzzing:** ADR-0012. **Edge ops:** ADR-0014.
* **Research:** `docs/research/` — `virtualization-streaming`, `obda-resource-governance`.

## Rules
* **R1** — user values are bound parameters, never concatenated.
* **R2** — SQL identifiers derive only from the mapping IR (the reachability floor; authorization is ADR-0018).
* **R3** — every recursive CTE carries a depth limit + cycle detection.
* **R4** — every generated query is governed (statement timeout, result cap, cost pre-check, admission control).
* **R5** — results stream with bounded memory **and** DB-bounded lifetime (`transaction_timeout`, stream-lane pool, cancel-on-drop).
