# HANDOVER — ADR-0024 M5 (full cutover + bounded-memory re-proof)

Branch `feat/adr0024-m5-cutover` (stacked on `feat/adr0024-m4-mysql-adapter`).
Worktree `/Users/henrik/source/hm/sf-adr0024-m5`.

## What shipped

M5 completes ADR-0024: the three per-database executors are now one generic core
plus thin per-driver adapters, `sf-serve` drives all backends through a single
async streaming lane, and constant engine memory is re-proved per backend.

1. **Single-home term-gen relocation (pure move).** The Class-X helpers
   (`reconstruct`/`build_term`/`derived_term`/`order_cmp`/`eval_expr`/`instantiate`/
   `Solutions`/`rust_group_result_rows`/`rust_agg`/`RawRow`/…) physically moved from
   `exec.rs` into `exec_core.rs` as the single physical home. Byte-identical bodies;
   `exec::Solutions` kept as a re-export for the `=_bag` harness + PG/MySQL executors.
2. **R2 owned cap-1 SQLite serve-bridge (§4.1).** `SqliteOwnedBackend` +
   `SqliteReceiverStream`: the `!Send` `Connection` lives only on a `spawn_blocking`
   thread behind a cap-1 mpsc; the owned `Receiver` is `Send + 'static`. `blocking_send`
   on cap-1 = explicit backpressure (~2-row materialisation).
3. **sf-serve collapse.** One `select_body_streaming` + one `construct_body_streaming`
   over a type-erased boxed driver; the SQLite `spawn_blocking` execution special-case
   and `ChannelWriter` are gone. (The abstract-`B` future is not provably `Send` — AFIT
   — so the streamer erases the *concrete* driver future rather than spawning
   `run::<B>` generically; each backend supplies a thin `exec_*::*_each_*` closure.)
   The only remaining `spawn_blocking` in `sf-serve/src` is the SPARQL→SQL rewriter
   off-runtime (ADR-0006), unrelated to execution.
4. **MySQL slow-consumer release test (§4.2).** `sf-serve/tests/mysql_release.rs`:
   over a size-1 pool, read one chunk then drop the body; `pool.get_conn()` succeeds
   within a timeout ⇒ the dedicated stream connection is released, not leaked.
5. **Bounded-memory re-proof per backend.** `constant_memory` gains PG (`query_raw`
   cursor) + MySQL (packet-bounded `exec_iter`, NOT cursor-grade) variants: peak
   engine heap stays ≈constant (~142 KB) while triples grow 16× (5200→83000);
   first-result ~630 µs (PG) / ~1.4 ms (MySQL).

## NET-LOC accounting (`git diff --stat 1e178ff` → M5 head, exec*.rs + backend/*.rs)

Baseline @ `1e178ff` (triplicated, `exec_core.rs`/`backend/` ABSENT):

| file | lines |
|---|---|
| `exec.rs` | 1287 |
| `exec_pg.rs` | 543 |
| `exec_mysql.rs` | 329 |
| **triplicated total** | **2159** |

Post-M5 (one generic core + trait + 3 adapters + 3 thin shims):

| file | lines | role |
|---|---|---|
| `exec_core.rs` | 1275 | the single generic core (branches loop, dedup/order/slice, rust_group, all term-gen) |
| `exec.rs` | 159 | SQLite sync shims + serve-lane wrappers |
| `exec_pg.rs` | 108 | thin PG delegators |
| `exec_mysql.rs` | 118 | thin MySQL delegators |
| `backend.rs` | 81 | `SqlBackend`/`BranchStream`/`RawTuple` trait seam |
| `backend/sqlite.rs` | 343 | borrowing + R2 owned adapters (marshalling only) |
| `backend/pg.rs` | 190 | PG adapter (q12 typed-bind, `pg_value`) |
| `backend/mysql.rs` | 135 | MySQL adapter (`mysql_value_to_string`) |
| **total** | **2409** | |

Raw LOC is roughly flat (+250, +12 %), but the **structure** changed decisively: the
~740-line correctness-critical logic (reconstruct / term-gen / `order_cmp` / DISTINCT
dedup / `rust_group` / `eval_expr`) that would have been *triplicated* to bring PG and
MySQL to full parity is now **single-homed once** in `exec_core.rs`; the three adapters
are marshalling-only. MySQL gained q9/q15/ORDER-expression-key parity for free, and the
R2 owned SQLite bridge (new in M5, cap-1 backpressure) accounts for most of the adapter
growth. `git diff --stat feat/adr0024-m4-mysql-adapter`: 12 files, +1852 / −1104.

## Gates (all green; live PG :5432 + MySQL :13306 up)

- `cargo build --workspace`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`
- `differential_pg_sqlite` — SQLite + PG + MySQL arms all run + pass (=_bag byte-identical)
- `constant_memory` — SQLite + PG + MySQL bounded-memory
- `sf-serve` `endpoint` + `mysql_release`
- `e2e::sqlite_owned_bridge_matches_borrowing_select_each` (owned bridge == borrowing, FIFO)

## Untouched (hard constraint)

`emit.rs`, `iq.rs`, `unfold.rs`, `leftjoin.rs`, `path.rs` — unchanged.
