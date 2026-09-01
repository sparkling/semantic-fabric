---
status: accepted
date: 2026-06-27
updated: 2026-09-01
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
* **Exact governed recursion:** every supported `P+`/`P*` recursive CTE collapses cycles on semantic node-pair identity. A work/deadline limit aborts semantic completion; accumulated rows are never labelled or receipted as complete. Once HTTP `200` begins, transport bytes may remain observable, so atomic no-prefix delivery is a separate response-layer gate (ADR-0049).
* **Statement timeout + result-size cap + pre-execution cost check + admission control** on every generated query must bound engine-originated load. These controls reduce overload risk; they cannot guarantee source-database availability.

### C. Result streaming (bounded memory + backpressure)
* Results stream via `tokio-postgres` `query_raw()` → `RowStream` (never `query()`, which buffers a `Vec<Row>`); `RowStream` already bounds client memory **and** propagates TCP backpressure to the backend. Serialise per-solution with `sparesults`, coalesce ~32 KiB chunks, into an `axum` streaming body (the Oxigraph `ReadForWrite` pattern). `prepare()` the SQL before the `200` (clean `4xx`); on stream drop, **cancel the query and discard the connection** (never recycle a possibly-undrained one).
* **Stream lifetime is bounded at the DB:** `statement_timeout` is per-`FETCH`, not per-cursor, so a slow client would otherwise pin a connection indefinitely — bound total lifetime with PostgreSQL 17 `transaction_timeout` (pre-17: `idle_in_transaction_session_timeout` + an app wall-clock watchdog; the watchdog is mandatory for DuckDB/SQLite sources). Run streams in a small, hard-capped **stream-lane connection pool** distinct from the point-query pool; shed overflow as HTTP `503` + `Retry-After` rather than queue; **never** `WITH HOLD` cursors (they materialise the full result at COMMIT).

### D. Delegated
* **Authorization / RLS / tenancy / sensitivity → ADR-0018.** **TLS, secrets store, rate-limiting, audit transport, deployment packaging → ADR-0014.** The engine consumes DB credentials via secret injection only (never logged; ADR-0011) and emits governance + access-decision events to observability (ADR-0011).

### Consequences
* Good, because neither table/column injection nor access to un-mapped data is expressible (user values are bound parameters; identifiers derive only from the trusted mapping IR).
* Good, because statement timeout, result-size cap, cost pre-check and admission control bound engine-originated load when implemented; they are not a source-database availability guarantee.
* Good, because pair-fixed recursion terminates on finite sources without authorizing or receipting a depth-truncated answer; total work must abort semantic completion under ADR-0049.
* Good, because client memory is bounded and TCP backpressure propagates to the backend (`RowStream`), and slow/abandoned clients cannot pin a connection indefinitely (DB-bounded stream lifetime, stream-lane pool, cancel-on-drop).
* Neutral, because authorization / RLS / tenancy / sensitivity is delegated to ADR-0018 and TLS / secrets store / rate-limiting / audit transport / deployment packaging to ADR-0014.

### Confirmation
* Fuzzing the rewriter (ADR-0012) surfaces no injection (always parameterised; identifiers always from the mapping).
* A `P+` query over a cyclic fixture reaches the exact pair fixed point; a pathological query must never report or receipt partial semantic success, while post-`200` transport atomicity remains a separate gate.
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

> **Status correction, part 2 (2026-07-18, built + measured).** The PG half of
> the clause above IS now implemented (M4 wave-2): `Backend::Pg` is a
> `deadpool_postgres::Pool` (`max_size` 16, `wait_timeout` 5s,
> `Runtime::Tokio1` — the runtime must be set explicitly or the timeout is
> silently never enforced), and pool exhaustion sheds `503` + `Retry-After: 1`
> instead of queueing (`acquire_pg`, test-locked incl. the exhaustion path).
> Measured under 16 concurrent SELECTs: ~2.3× wall-clock improvement over the
> single-client behavior (4.10s→1.75s / 3.94s→1.68s, all responses complete
> and correct). SQLite remains a single `Mutex<Connection>` by choice (an
> embedded-source serialization question, out of this clause's scope);
> `Retry-After` is a fixed `1`, not pressure-derived — both recorded as open
> refinements, not gaps in the clause.

> **Status correction, part 3 (2026-08-25, issues #6 and #7).** Four adapter
> families currently substitute values into SQL text with quote doubling
> rather than binding them: REST, MonetDB, HANA, and ODBC. None conforms to R1.
> The REST family additionally collects complete result pages before returning
> a `BranchStream`, so it does not conform to R5; a one-row stream interface
> alone proves neither bounded first-result latency nor bounded memory. The
> supported `sf-serve` surface remains SQLite, PostgreSQL, and MySQL. Any other
> adapter may join that surface only after provider-native parameter transport,
> bounded streaming, lifecycle/error handling, cancellation, and direct
> conformance evidence exist. Issue #6's optional fallible/early-exit quad sink
> is an API refinement and does not change this ADR's accepted status.

> **Status correction, part 4 (2026-09-01, completion audit).** ADR-0049 removes
> the successful hard-coded 256-hop prefix: evidenced compiler targets now use an
> exact finite-pair fixed point and unproved dialects reject before emission.
> R4 remains incomplete: compilation is outside the request timeout and there is
> no common result-row/result-byte cap, cost pre-check, source-native statement
> timeout, or cancellation contract across all three backend paths. No production-
> admission claim follows from pair exactness or the current timeout/pool controls.

> **Status correction, part 5 (2026-09-01, built + measured).** Commit `6cd85eb`
> mints one absolute deadline before request-body extraction and carries that same
> instant through a fixed-capacity compiler admission wait, the blocking compile
> waiter, PostgreSQL acquisition, ASK execution, the complete stream driver,
> serializer finish and body send. A timed-out or abandoned compile keeps its one
> of four permits until the blocking closure really returns, and the executor
> yields once per bounded raw batch even when OFFSET/dedup discards every row
> before the sink. Deterministic tests cover every phase and prove that no phase
> refreshes the clock. This is a narrow elapsed-time boundary, not completed R4/R5:
> compiler CPU is not cooperatively cancellable; queued HTTP waiters, rows, bytes,
> source work and recursion lack one total budget; sources lack a common native
> statement-cancellation contract; SQLite cannot interrupt an in-flight blocking
> bridge; and work inside one raw batch is not pre-empted. After HTTP `200`, a
> SELECT/CONSTRUCT timeout terminates the body and may expose a usable prefix, so
> no atomic/no-success-prefix response claim or backend admission follows.

## More Information
* **Rewriter / `P+`:** ADR-0007. **Exact closure:** ADR-0049. **Exec / pooling:** ADR-0006. **Reasoning:** ADR-0008. **Authorization:** ADR-0018. **Observability / secrets:** ADR-0011. **Fuzzing:** ADR-0012. **Edge ops:** ADR-0014.
* **Research:** `docs/research/` — `virtualization-streaming`, `obda-resource-governance`.

## Rules
* **R1** — user values are bound parameters, never concatenated or textually interpolated, even with escaping.
* **R2** — SQL identifiers derive only from the mapping IR (the reachability floor; authorization is ADR-0018).
* **R3** — every supported recursive CTE collapses semantic cycles; any work or deadline ceiling aborts semantic completion and never labels or receipts a depth prefix as complete. Post-`200` bytes may remain observable until a response-layer atomicity gate is implemented (ADR-0049).
* **R4** — every generated query is governed (statement timeout, result cap, cost pre-check, admission control).
* **R5** — `open_branch` returns without first collecting the complete result, and results stream with bounded memory **and** DB-bounded lifetime (`transaction_timeout`, stream-lane pool, cancel-on-drop).
* **R6** — an adapter that fails R1 or R5 is excluded from `sf-serve`; an admission test locks the supported serving set until the adapter passes those rules.
