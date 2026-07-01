---
status: accepted
date: 2026-07-01
ratified: 2026-07-01
tags: [execution, drivers, dialect, postgres, sqlite, mysql, scaling, backend-abstraction, streaming, charter, ontop-parity]
supersedes: []
depends-on:
  - ADR-0006
  - ADR-0007
  - ADR-0010
implements: []
---

# Unify per-database execution behind a `Backend` abstraction (dialect SQL + thin driver adapters, not per-driver executors)

## Context and Problem Statement

semantic-fabric emits one SQL query per leaf CQ and pushes the set-work to the source RDBMS (ADR-0006), streaming rows back under a bounded-memory invariant (server-side cursors, ADR-0006/0010). **SQL *generation* is already dialect-parameterised and shared** — `emit.rs` renders `Dialect::{Sqlite,Postgres,Mysql}` from one code path. But the **execution + result-reconstruction layer is hand-written once per driver**:

| module | lines | driver crate | responsibility |
|---|---|---|---|
| `exec.rs` | 1287 | `rusqlite` (SQLite) | prepare, bind params, stream rows, reconstruct RDF terms, `rust_group`, EXISTS, DISTINCT, ORDER, LIMIT |
| `exec_pg.rs` | 543 | `tokio-postgres` | the same, re-implemented for Postgres |
| `exec_mysql.rs` | 329 | `mysql_async` | the same, re-implemented for MySQL |

Each executor independently re-derives parameter binding, cursor streaming, typed value marshalling, and the handling of the harder `Plan` shapes. **This duplication is not a style nit — it is an active correctness hazard, empirically demonstrated (ADR-0023, 2026-07-01).** The live Ontop-vs-sf head-to-head over PostgreSQL exposed five defects and one perf blowup that were **Postgres-path-only** and completely invisible to the (green) SQLite differential:

* **q12 FILTER over an `INT4` column** aborted mid-stream — `exec_pg` bound the constant as a Rust `String`; PostgreSQL infers the placeholder as `INT4`; `String` does not `accepts(INT4)`, so the driver errored *after* the HTTP `200` header was already flushed. SQLite never exercised typed-column binding, so its differential stayed green.
* **q9 aggregate-over-UNION** (`rust_group`) aborted — `exec_pg` hard-returned `Unsupported` for the multi-branch aggregation path that `exec.rs` implements. The ADR-0023 tree closed this bug *in-process over SQLite*; the PostgreSQL executor had never been taught it.
* MINUS, sequence paths, and DISTINCT-over-join were likewise wrong or empty only on the PG path.

The root pattern: **the in-process SQLite differential green-lit an engine that was broken on the production PostgreSQL path**, because "the executor" is really *N* executors and the test corpus only covered one. Today `N = 3` (SQLite for CI, PostgreSQL primary, MySQL following — ADR-0006). The charter direction is more sources over time; Ontop, the parity oracle, ships ~15–20 dialects. The question this ADR settles: **what is the per-database cost of a correct executor, and does the current shape scale to `N ≈ 20`?**

As structured, adding a database costs a **full new executor** — the typed-bind bug class, the streaming logic, and every hard `Plan` shape (`rust_group`, EXISTS, DISTINCT, ORDER/LIMIT push, term reconstruction) re-implemented and re-tested. The maintenance and bug surface is **O(databases × query-features)**, and *trustworthiness* requires an **O(databases)** differential matrix. At `N ≈ 20` this is untenable: the q12 typed-bind bug is a preview of a class you would pay ~17 more times.

**How the oracle avoids this.** Ontop confines per-DBMS variation to two thin layers: (1) a per-DBMS **SQL code generator** handling only *syntax* variation points (string concatenation, IRI casting, regex operators, date arithmetic, UUID, NULL-in-aggregates, BOOLEAN representation — `docs/research/ontop.md`), and (2) **one uniform execution + result-binding path via JDBC** for all databases, where the driver layer marshals typed values uniformly. Adding a dialect is small enough that Ontop v5 shipped a scaffolding tool for it. semantic-fabric already has layer (1) in `emit.rs`; it is **layer (2) that is duplicated**.

## Decision Drivers

* **Bounded-memory streaming is non-negotiable (ADR-0006/0010).** Whatever unifies execution must preserve the server-side-cursor, no-instance-buffering invariant on every backend. This is *why* ADR-0006 chose native drivers over a columnar intermediary; that choice stands and is not reopened here.
* **Correctness parity across backends must be structural, not per-port heroics.** A feature implemented once and executed uniformly cannot silently diverge on one backend (the q9/q12 failure mode).
* **Per-database cost must be sub-linear in features.** Adding a database should cost a dialect entry + a thin adapter, not a re-implementation of the executor.
* **Charter (ADR-0004/0006/0007).** Own the engine in Rust, no JVM; `=_bag` absolute; term-construction lifted; push-down to the source. Any abstraction preserves all of these.
* **Rust has no JDBC.** The per-driver crates (`rusqlite`, `tokio-postgres`, `mysql_async`) have distinct APIs, type systems, and binding models — so uniformity must be *built*, either as a trait we own or via a crate (`sqlx`) that already spans backends.

## Considered Options

* **A. Status quo — one hand-written executor per driver.** Rejected. O(databases × features) surface; the q9/q12 bug class recurs per backend; requires an O(databases) differential matrix to be trustworthy; scales poorly toward `N ≈ 20`.
* **B. Adopt `sqlx` as the uniform execution layer.** `sqlx` is async and spans PostgreSQL/MySQL/SQLite/MSSQL behind one API with typed `Encode`/`Decode` (which *dissolves the q12 typed-bind bug class outright*) and a uniform `Row`. Attractive, but carries a real risk against Driver #1: `sqlx`'s streaming/cursor semantics differ per backend, and the **bounded-memory server-side-cursor invariant (ADR-0006/0010) is unproven under `sqlx`** and must be validated per backend before adoption. Also a substantial new dependency on the hot path.
* **C. A native `Backend` trait we own.** Define a small trait — `connect`, `prepare`, `bind_lexical_typed_param(value, inferred_type)`, `stream_rows` (server-side cursor) — implemented once per driver as a **thin adapter** (~100 lines), and write the executor + RDF-term reconstruction + hard-`Plan`-shape logic **once, generic over the trait**. Preserves full control of the streaming invariant; no new hot-path dependency; the typed-bind bug becomes a single well-tested trait method. More upfront refactoring of `exec.rs` into a driver-agnostic core.
* **D. Columnar/OLAP intermediary (DataFusion/DuckDB) to normalise execution.** Rejected — already rejected by ADR-0006: an in-process columnar engine buffers instance data and breaks bounded memory.

## Decision Outcome

**Chosen: Option C — unify execution behind a native `Backend` abstraction, with Option B (`sqlx`) held as a fallback pending a streaming spike.**

Introduce a `SqlBackend` trait (in `sf-sql`, alongside the existing dialect layer) with the minimal surface an executor needs: connect/pool, prepare, **typed lexical parameter binding** (parse the lexical constant to the native type using the driver-inferred placeholder type — the q12 fix, generalised and written once), and **server-side-cursor row streaming**. Refactor the ~2159 lines of `exec*.rs` into:

* **one driver-agnostic executor + RDF-term reconstruction core** (the `rust_group` / EXISTS / DISTINCT / ORDER / LIMIT / term-generation logic, written once), generic over `SqlBackend`; and
* **thin per-driver adapters** (`rusqlite`, `tokio-postgres`, `mysql_async`) implementing only connect + prepare + bind + stream.

Per-database variation is thereby confined to exactly two thin, declarative places: **(1) dialect SQL generation** (`emit.rs`, already there) and **(2) the `SqlBackend` adapter**. Adding database #4…#N is a dialect table entry plus a ~100-line adapter — **O(databases) thin adapters, not O(databases × features) executors** — matching Ontop's cost profile while staying Rust-native and JVM-free.

`sqlx` (Option B) is not adopted now because ADR-0006's bounded-memory server-side-cursor invariant is unproven under it. It is recorded as the fallback: **if a focused streaming spike proves `sqlx` preserves bounded first-result latency and constant memory under growing source data on PostgreSQL and MySQL**, adopting `sqlx` as the `SqlBackend` implementation (rather than hand-rolled adapters) is a sound simplification and eliminates the typed-bind class for free. The trait boundary makes this swap local either way.

**This ADR does not reverse ADR-0006** (native drivers, push-down, no columnar intermediary all stand); it refines ADR-0006's *execution-layer structure* from "one executor per driver" to "one executor over a `Backend` trait."

### Consequences

* Good: the q9/q12 class of per-backend bug is written and tested **once**; a feature added to the executor is uniform across all backends by construction.
* Good: per-database cost drops from a full executor to a dialect entry + thin adapter; scaling toward Ontop's ~20 dialects becomes tractable.
* Good: the differential coverage collapses — one executor core over a test double / one backend proves the logic; per-backend differential shrinks to *adapter* conformance (connect/bind/stream), not full-feature re-testing (though a live per-backend smoke stays valuable — see Confirmation).
* Bad/cost: a significant, correctness-sensitive refactor of the engine's hot path (`exec*.rs` → generic core + adapters); the streaming invariant must be re-proven through the new boundary on each backend; the refactor is `=_bag`- and bounded-memory-gated, so it is careful work, not a sweep.
* Neutral: `emit.rs` dialect generation is unchanged; the `Plan`/`Branch` lowering (ADR-0023) is unchanged — this is purely below the SQL string.

### Confirmation

* The unified executor passes the existing `=_bag` differential (PG↔SQLite) + W3C RDB2RDF floor + the live Ontop head-to-head (all 15 feature classes row-parity on PostgreSQL *and* SQLite *and* MySQL) — i.e. the ADR-0023 fixes hold through the new boundary, and a **hard typed-column bind regression test** (the q12 guard) runs on the live PG path.
* The bounded-memory invariant (ADR-0006/0010) is re-confirmed on each backend: server-side cursor, constant engine memory + bounded first-result latency under growing source data (`sf-bench` `constant_memory`), for both the hand-rolled adapters and — if pursued — the `sqlx` spike.
* Adding a new backend is demonstrated to require only a dialect entry + a `SqlBackend` adapter, with no change to the executor core, and its correctness follows from the shared differential rather than a new per-backend feature port.

## More Information

* **Evidence (2026-07-01):** the live Ontop 5.5.0 vs semantic-fabric head-to-head (`BENCHMARKS.md`, `scripts/compare/race.sh`) — five Postgres-path-only correctness defects (q9 agg-over-union, q10 sequence path, q11 MINUS, q12 FILTER-EXISTS/typed-column, q15 DISTINCT-over-join) + one perf blowup (q14), all invisible to the green SQLite differential; fixed in `exec_pg.rs`/`unfold.rs`/`iq.rs`/`leftjoin.rs` and re-verified at row-parity on the live PG endpoint.
* **Oracle model:** `docs/research/ontop.md` §SQL generation / §dialects — Ontop's per-DBMS code generator + uniform JDBC execution; the enumerated dialect variation points; the v5 dialect scaffolding tool.
* **Cross-refs:** ADR-0006 (execution & performance model / native drivers / bounded memory — this refines its execution-layer structure), ADR-0010 (streaming governance / server-side cursors), ADR-0007 (`=_bag` / term-construction lifting — preserved), ADR-0023 (the operator-tree IR whose live-PG validation exposed the per-executor divergence).
* **Candidate crate:** `sqlx` (PostgreSQL/MySQL/SQLite/MSSQL, typed `Encode`/`Decode`, streaming `fetch`) — the fallback `SqlBackend` implementation, gated on the bounded-memory streaming spike.
