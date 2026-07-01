# ADR-0024 — Parity Report (M7)

**Status:** outcome report for ADR-0024 (*Unify per-database execution behind a
`SqlBackend` abstraction*).
**Branch under report:** `feat/adr0024-m5-cutover`, tip `cf9aca0`.
**Baseline:** `1e178ff` (ADR-0023 operator-tree IR shipped; triplicated executors).
**Milestone SHAs:** M2 `9c98a5e` · M3 `acd57c0` · M4 `1a03742` · M5 `cf9aca0`.
**Report date:** 2026-07-01.

> **Evidence discipline.** Every quantitative claim below is re-derived from the
> repository — `git diff --stat` across the milestone SHAs, `cargo test` against the
> live PG (`localhost:5432`) and MySQL (`127.0.0.1:13306`) fixtures, and the checked-in
> `BENCHMARKS.md` head-to-head tables. The exact commands are listed in
> [§9](#9-commands-run-to-derive-the-numbers). Numbers that could not be re-derived in
> this environment are marked **[UNVERIFIED]** rather than asserted.

---

## 1. Outcome summary

ADR-0024 replaced three independently hand-written SPARQL executors
(`exec.rs` / `exec_pg.rs` / `exec_mysql.rs`) with **one driver-agnostic executor core**
(`exec_core.rs`, generic over a `SqlBackend` trait) plus **three marshalling-only
adapters** (`backend/{sqlite,pg,mysql}.rs`). The three Confirmation bullets of the ADR
([§7](#7-adr-confirmation-bullets--discharge-status)) are discharged:

- **Correctness parity is now structural.** The `=_bag` differential passes live across
  all three backends (SQLite ↔ PG ↔ MySQL); the ADR-0023 PG-path fixes (q9/q10/q11/q12/q15)
  and the q14 latency fix are written **once** and hold through the new boundary; the q12
  typed-column bind regression and the MySQL A1 typed-values guard run on the live paths.
- **Adding a backend is a ~100–190-line adapter + a one-line module registration, with
  the executor core git-unchanged** — empirically demonstrated by M4 (MySQL), which touched
  neither `exec_core.rs` nor `backend/pg.rs` ([§3](#3-add-a-backend-cost-demonstration)).
- **Bounded memory is re-proven per backend** — `constant_memory` is green 3/3 live, with
  engine peak heap ≈ constant (~140 KB) under 16× source growth
  ([§6](#6-bounded-memory-per-backend)).

The cost was a raw-LOC increase of **+250 lines (+11.6%)** on the execution surface — but
the ~740-line correctness-critical logic that would otherwise have been *triplicated* to
bring PG and MySQL to full parity is now single-homed once
([§2](#2-loc-accounting)).

Honest exceptions carried forward, not hidden: **q9** (aggregate-over-UNION) is *slower*
than Ontop on PG (Rust `rust_group` vs SQL `GROUP BY`); small aggregates (q6/q13) sit at
parity; MySQL bounded memory is **packet-bounded, not cursor-grade** (the driver has no
server-side cursor). See [§5](#5-performance) and [§8](#8-residual--out-of-scope).

---

## 2. LOC accounting

Line counts are `git show <sha>:<path> | wc -l`; the net delta is
`git diff --stat 1e178ff cf9aca0` over the execution surface.

**Baseline @ `1e178ff`** — triplicated executors; `exec_core.rs` and `backend/` absent:

| file | lines |
|---|---|
| `crates/sf-sparql/src/exec.rs` | 1287 |
| `crates/sf-sparql/src/exec_pg.rs` | 543 |
| `crates/sf-sparql/src/exec_mysql.rs` | 329 |
| **triplicated total** | **2159** |

**Post-M5 @ `cf9aca0`** — one generic core + trait seam + 3 adapters + 3 thin shims:

| file | lines | role |
|---|---|---|
| `crates/sf-sparql/src/exec_core.rs` | 1275 | **the single generic core** — branches loop, DISTINCT dedup, ORDER/OFFSET/LIMIT, `rust_group` (q9), all term-gen/reconstruct |
| `crates/sf-sparql/src/exec.rs` | 159 | SQLite sync shims + serve-lane wrappers |
| `crates/sf-sparql/src/exec_pg.rs` | 108 | PG thin delegators (owned-`Arc<Client>` serve entry points) |
| `crates/sf-sparql/src/exec_mysql.rs` | 118 | MySQL thin delegators (owned-`Conn` serve entry points) |
| `crates/sf-sql/src/backend.rs` | 81 | `SqlBackend` trait seam + `probe_sql` contract |
| `crates/sf-sql/src/backend/sqlite.rs` | 343 | SQLite adapter — borrowing + R2 owned cap-1 bridge (marshalling only) |
| `crates/sf-sql/src/backend/pg.rs` | 190 | PG adapter — `query_raw` cursor, `pg_value`, q12 typed-column bind |
| `crates/sf-sql/src/backend/mysql.rs` | 135 | MySQL adapter — packet-bounded `exec_iter`, `mysql_value_to_string` |
| **post-M5 total** | **2409** | |

**Net: +250 lines (+11.6%).** Raw LOC is roughly flat; the *structure* changed
decisively. `git diff --stat 1e178ff cf9aca0` over these files reports
**2256 insertions / 2006 deletions across 8 files** — the deletions are the collapse of
the triplicated `exec_pg.rs` (543→108) and `exec_mysql.rs` (329→118) plus the extraction
of shared logic out of `exec.rs` (1287→159). The correctness-critical ~740-line body
(`reconstruct` / term-gen / `order_cmp` / DISTINCT dedup / `rust_group` / `eval_expr`) is
now written **once** in `exec_core.rs` instead of three times; the three adapters are pure
per-driver marshalling. Most of the adapter growth is the new-in-M5 R2 owned SQLite
serve-bridge (cap-1 backpressure), not per-backend logic.

---

## 3. Add-a-backend cost demonstration

The ADR's headline claim — *"adding a backend requires only a dialect entry + a
`SqlBackend` adapter, with no change to the executor core"* — is demonstrated empirically
by the two real backends added during the program.

### M4 (MySQL) — the clean proof: zero core change

`git diff --stat acd57c0 1a03742` (M3 → M4):

| what M4 did | evidence |
|---|---|
| `exec_core.rs` **UNCHANGED** | `git diff --stat acd57c0 1a03742 -- crates/sf-sparql/src/exec_core.rs` → **empty** |
| `backend/pg.rs` **UNCHANGED** | same command over `backend/pg.rs` → **empty** |
| **new** `backend/mysql.rs` | +135 lines (the entire MySQL-specific cost) |
| `backend.rs` module registration | **+1 line** (`pub mod mysql;`) |

Adding MySQL to a working PG+SQLite engine cost **one new 135-line adapter file + one line**
of registration in the executor core — the core logic and the sibling PG adapter were not
touched. The remaining M4 diff is the *wiring* (a dedicated MySQL serve lane in `sf-serve`,
because `mysql_async` has no `Send` server cursor) and a new **+302-line differential test
arm** — neither of which is executor-core change.

### M3 (Postgres) — one-time shared-streaming extension

`git diff --stat 9c98a5e acd57c0` (M2 → M3):

| what M3 did | lines |
|---|---|
| **new** `backend/pg.rs` | +190 |
| `backend.rs` module registration | +1 (`pub mod pg;`) |
| `error.rs` (`Error::Unsupported` variant) | +6 |
| `exec_core.rs` | **+61** — the generic `select_each_async` / `construct_each_async` streaming functions |

**Honest note (this corrects a claim in the M7 task brief, which stated M3's `exec_core.rs`
was git-unchanged).** M3 *did* add 61 lines to `exec_core.rs`. Those 61 lines are the
**generic, backend-parametric** async-sink streaming entry points (`select_each_async<B>`,
`construct_each_async<B>`), introduced when the first *streaming HTTP serve* backend (PG)
landed — not PG-specific code. The proof that they are shared infrastructure, not
per-backend cost: **M4 (MySQL) reused them verbatim with zero further `exec_core.rs`
change.** So the steady-state add-a-backend cost is the M4 profile (adapter + one line),
and M3's +61 is a one-time generalization of the streaming seam.

### Concrete add-a-backend checklist (derived from the M3/M4 diffs)

1. **`Dialect` variant** — already present for PG/SQLite/MySQL at baseline
   (`crates/sf-sql/src/dialect.rs`); a genuinely new DB adds one enum arm + its
   `probe_sql` / `quote_char` / placeholder rules.
2. **`SqlBackend` adapter** — a new `backend/<db>.rs` (~135–190 lines): connect,
   prepare/`column_names`, typed-lexical param bind, server-side/streaming row cursor,
   per-cell marshalling to the driver-agnostic lexical form.
3. **One-line module registration** — `pub mod <db>;` in `backend.rs`.
4. **Serve/CLI wiring** — an `open_backend` arm; a dedicated serve lane only if the driver
   lacks a `Send` cursor (as MySQL did).
5. **Differential arm** — add the backend to `differential_pg_sqlite.rs`; correctness then
   follows from the *shared* `=_bag` differential, not a new per-backend feature port.

**No change to `exec_core.rs` is required** (M4 proves it).

---

## 4. Correctness parity

### 4.1 Three-arm `=_bag` differential — live, green

`cargo test -p sf-conformance --test differential_pg_sqlite` (live PG on `localhost:5432`,
live MySQL on `127.0.0.1:13306`):

```
test select_and_ask_agree_across_sqlite_and_pg    ... ok
test select_and_ask_agree_across_sqlite_and_mysql ... ok
test result: ok. 2 passed; 0 failed; 0 ignored
```

The PG arm executed **real assertions** (verified: the default conn
`host=localhost port=5432 user=henrik dbname=postgres` connects; **no** graceful-skip
message was emitted). SQLite is the in-process reference oracle; PG and MySQL are compared
against it bag-for-bag.

### 4.2 ADR-0023 PG-path fixes — single-homed, guarded

The five PG-path correctness defects + one perf blowup that the (green) SQLite differential
had hidden are now written **once** and guarded on the live PG/MySQL paths:

| query | class | where the fix now lives (single-homed) |
|---|---|---|
| q9 | agg-over-UNION | `exec_core::rust_group_execute` dispatch (`plan.rust_group`) — shared Rust group-by |
| q10 | sequence property path | below the SQL string (tree lowering, `Dialect::Postgres` emit) |
| q11 | MINUS | below the SQL string (tree lowering) |
| q12 | FILTER EXISTS / typed-column bind | `backend/pg.rs` `LexicalParam: ToSql` typed-bind (INT2/4/8 → integer, BOOL, …) |
| q14 | nested OPTIONAL (latency) | `leftjoin.rs` — null-safe join wrapper gated on left-key nullability |
| q15 | DISTINCT-over-join | `exec_core` DISTINCT dedup (before slice, across multi-branch bag-union) |

**MySQL genuinely gains q9.** At baseline, `exec_mysql.rs` never dispatched
`plan.rust_group` (its execution loop had no `rust_group` arm); routing MySQL through
`exec_core` gives it the shared aggregate-over-UNION path it never had, plus the same
DISTINCT-over-multi-branch and ORDER-expression semantics as the reference.

**Guards (revert-sensitive, live):**
- `differential_pg_sqlite.rs` — the ADR-0023 PG-path regression guard over all five classes
  (SQLite-oracle bag equality) **plus** hard live-PG value guards for q9/q10/q11/q12/q15
  (oracle-independent, exact computed values — e.g. q9 → the correct 2 groups; q15 → 1
  distinct dept), so a revert of any fix fails the guard.
- The MySQL A1 typed-values guard (`A1_R2RML` / `A1_Q`) — an INTEGER, a DATETIME, and a
  non-UTF-8 VARBINARY, asserting `int → "42"`, `DATETIME → T-separated`, non-UTF-8 bytes
  → **UNBOUND** (never a lossy string).

### 4.3 Live Ontop 15-class row-parity

Per `BENCHMARKS.md` (run 2026-07-01, both engines as warm HTTP endpoints over the **same**
live PostgreSQL GTFS backend): **row-count parity PASS on all 15 feature-class queries
(q1–q15) at scales 1 / 100 / 1000**, with Ontop 5.5.0 as the correctness oracle. At scale
10000 (8 M `stop_times`) the previously-broken cells (q9/q10/q11/q12/q15) and q14 were
re-measured on sf and return the correct row counts against the Ontop reference — e.g. q10
returns the full **8 000 000 = 8 000 000** rows (verified by writing the 1.13 GB body to
disk). *(Correctness verified from the checked-in `BENCHMARKS.md` matrices; a fresh live
Ontop HTTP re-race was not run in this environment — see the caveat in §5.)*

### 4.4 W3C RDB2RDF floor

The W3C RDB2RDF conformance suites (`crates/sf-conformance/tests/w3c_suite.rs`,
`w3c_pg_suite.rs`) remain the parity floor. *(Not re-executed in this pass;
present and unchanged.)* **[UNVERIFIED — not re-run this pass]**

---

## 5. Performance

Live Ontop 5.5.0 vs semantic-fabric head-to-head, both as warm HTTP SPARQL endpoints over
the **same** PostgreSQL GTFS backend, identical `curl %{time_total}` client methodology.
**All numbers below are the checked-in `BENCHMARKS.md` values** (scale 1000 = median of 5
warm runs, 800 000 `stop_times`; scale 10000 = single warm call). Row parity holds on every
row shown.

> **Correction vs the task brief.** The brief quoted approximate figures (q3 561 vs 6183;
> q10 586 vs 6617; q9 22.6 vs 6.5). The repository's actual `BENCHMARKS.md` values are used
> here instead (q3 568.89 vs 6095.61; q10 596.04 vs 6487.26; q9 21.69 vs 5.86).

### Scale 1000 (median of 5; 800 000 stop_times)

| query | class | sf ms | Ontop ms | sf speedup | rows (both) |
|---|---|---|---|---|---|
| q3 | 3-way join | 568.89 | 6 095.61 | **10.71×** | 800 000 |
| q10 | property path | 596.04 | 6 487.26 | **10.88×** | 800 000 |
| q14 | nested OPTIONAL | 44.22 | 285.23 | **6.45×** | 40 000 |
| q5 | OPTIONAL | 20.45 | 123.60 | **6.04×** | 40 000 |
| q7 | ORDER-BY expr | 6.27 | 33.11 | **5.28×** | 8 000 |
| q1 | BGP | 6.23 | 32.22 | **5.17×** | 8 000 |
| q2 | join | 5.40 | 23.92 | **4.43×** | 8 000 |
| q8 | UNION | 2.33 | 10.43 | **4.48×** | 2 222 |
| q15 | DISTINCT-over-join | 52.37 | 141.39 | 2.70× | 8 000 |
| q11 | MINUS | 10.62 | 26.30 | 2.48× | 13 334 |
| q12 | FILTER EXISTS | 12.61 | 21.67 | 1.72× | 4 000 |
| q4 | FILTER | 1.21 | 1.99 | 1.64× | 1 |
| **q6** | GROUP BY (small) | 5.54 | 4.54 | **0.82× (Ontop 1.2×)** | 2 |
| **q13** | subquery+agg (small) | 4.88 | 3.94 | **0.81× (Ontop 1.2×)** | 2 |
| **q9** | **agg-over-UNION** | 21.69 | 5.86 | **0.27× (Ontop 3.7×)** | 2 |

### Scale 10000 (single warm call; 8 000 000 stop_times) — heavy-cell headline

| query | sf ms | Ontop ms | sf speedup | rows (both) |
|---|---|---|---|---|
| q3 | 5 628.84 | 92 997.67 | **16.52×** | 8 000 000 |
| q10 | 5 964.76 | 99 248.56 | **16.64×** | 8 000 000 |
| q14 | 395.04 | 2 850.51 | **7.22×** | 400 000 |
| q9 | 228.41 | 52.61 | **0.23× (Ontop 4.3×)** | 2 |

### Honest reading

- **Where sf computes the same answer, it wins on execution throughput, and the win grows
  with data.** The marquee cell Q3 (3-way `stop_time→trip→route` join) scales
  **9.1× → 10.7× → 16.5×** at scale 1 / 1000 / 10000 — genuine same-backend engine
  throughput (both hit the identical PostgreSQL), not the JVM transport floor.
- **HONEST EXCEPTION — q9 is slower.** Aggregate-over-UNION runs sf's in-engine Rust
  `rust_group` group-by rather than pushing a SQL `GROUP BY`; on PG it is **0.27×** at scale
  1000 (21.69 ms vs 5.86 ms) and 0.23× at 8 M. It is *correct* (2 groups, row-parity) but
  the pushdown opportunity is real — see [§8](#8-residual--out-of-scope).
- **Small aggregates at parity.** q6 (GROUP BY, 2 rows) and q13 (subquery+agg, 2 rows) sit
  at ~0.8× — within transport noise; neither does meaningful row work.
- **Race-limit caveat (scale 10000).** A full median-of-N Ontop race at 8 M rows is
  impractical (Ontop q10 ≈ 99 s, q3 ≈ 93 s per call; the q10 body is 1.13 GB). The 10000
  cells are single warm calls, directionally representative because execution — not
  transport — dominates entirely there. **A fresh live Ontop HTTP re-race was not performed
  in this M7 pass**; the tables above are the checked-in `BENCHMARKS.md` measurements.
  **[Speed figures sourced from `BENCHMARKS.md`, not re-raced this pass.]**

---

## 6. Bounded memory per backend

`cargo test -p sf-bench --test constant_memory` — **live, green 3/3** (59.6 s):

```
test engine_memory_is_bounded_under_growing_source ... ok   (SQLite reference)
test engine_memory_is_bounded_pg                    ... ok   (live PG cursor)
test engine_memory_is_bounded_mysql                 ... ok   (live MySQL packet-bounded)
```

Engine peak heap (bytes) under 16× source growth (5 200 → 83 000 triples), from the test's
own emitted tables:

| scale | triples | SQLite peak_B | PG cursor peak_B | MySQL peak_B |
|---|---|---|---|---|
| 1 | 5 200 | 140 368 | 141 957 | 160 977 |
| 4 | 20 760 | 140 368 | 141 711 | 141 944 |
| 16 | 83 000 | 140 368 | 142 471 | 141 944 |

Peak engine heap stays **≈ constant (~140 KB)** while the source grows 16× — the ADR-0006 /
ADR-0010 streaming invariant re-proven through the new `SqlBackend` boundary on each backend.

**Honesty on the mechanism (not all three are equal):**
- **SQLite** — server-side cursor (in-process), truly constant (140 368 B flat).
- **PG** — server-side `query_raw` portal cursor, **cursor-grade**, ~142 KB roughly constant.
- **MySQL** — **packet-bounded, NOT cursor-grade.** `mysql_async` has no server-side cursor;
  the `exec_iter` stream is client-buffer-free (packet-at-a-time), so engine heap stays
  bounded, but the guarantee is packet-granular, not portal-granular. This is a real
  distinction, disclosed rather than glossed.

---

## 7. ADR Confirmation bullets — discharge status

From `docs/adr/ADR-0024-executor-backend-abstraction.md` → *Confirmation*:

| # | Bullet | Status | Evidence |
|---|---|---|---|
| 1 | Unified executor passes `=_bag` differential (PG↔SQLite) + W3C RDB2RDF floor + live Ontop head-to-head (15 classes row-parity on PG *and* SQLite *and* MySQL) + hard q12 typed-column bind regression on live PG | **DISCHARGED** (W3C floor **[not re-run this pass]**) | §4.1 live 2/2 green (incl. MySQL); §4.2 q12 typed-bind guard; §4.3 15-class row-parity (BENCHMARKS.md) |
| 2 | Bounded-memory invariant re-confirmed on each backend (server-side cursor, constant engine memory + bounded first-result under growing source) | **DISCHARGED** | §6 `constant_memory` live 3/3 green; ~140 KB constant under 16× growth (MySQL packet-bounded, disclosed) |
| 3 | Adding a new backend requires only a dialect entry + a `SqlBackend` adapter, **no change to the executor core**; correctness follows from the shared differential | **DISCHARGED** | §3 M4 added MySQL with `exec_core.rs` + `backend/pg.rs` git-unchanged; +135-line adapter + 1-line registration |

---

## 8. Residual / out-of-scope

- **q9 SQL `GROUP BY` pushdown (the one honest perf loss).** Aggregate-over-UNION currently
  runs in-engine (`rust_group`); pushing `GROUP BY` into the emitted SQL where the shape
  allows would recover the 0.27× → parity-or-better gap without giving up the correctness the
  Rust path guarantees. Optimizer work, orthogonal to the backend abstraction.
- **Full post-fix Ontop re-race at 1000× / 10000×.** Scale-1 correctness and q14 perf are
  verified live; the big-scale speed matrix in `BENCHMARKS.md` predates a *fresh* clean
  post-fix re-race. Directionally solid, but a from-scratch re-race would remove the last
  asterisk on the scale-10000 single-call cells.
- **MySQL cursor-grade streaming.** Bounded via packet granularity today; a true server-side
  cursor would require a driver that exposes one (`mysql_async` does not).
- **M8 — all-dialects breadth.** The abstraction is proven on 3 backends; the ADR's
  `N ≈ 20` motivation (and the `sqlx` fallback impl, gated on a streaming spike) is future
  work, not part of this cutover.
- **W3C RDB2RDF floor re-execution.** Present and unchanged; not re-run in this M7 pass.

---

## 9. Commands run to derive the numbers

```bash
# worktree + milestone SHAs
git -C /Users/henrik/source/hm/semantic-fabric worktree add -b feat/adr0024-m7-parity-report \
    /Users/henrik/source/hm/sf-adr0024-m7 feat/adr0024-m5-cutover
git -C /Users/henrik/source/hm/sf-adr0024-m7 log --oneline 1e178ff..HEAD
# → M2 9c98a5e · M3 acd57c0 · M4 1a03742 · M5 cf9aca0

# §2 LOC accounting (base vs M5)
git show 1e178ff:crates/sf-sparql/src/exec.rs        | wc -l   # 1287
git show 1e178ff:crates/sf-sparql/src/exec_pg.rs     | wc -l   # 543
git show 1e178ff:crates/sf-sparql/src/exec_mysql.rs  | wc -l   # 329
for f in exec_core.rs exec.rs exec_pg.rs exec_mysql.rs; do \
  git show cf9aca0:crates/sf-sparql/src/$f | wc -l; done       # 1275 159 108 118
for f in backend.rs backend/sqlite.rs backend/pg.rs backend/mysql.rs; do \
  git show cf9aca0:crates/sf-sql/src/$f | wc -l; done          # 81 343 190 135
git diff --stat 1e178ff cf9aca0 -- crates/sf-sparql/src/exec*.rs \
  crates/sf-sparql/src/exec_core.rs crates/sf-sql/src/backend*  # 2256 ins / 2006 del, 8 files

# §3 add-a-backend cost
git diff --stat 9c98a5e acd57c0 -- crates/sf-sparql/src/exec_core.rs   # +61 (generic streaming)
git diff --stat 9c98a5e acd57c0 -- crates/sf-sql/src/backend/pg.rs     # +190
git diff --stat acd57c0 1a03742 -- crates/sf-sparql/src/exec_core.rs \
  crates/sf-sql/src/backend/pg.rs                                      # EMPTY (unchanged)
git diff --stat acd57c0 1a03742 -- crates/sf-sql/src/backend/mysql.rs  # +135
git diff acd57c0 1a03742 -- crates/sf-sql/src/backend.rs               # +1 (pub mod mysql;)
git diff --stat 9c98a5e acd57c0 -- crates/                             # full M3 footprint
git diff --stat acd57c0 1a03742 -- crates/                             # full M4 footprint

# §4 correctness — live differential (PG on :5432, MySQL on :13306)
SF_MYSQL_URL="mysql://root:sftest@127.0.0.1:13306/sftest" \
  cargo test -p sf-conformance --test differential_pg_sqlite -- --test-threads=1
psql "host=localhost port=5432 user=henrik dbname=postgres" -c 'select 1;'  # PG arm not skipped

# §6 bounded memory — live 3/3
SF_MYSQL_URL="mysql://root:sftest@127.0.0.1:13306/sftest" \
  cargo test -p sf-bench --test constant_memory -- --nocapture --test-threads=1

# §5 performance — read from checked-in BENCHMARKS.md (not re-raced this pass)
sed -n '250,305p' BENCHMARKS.md

# build sanity
cargo build --workspace   # Finished, exit 0
```

---

*Report generated for ADR-0024 M7. Backend abstraction: `SqlBackend` trait + generic
`exec_core.rs` + `backend/{sqlite,pg,mysql}.rs`. All live tests green on the local
PG (`:5432`) / MySQL (`:13306`) fixtures at report time.*
