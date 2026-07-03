# ADR-0024 — sqlx Streaming Spike (decision)

**Status:** spike outcome / recommendation for future work. Discharges the one
open gate in [`ADR-0024-parity-report.md` §8](./ADR-0024-parity-report.md):
*"the `sqlx` fallback impl, gated on a streaming spike."*
**Branch:** `spike/sqlx-streaming` (throwaway harness at `scratch/sqlx-spike/`; NOT
wired into `exec_core.rs` or any production path).
**Report date:** 2026-07-03.

> **Evidence discipline.** Every number below is measured on this machine by the
> committed harness (`scratch/sqlx-spike/`, sqlx 0.8.6, rustc 1.96.0) against a live
> local PostgreSQL 17 (`localhost:5432`) and a live local MySQL 8.0.46
> (`localhost:13306`, container `sf-mysql-test`). The full CSV is checked in
> (`results.csv`, plus a `results_pass2.csv` confirmation run). Mechanism claims are
> grounded in the sqlx 0.8.6 source, cited by file:line. Reproduce with
> [§7](#7-reproduce).

---

## 1. The question (and the decision)

ADR-0024 unified three hand-written executors behind a `SqlBackend` trait, but never
adopted `sqlx`; §8 named a `sqlx` backend as future work **gated on proving it can
stream**. The core invariant (ADR-0006 / ADR-0010): **bounded engine memory + bounded
time-to-first-row regardless of result size** — never buffer the whole resultset
client-side.

Concretely: does sqlx's streaming `.fetch()` (returns a `Stream`) truly pull row-by-row
off the wire, or does it secretly buffer like `.fetch_all()`? Answered **independently
for PostgreSQL and MySQL** — MySQL being the interesting case, since its wire protocol
has no server-side cursor.

**DECISION: adopt-later is sound — for BOTH engines.** sqlx `.fetch()` delivers
constant engine memory (~7.5 MB flat from 10 → 2,000,000 rows) and constant, sub-ms
time-to-first-row on **both** PostgreSQL **and** MySQL. Neither engine buffers. Adopting
`sqlx` as a `SqlBackend` impl is a viable simplification to pursue later; it would **not
regress** the MySQL guarantee (sqlx-mysql is mechanically equivalent to today's
`mysql_async` adapter — both socket-incremental, both bounded) and is a **like-for-like**
swap on PG (both use the extended-query protocol with incremental `DataRow` reads).

This is a recommendation for future work, **not** a migration of `exec_pg.rs` /
`exec_mysql.rs` in this session, and its scope is *only* the streaming invariant — see
the limits in [§5](#5-scope--what-this-spike-did-not-evaluate).

---

## 2. Method / measurement rig

- **Synthetic table** `sqlx_spike_bench (id BIGINT, payload TEXT)`, `payload` = 200 bytes,
  populated once to **2,000,000 rows** (≈ 416 MB of payload) on each engine.
- **Query** `SELECT id, payload FROM sqlx_spike_bench LIMIT {n}` — **no `ORDER BY`**, so
  the server streams from the first page and does *not* buffer/sort before row 1 (an
  `ORDER BY` would measure a server-side sort, not client streaming).
- **Two modes per row-count**, so the rig is self-calibrating:
  - `stream` — sqlx `.fetch()` (a `Stream`); each row is marshalled to a `String` and
    dropped (never retained).
  - `buffer` — sqlx `.fetch_all()` (a `Vec<Row>`); the **control**. If the rig can *see*
    linear growth here, then flat growth under `stream` is meaningful.
- **RSS** = whole-process resident set, sampled every 20 ms by a background thread reading
  `ps -o rss=` (captures driver buffers too, not just Rust heap — this is "engine memory").
  Peak is reset immediately before the consume loop.
- **One measurement per process invocation** → a clean RSS baseline every time (no
  allocator high-water-mark contamination between cases).
- **Time-to-first-row (TTFR)** = from just-before the query is issued to the first row
  yielded by the stream. For a true stream this is ~one round-trip regardless of `n`; if
  the driver materialised the whole set first, TTFR would scale with `n` (≈ total time).
- **Scales**: 10, 100 000, 1 000 000, 2 000 000. Confirmed over two independent passes.

---

## 3. Results (pass 1; pass 2 reproduced within noise)

### PostgreSQL 17 (`localhost:5432`)

| rows | stream TTFR (ms) | **stream peak RSS (MB)** | stream total (ms) | **buffer peak RSS (MB)** | buffer total (ms) |
|---:|---:|---:|---:|---:|---:|
| 10 | 1.83 | **7.4** | 3.6 | 7.4 | 0.6 |
| 100 000 | 0.65 | **7.5** | 32.3 | 7.4 † | 24.9 |
| 1 000 000 | 0.24 | **7.5** | 206.3 | **412.7** | 245.5 |
| 2 000 000 | 0.25 | **7.5** | 405.4 | **806.4** | 456.7 |

### MySQL 8.0.46 (`localhost:13306`)

| rows | stream TTFR (ms) | **stream peak RSS (MB)** | stream total (ms) | **buffer peak RSS (MB)** | buffer total (ms) |
|---:|---:|---:|---:|---:|---:|
| 10 | 0.79 | **7.5** | 2.6 | 7.6 | 0.7 |
| 100 000 | 0.86 | **7.7** | 43.3 | 32.5 | 38.1 |
| 1 000 000 | 0.38 | **7.7** | 406.2 | **415.6** | 312.1 |
| 2 000 000 | 0.59 | **7.7** | 593.9 | **845.1** | 750.0 |

### Reading the numbers

- **Bounded memory — PASS, both engines.** `stream` peak RSS is **flat at ~7.5 MB
  (PG) / ~7.7 MB (MySQL) across a 200 000× row range** (10 → 2 000 000). The `buffer`
  control grows **linearly** — PG 412.7 → 806.4 MB and MySQL 415.6 → 845.1 MB as rows
  double 1M → 2M (≈ 2.0×, ≈ 0.4 KB/row: 200 B payload + raw wire bytes + `Row`
  overhead). At 2M rows the stream-vs-buffer separation is **~107× (PG)** and **~110×
  (MySQL)**. A linear stream curve would have been a FAIL; it is dead flat.
- **Bounded first-row latency — PASS, both engines.** `stream` TTFR is **sub-ms and does
  not grow with `n`** (PG 0.24 ms and MySQL 0.59 ms at 2M rows — no higher than at 10
  rows; the 1.8 ms at PG/10 is the cold prepared-statement round-trip). Had `.fetch()`
  buffered before yielding, TTFR at 2M would be ≈ the total (≈ 400–600 ms). It is not.
- **† PG `buffer` at 100 000 rows reads 7.4 MB** — a *sampler-resolution artifact*, not
  streaming: `fetch_all` finished in ≈ 24 ms, near the 20 ms sample cadence, so the ~20 MB
  transient was missed (MySQL, slightly slower, caught 32.5 MB). It reproduces identically
  in pass 2 and is immaterial given the unambiguous 1M/2M buffer points. The stream
  measurements are long enough (200–600 ms) to be sampled hundreds of times.

Pass 2 (`results_pass2.csv`) reproduced every headline cell within noise: PG stream peak
7.4–7.5 MB / buffer 413.6 → 808.7 MB; MySQL stream peak 7.6–7.7 MB / buffer 418.7 → 850.5
MB; all stream TTFR sub-ms and flat in `n`.

---

## 4. Mechanism (source-grounded, not just black-box)

The measurements *are* the proof; the source confirms *why*, and pins down the MySQL
nuance the ADR cares about.

- **PostgreSQL** — `sqlx-postgres` `fetch_many` runs the **extended-query protocol**
  (Parse/Bind/Execute) with **`Execute { limit: 0 }`** = "no row limit", then decodes and
  **`yield!`s each `DataRow` message one at a time** as it arrives off the socket, with no
  client-side `Vec` accumulation (`sqlx-postgres-0.8.6/src/connection/executor.rs:256`,
  `:346–357`; `.../message/execute.rs:13-14`). TCP backpressure throttles the server when
  the consumer is slow. **This is the same mechanism as today's adapter**, which uses
  `tokio_postgres::query_raw` (extended-protocol portal, incremental rows —
  `crates/sf-sql/src/backend/pg.rs:168-169`). Neither is a literal `DECLARE CURSOR`/`FETCH`
  chunked cursor; both are incremental-`DataRow` reads. So sqlx PG is a **like-for-like**
  swap.
- **MySQL** — `sqlx-mysql` `fetch_many` is a `try_stream!` that pins `self.run(...)` and
  **yields each row as `s.try_next()` produces it** — no buffering
  (`sqlx-mysql-0.8.6/src/connection/executor.rs:254–283`). Crucially, **`COM_STMT_FETCH`
  (the MySQL server-side-cursor command) is never sent** — it appears only in
  status-flag *comments* (`.../protocol/response/status.rs:20,23`). So sqlx-mysql reads the
  server-pushed resultset **incrementally off the socket** (client memory bounded, TCP
  backpressure), which is **exactly** what the current `mysql_async`-based adapter does
  (`crates/sf-sql/src/backend/mysql.rs` — `exec_iter`, "packet-bounded, not cursor-grade").

**Honest nuance on MySQL.** sqlx proves bounded *client* memory + bounded TTFR — but it
does **not** upgrade MySQL to a true server-side portal/cursor. MySQL's protocol has no
such thing (unless one issues `COM_STMT_FETCH` against a cursor-typed prepared statement,
which sqlx does not), so ADR-0024 §6's characterisation — *"packet/socket-bounded, not
cursor-grade"* — **remains true under sqlx**. The point is that this was never a driver
deficiency to fix: it is a MySQL-protocol fact. sqlx is **at parity** with today's MySQL
adapter on the streaming axis (equivalent mechanism, empirically bounded), so the failure
mode this spike was hunting — "sqlx-mysql secretly buffers / is no better than what we
have" — is **disproven**.

---

## 5. Scope — what this spike did NOT evaluate

The gate in §8 was specifically the **streaming invariant**; that gate is now cleared for
both engines. A full `sqlx` adoption still needs a separate marshalling-parity pass — out
of scope here and deliberately not claimed:

- **Per-cell lexical fidelity.** The hand-rolled adapters encode hard-won correctness the
  differential locks in: non-UTF-8 `VARBINARY`/`BLOB` → **UNBOUND** (never a lossy
  string), the `DATE`-midnight vs `DATETIME` disambiguation, `bytea` uppercase-hex, PG
  `bool` → `true`/`false` not `t`/`f`, and the `Error::Unsupported` → documented-501 path
  (`backend/mysql.rs:104-135`, `backend/pg.rs:123-146`). A sqlx backend must reproduce all
  of these bag-for-bag against the `=_bag` differential before it could replace them.
- **The GAT streaming seam.** `SqlBackend::Stream<'s>` (a GAT) exists because the MySQL
  cursor borrows `&mut Conn`; mapping sqlx's `BoxStream` onto the owned-vs-borrowing
  two-lane design (and the `for<'s> Stream<'s>: Send` spawn bound) is real integration work.
- **21-dialect breadth**, connection pooling, and cancellation-on-drop (ADR-0010) semantics.

---

## 6. Secondary benefit (not the main evidence): typed-bind bug-class collapse

A `SqlBackend`-over-sqlx would also retire a documented bug class the hand-rolled adapters
are structurally prone to: **each adapter re-derives, differently, how to bind a lexical
SPARQL value to the driver's native parameter type.**

- PG needs a bespoke `LexicalParam: ToSql` wrapper that parses the lexical string into the
  placeholder's *inferred* type at serialise time (`backend/pg.rs:84-114`). Its own doc
  records the failure it fixes — the **q12 regression**: binding a Rust `String` to an
  `INT4` placeholder fails *client-side* and aborts an already-`200` response mid-stream.
- MySQL solves the *same* problem *differently* — it binds **everything as a string**
  (`Value::from(s.as_str())`, `backend/mysql.rs:75-78`) and leans on MySQL's implicit
  coercion.

Two divergent, hand-maintained solutions to one problem. sqlx's uniform `Encode`/`Type`
system collapses this into one typed path shared across drivers. Noted as a bonus — the
*decision* rests on the streaming evidence in §3–§4, not this.

---

## 7. Reproduce

```bash
# From this worktree. The spike is its own [workspace] root, so the parent
# `cargo build --workspace` never sees it.
cd scratch/sqlx-spike
./run.sh          # builds --release, populates 2M rows on PG+MySQL, runs the matrix

# or manually:
cargo build --release
BIN=./target/release/sqlx-streaming-spike
$BIN setup pg          # SPIKE_PG_URL     overrides (default: postgres://henrik@localhost:5432/gtfs_bench)
$BIN setup mysql       # SPIKE_MYSQL_URL  overrides (default: mysql://root:sftest@localhost:13306/sftest)
$BIN measure pg    2000000 stream   # -> CSV line
$BIN measure pg    2000000 buffer
$BIN measure mysql 2000000 stream
$BIN measure mysql 2000000 buffer
$BIN teardown          # drops sqlx_spike_bench on both engines
```

**Environment at report time:** PostgreSQL 17 (Homebrew, `localhost:5432`, user `henrik`);
MySQL 8.0.46 (OrbStack container `sf-mysql-test`, `localhost:13306`, `root`/`sftest`);
sqlx 0.8.6 (`runtime-tokio`, `postgres`, `mysql`, no TLS — plaintext local); rustc/cargo
1.96.0. No local-MySQL limitation applied — both engines were measured live.
