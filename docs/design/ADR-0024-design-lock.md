# ADR-0024 M1 DESIGN-LOCK — `SqlBackend`: one async pull-cursor core, thin GAT adapters

Status: **LOCKED (M1 design-only)** · Branch `feat/operator-tree-ir` HEAD `1e178ff` · Horizon `adr-0024-executor-backend-abstraction-horizon`
Authority: `docs/adr/ADR-0024-executor-backend-abstraction.md`
Constraint gates preserved: ADR-0006 (native drivers + push-down + bounded memory), ADR-0007 (`=_bag` + term-construction lifted), ADR-0010 §C (streaming governance).
No code under `crates/` modified. Every signature, line anchor, and structural fact below is read from source at this HEAD.

---

## 0. Scope and verdict

Unify the three per-database executors — `crates/sf-sparql/src/exec.rs` (1287, rusqlite, sync), `exec_pg.rs` (543, tokio-postgres, async), `exec_mysql.rs` (329, mysql_async, async) — behind **one** driver-agnostic async pull-cursor core generic over a `SqlBackend` trait, plus three thin per-driver adapters. Per-DB variation is confined to exactly **two** places: dialect SQL generation (`emit.rs` + `dialect.rs`, preserved) and the `SqlBackend` adapter.

Three decisive facts, verified in source, settle the shape:

1. `PgRowStream` is **owned and `'static`** — `struct PgRowStream { inner: Pin<Box<tokio_postgres::RowStream>>, seen: u64 }` (`stream.rs:128`), no lifetime parameter. PG's pull cursor needs no bridge.
2. `SqliteRowStream<'stmt>` is **borrowed + sync** — yields `&rusqlite::Row`, holds `Rows<'stmt>` borrowing the `Statement` (`stream.rs:82-103`). SQLite is genuinely self-referential and *must* be bridged sync→async.
3. `mysql_async`'s result stream borrows `&mut Conn` and MySQL has **no server-side cursor** (`stream.rs:173-178`, `exec_mysql.rs:7-11,159-162`). MySQL is the only driver whose pull cursor must borrow the handle, and the only one that cannot offer cursor-grade backpressure.

**Locked spine:** async `SqlBackend` with a **GAT** `type Stream<'s>` (PG/SQLite streams are `'static` and satisfy it trivially; MySQL borrows `&'s mut Conn`); a **pull** cursor (`next_row().await`) keeping `=_bag`-critical DISTINCT/ORDER/OFFSET state as plain locals; **typed-bind folded into `open_branch`** (a standalone `bind_lexical_typed_param(value, ty)` cannot be spelled agnostically — `ty` is `tokio_postgres::types::Type`). Static dispatch only (`run::<B>()`), never `dyn`.

---

## 1. Final trait signatures — new file `crates/sf-sql/src/backend.rs`

`sf-sql` already links `sf-core` and all three driver crates, and `error.rs` already has `#[from]` for `rusqlite` / `tokio_postgres` / `mysql_async`, so the trait adds **zero** new error plumbing (returns `sf_sql::Result`).

```rust
//! The single per-database execution seam (ADR-0024). Everything BELOW the emitted
//! SQL string. Home = sf-sql (alongside dialect.rs / stream.rs / error.rs).

use crate::error::Result;
use sf_core::datatype::XsdTypeCode;

/// One projected result row, marshalled into the driver-agnostic lexical form the
/// term-gen core consumes (ADR-0003 R3 / ADR-0007). The adapter has ALREADY:
///   * extracted each cell to its lexical string (NULL ⇒ None) via the driver's
///     existing per-cell decoder (SQLite `lexical_typed`, PG `pg_value`, MySQL
///     `mysql_value_to_string` — reused VERBATIM, see §2),
///   * derived the §10 natural XSD code (ADR-0015) where the driver carries type
///     info (SQLite decltype+storage-class, PG result-column Type, MySQL all-None v1),
///   * applied per-dialect lexical normalisation (SQLite CHARACTER(n) blank-pad).
/// Owned by value; one row's Vecs are freed each row — the exact per-row allocation
/// the three executors already perform. No driver-native Row and no driver lifetime
/// ever crosses this boundary.
pub struct RawTuple {
    pub values: Vec<Option<String>>,
    pub codes:  Vec<Option<XsdTypeCode>>,
}

/// A bounded pull cursor over ONE emitted branch SELECT. One row in flight; the
/// signature CANNOT return a `Vec<Row>`, so no impl can buffer the full result set
/// (ADR-0006 / ADR-0010 §C "bounded by shape"). See §4 for the per-driver strength
/// this shape actually confers (cursor-grade for PG/SQLite; client-buffer-free but
/// packet-bounded for MySQL).
pub trait BranchStream {
    /// Next row, or None at end. A mid-stream marshalling failure is a HARD Err
    /// (never a silent short read): the SQLite bridge forwards `Result<RawTuple>`
    /// over its channel so an Err surfaces here rather than closing as clean EOF
    /// (resolves refutation A2).
    async fn next_row(&mut self) -> Result<Option<RawTuple>>;
}

/// One driver's connect-time-fixed / prepare / typed-bind / server-side-cursor
/// surface. Used ONLY via static dispatch (`run::<B>()`), never `dyn SqlBackend`, so
/// async-fn-in-trait (stable ≥1.75) + GAT (stable ≥1.65) carry no object-safety cost.
pub trait SqlBackend {
    /// GAT so the stream may borrow the handle for its lifetime. PG's PgRowStream is
    /// 'static and SQLite's channel-bridged Receiver is 'static (both satisfy any 's
    /// trivially); ONLY MySQL's native stream actually borrows `&'s mut Conn`.
    type Stream<'s>: BranchStream where Self: 's;

    /// Prepare-time result-column NAMES of `probe_sql`, in projection order, for
    /// `emit::resolve_col` identifier case-folding. Metadata only — fetches no rows.
    /// `probe_sql` is built ONCE by the core via `Dialect::probe_sql` (§2, resolves
    /// refutation C1), so no SQL is generated inside this method. A per-source
    /// failure is swallowed by the caller (catalog omits the source; resolution
    /// falls back to the raw identifier), so this returns Result but the core never
    /// `?`-propagates it (resolves refutation A4).
    async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>>;

    /// Open a server-side cursor for one emitted branch and bind `lexical_params`
    /// (= `EmittedBranch::params`, every value a `&str`) as N positional params.
    ///
    /// TYPED-BIND CONTRACT (the q12 fix, generalised): each lexical value MUST bind
    /// so it satisfies the parameter type the emitted SQL implies for that
    /// placeholder. A dynamically-typed backend binds the string as-is
    /// (SQLite/MySQL); a statically-typed backend parses the lexical form to the
    /// DRIVER-INFERRED native type (PG, via the `LexicalParam: ToSql` logic moved
    /// verbatim into `pg.rs`). The core NEVER performs this coercion — it emits only
    /// `Vec<String>` and has no bind site — so the q12 class is structurally
    /// impossible in shared code.
    async fn open_branch<'s>(
        &'s mut self,
        sql: &str,
        lexical_params: &[String],
    ) -> Result<Self::Stream<'s>>;
}
```

Two methods, one GAT, one associated type. `&mut self` is the superset (MySQL's `mysql_column_names` / native stream require it; PG/SQLite hold `Arc<Client>` / `Arc<Mutex<Connection>>` and tolerate it). No `connect`/`pool` method: connection acquisition and pooling/cancel-on-drop (`stream.rs:120-124`) stay a serve-lane concern (ADR-0010 §C), deliberately not smuggled into this seam — with **one** serve-lane requirement added for MySQL (§4.2, resolves B2).

**Send/AFIT reconciliation.** No explicit `Send` bound on the trait; `Send` is required only at the concrete monomorphized spawn site (`tokio::spawn(run::<PgBackend>(…))`). `PgRowStream`/`Client`, the SQLite `Arc<Mutex<Connection>>`-off-thread `Receiver`, and the MySQL `Conn` are all `Send`. Confirming AFIT + GAT + the generic async sink monomorphize and yield a `Send` future on the pinned toolchain for all three impls is the **M2 exit gate** (§5) — the one novel-language-feature risk, gated before any live-DB change.

### 1.1 Probe-SQL unification — new `Dialect::probe_sql` (in `dialect.rs`, place #1)

Ground truth: `build_catalog` probes `SELECT * FROM {quote_ident}` with **no** limit (`exec.rs:42`, `exec_pg.rs:48`), but `build_catalog_mysql` probes `SELECT * FROM {} LIMIT 0` (`exec_mysql.rs:54`). A string seam (`column_names(probe_sql: &str)`) would otherwise force the core to reproduce that MySQL-vs-others divergence with a `match dialect` — a **third** per-DB SQL-generation site outside emit.rs and the adapter, breaking the "two places" invariant (refutation C1, **valid**).

Resolution: add one helper to `dialect.rs` (already place #1, already owns `quote_ident`), uniform across all three dialects:

```rust
impl Dialect {
    /// Prepare-only metadata probe for a source's result-column names. `LIMIT 0` is
    /// uniform across SQLite/PG/MySQL: column metadata is available at prepare time
    /// regardless of LIMIT, and the statement never executes, so `LIMIT 0` vs none is
    /// immaterial to SQLite/PG and required by the existing MySQL path.
    pub fn probe_sql(&self, source: &LogicalSource) -> String {
        match source {
            LogicalSource::Table(t) => format!("SELECT * FROM {} LIMIT 0", self.quote_ident(t)),
            LogicalSource::Query(q) => q.clone(),
        }
    }
}
```

The core builds `plan.dialect.probe_sql(source)` and passes the string to `column_names`. SQL generation stays in the dialect layer; the seam stays agnostic; the invariant is restored. (Adding `LIMIT 0` to the SQLite/PG table probe is a prepare-only, metadata-neutral change — verified against `sqlite_column_names`/`prepare` semantics.)

---

## 2. Core / adapter file layout

### New generic core — `crates/sf-sparql/src/exec_core.rs` (written ONCE, generic `<B: SqlBackend>`)

Lift `exec_pg.rs`'s async orchestration into one generic function; move the already-shared pure helpers beside it. The two-function split is **preserved verbatim** to keep the `rust_group` non-recursion guard (`exec.rs:1070-1073`, `exec_pg.rs:337-338`):

```rust
async fn for_each_solution<B, F, Fut>(plan: &Plan, b: &mut B, sink: F) -> Result<()>
where B: SqlBackend, F: FnMut(&Branch, &BTreeMap<String,Term>) -> Fut, Fut: Future<Output=Result<()>>
{
    if let Some(rg) = &plan.rust_group { return rust_group_execute(plan, b, rg, sink).await; }
    for_each_branch_solution(plan, b, sink).await
}

async fn for_each_branch_solution<B, F, Fut>(plan: &Plan, b: &mut B, mut sink: F) -> Result<()> {
    let branches = plan.prepared_branches();                     // lib.rs push-down split — unchanged
    // Catalog: per-source, SWALLOW column_names errors (refutation A4).
    let mut catalog = ColumnCatalog::default();
    let mut seen_probe = HashSet::new();
    for branch in &branches {
        for (_, source) in branch.alias_sources() {
            let probe = plan.dialect.probe_sql(source);          // §1.1 — single SQL-gen home
            if !seen_probe.insert(probe.clone()) { continue; }
            if let Ok(names) = b.column_names(&probe).await { catalog.insert(source, names); }
        }
    }
    let multi = branches.len() > 1;
    let distinct_vars = /* exec.rs:374 verbatim */;
    let (mut seen_tuples, mut seen, mut emitted) = /* … */;
    let ordered = !plan.order.is_empty();
    let mut buffer: Vec<(usize, BTreeMap<String,Term>)> = Vec::new();
    for (bi, branch) in branches.iter().enumerate() {
        let e = emit::emit_branch_with(branch, plan.dialect, &catalog)?;   // PRESERVED seam, untouched
        let mut s = b.open_branch(&e.sql, &e.params).await?;               // the ONLY bind site
        while let Some(t) = s.next_row().await? {                          // one row in flight
            let raw = RawRow { schema: &e.projection, values: &t.values, codes: &t.codes };
            let bindings = reconstruct(branch, &raw)?;        // single term-gen chokepoint
            // ── CORRECTED sequence, mirroring exec.rs:439-511 (refutation A3) ──
            if multi { if let Some(vars) = &distinct_vars {   // 1. DISTINCT dedup (before slice)
                let key = vars.iter().map(|v| bindings.get(v).cloned()).collect();
                if !seen_tuples.insert(key) { continue; }
            }}
            if ordered {                                       // 2. ordered ⇒ buffer, DEFER slicing
                let bindings = inject_order_expr_keys(&plan.order, bindings);  // exec.rs:462-475
                buffer.push((bi, bindings));
                continue;                                      //    (streaming OFFSET/LIMIT SKIPPED)
            }
            if multi {                                         // 3. else: streaming OFFSET/LIMIT
                if seen < plan.offset { seen += 1; continue; }
                if let Some(limit) = plan.limit { if emitted >= limit { break; } }
            }
            emitted += 1;
            sink(branch, &bindings).await?;
        }
    }
    if ordered {                                               // 4. sort THEN slice (SPARQL §15)
        buffer.sort_by(|(_,a),(_,b)| order_cmp(&plan.order, a, b));
        let take = plan.limit.unwrap_or(usize::MAX);
        for (bi, bindings) in buffer.iter().skip(plan.offset).take(take) {
            sink(&branches[*bi], bindings).await?;
        }
    }
    Ok(())
}
```

`rust_group_execute<B>` collects inner solutions by calling the **same** generic `for_each_branch_solution<B>` with an inner `Plan{ rust_group: None, .. }` — the exact non-recursive split of `exec.rs:1075` / `exec_pg.rs:348`, so no recursive `async fn`, no boxing, no new monomorphization hazard. The `inject_order_expr_keys` helper is the extraction of `exec.rs:462-475` (SQLite-only today) — now uniform across all three backends.

**Moved into `exec_core.rs` once (deleted from all three exec files):** `RawRow`/`AliasRow` (exec.rs:122/140), `reconstruct`/`build_term`/`derived_term`/`natural_literal`/`avg_result_code` (157-304), `order_cmp`/`cmp_term`/`cmp_literal`/`numeric_value` (522-622), `eval_expr` + `eval_function`/`eval_bool` + ORDER-key injection (639-846, incl. the expression-key path SQLite-only today), `rust_group_execute`/`rust_group_result_rows`/`rust_agg` (1052-1287), `distinct_vars`+`seen_tuples` (q15), ORDER buffer+slice, Rust OFFSET/LIMIT, `instantiate`/`Solutions`, and the per-form entry points collapsed to one generic set: `select<B>/select_each<B>/ask<B>/construct<B>/construct_each<B>/dump_quads<B>`.

**Sync-API bridge (M2 detail).** The collecting convenience fns (`select`, `construct_triples`, `dump_quads`) become `async` + generic. Existing **sync** call sites (in-crate unit tests, the `=_bag` differential harness, any sync serve entry) wrap them with a runtime `block_on` shim; no test *logic* changes. This is a mechanical call-site change, gated by the M2 byte-identical differential.

### Three adapters — `crates/sf-sql/src/backend/{sqlite,pg,mysql}.rs`

`sf-sql` already links all three driver crates, so the per-cell marshalling moves here cleanly and **verbatim** (no decoder is rewritten — the correctness-bearing point of refutation A1).

| Adapter | `Stream<'s>` | `column_names` | `open_branch` + `next_row → RawTuple` |
|---|---|---|---|
| **pg.rs** (~90) | `PgRowStream` (**`'static`**) | `client.prepare` → `columns().name()` | bind via `LexicalParam: ToSql` (`exec_pg.rs:100-130` **verbatim** — the q12 fix); `PgRowStream::open` (`query_raw`); per row `pg_xsd_code(ty)`→codes, `pg_value(row,i,ty)`→values (`exec_pg.rs:67/138` **verbatim**) |
| **sqlite.rs** (~160) | owned `Receiver<Result<RawTuple>>` (**`'static`**) | `sqlite_column_names` | `open_branch` `spawn_blocking`s a task holding `Arc<Mutex<Connection>>`; it prepares `e.sql`, reads `declared_codes`+`declared_char_pads` (`exec.rs:62-103`), runs `sqlite_for_each`, and per row does `storage_class_code`+`lexical_typed`+`char_pad` (`exec.rs:109-116/309-339/403-430` **verbatim**), `blocking_send`ing each `Result<RawTuple>` into a **cap-1** channel; `next_row = rx.recv().await` maps `None`→EOF, `Some(Err)`→hard error |
| **mysql.rs** (~95) | native `QueryResult` streaming loop borrowing `&'s mut Conn` (**GAT**) | `mysql_column_names` | bind `Value::from(&str)`; drive `conn.exec_iter(stmt, Params::Positional)` and per row `row.take::<Value,usize>(i)` → **`mysql_value_to_string`** (`exec_mysql.rs:91-122` **verbatim**)→values; `codes = None` (v1). **Does NOT use `mysql_for_each`** (refutation A1). |

**Critical A1 correction — MySQL decoder.** The MySQL adapter is built on `exec_iter` + `row.take::<Value>(i)` + `mysql_value_to_string`, **not** on `stream.rs::mysql_for_each`. `mysql_for_each` reads every column as `Option<Vec<u8>>` and decodes via `String::from_utf8_lossy` (`stream.rs:199-207`), which (a) turns non-UTF-8 BLOB/VARBINARY bytes into `Some(replacement-chars)` = BOUND, where `mysql_value_to_string` yields `None`/unbound (`exec_mysql.rs:96`) — flipping OPTIONAL/MINUS/COALESCE outcomes and emitting a corrupt literal; and (b) loses the typed `Value::Date`/`Value::Time` → `T`-separated xsd lexical formatting (`exec_mysql.rs:101-121`), yielding wrong wire text or a `Vec<u8>` conversion failure. Reusing it is a `=_bag`/3-valued-logic regression mis-framed as a memory fix. The lock **forbids** it; the streaming primitive it retains is only the `while let Some(row) = result.next().await?` shape, with `row.take::<Value>` preserving the exact `fetch_rows` semantics minus the client buffer.

Per-DB variation is now exactly the two ADR-mandated places: **`emit.rs`/`dialect.rs`** (dialect SQL gen, incl. `probe_sql`) + **the adapter**. `iq.rs`, `unfold.rs`, `leftjoin.rs`, `path.rs` are unchanged.

---

## 3. Per-fix placement table (written once)

| Fix | Nature today | Locked home | Written-once mechanism |
|---|---|---|---|
| **q9** agg-over-UNION (`rust_group`) | dispatch duplicated (`exec.rs:351` / `exec_pg.rs:206`); **absent on MySQL** (`exec_mysql.rs:163` never checks `plan.rust_group`) | `exec_core::for_each_solution` dispatch + `rust_group_execute<B>` → shared `rust_group_result_rows` | core owns `if plan.rust_group`; an adapter has no row loop to omit it. **MySQL gains q9 for free** (verified: no rust_group handling anywhere in `exec_mysql.rs`). |
| **q10** sequence path | already single-home | `path.rs` (plan build) — untouched | runs as emitted SQL through the one adapter |
| **q11** MINUS→NotExists | already single-home | `unfold.rs` + `emit.rs` dialect render — untouched | below the SQL string |
| **q12** typed-column bind | driver mechanism, **PG-only** (`exec_pg.rs:100-130`) | `sf-sql/src/backend/pg.rs::open_branch` (`LexicalParam` verbatim) | core has **no bind site** (emits `Vec<String>`); contract on `open_branch`; one live-PG guard test |
| **q14** nested-OPTIONAL perf | already single-home | `leftjoin.rs null_safe` — untouched | plan shaping, feeds emit |
| **q15** DISTINCT-over-join | triplicated (`exec.rs:374/448`, `exec_pg.rs:233`, `exec_mysql.rs:171`) | `exec_core::for_each_branch_solution` (dedup **before** ORDER/slice) | one core copy; ordering invariant guaranteed by being one function |
| *bonus* ORDER-BY expression keys | SQLite-only injection (`exec.rs:462-475`) | `exec_core` (`inject_order_expr_keys` + `eval_expr`) | uniform across backends |
| *bonus* MySQL `fetch_rows` client buffering | client-side full-result buffer (`exec_mysql.rs:128-152`) | `sf-sql/src/backend/mysql.rs` streaming `next_row` (Value-decoded) | trait shape forbids `Vec<RawTuple>`; fixed by construction — **client buffer only**, see §4.2 |

---

## 4. Bounded-memory proof sketch (ADR-0006/0010, per driver — honestly scoped)

**Invariant:** one `RawTuple` in flight; the ONLY sanctioned buffers are ORDER-BY `buffer`, DISTINCT `seen_tuples`, and `rust_group` `inner_rows` — one of each, in the core, not one per backend. Net buffering sites drop from 3×N to 3.

**Trait-level guarantee (universal):** `BranchStream::next_row -> Result<Option<RawTuple>>` yields one owned row; no `Vec<Row>` appears in any signature, so **no backend can hand back the result set from engine memory**. This is what the trait shape buys — for *all three* — and no more.

**Per-driver strength (differentiated — resolves B1/C3):**

- **PG — cursor-grade.** `PgRowStream` over `query_raw` (server-side portal, `stream.rs:145`), never `query()`. Per-row `sink(..).await` propagates HTTP→TCP backpressure to the backend. Cancel-on-drop (`stream.rs:120`) unchanged. True `⟨T,M⟩`-bounded.
- **SQLite — cursor-grade, strengthened.** `rusqlite`'s lazy cursor (`sqlite_for_each`, one `&Row` live) behind a **cap-1** mpsc bridge caps materialisation at ~2 rows; `blocking_send` blocks the cursor thread until the core consumes → explicit backpressure. Bounded memory is *strengthened* vs today, not weakened.
- **MySQL — client-buffer-free, packet-bounded (NOT cursor-grade).** `mysql_async` has **no** server-side cursor (`stream.rs:173-178`, `exec_mysql.rs:159-162`). `exec_iter` removes only the client-side `Vec<Row>`; the server still eagerly pushes the full result set, bounded on the client at packet / TCP-buffer granularity (`max_allowed_packet`), not one row, with no per-row server backpressure. The claim credited here is **exactly** "no engine-side full-result buffer + one `RawTuple` in flight," and the pre-existing honest caveat (`stream.rs:176-178`) is carried forward, not overwritten. "Repaired by construction" applies to the *client buffer*, not to conferring cursor semantics.

### 4.1 Sync↔async reconciliation

One async `SqlBackend`. PG/MySQL are async-native and collapse their loops into the core directly. SQLite's sync, self-referential `SqliteRowStream<'stmt>` is bridged via `spawn_blocking` + cap-1 channel — matching how `sf-serve/stream.rs` already runs SQLite, so the sync/async seam moves *into* the adapter and `sf-serve` can later drop its SQLite `spawn_blocking` special-case (M5). The `!Send` `Connection` lives only on the blocking thread; the core future stays `Send`. Cost, stated honestly: one channel hand-off per row on the SQLite/CI path — FIFO (`=_bag`-preserving), cap-1 (bounded).

**SQLite reactor-CPU relocation (resolves B3).** Under this design the blocking thread produces `RawTuple`; `reconstruct()` (oxrdf term construction) then runs on the tokio worker via the core. Rebuttal + accept: the PG path **already** calls `reconstruct()` on the async reactor per row (`exec_pg.rs:275`, inside the async `while` loop, no `spawn_blocking`) — reactor-side term-gen is the *established, shipped, bounded* pattern, not an anti-pattern SQLite alone must avoid. Today's SQLite `spawn_blocking`-wraps-everything is forced by `rusqlite` being sync + `!Send`, not by a deliberate "keep term-gen off the reactor" policy. Moving SQLite to bridge-raw + reconstruct-on-reactor makes it *consistent* with the accepted PG path; per-row term-gen is small bounded CPU, throttled by cap-1 backpressure. The relocation is recorded here explicitly (not sold as "one hand-off only"). If a future embedded-SQLite heavy-streaming workload shows reactor starvation, the mitigation (reconstruct on the blocking side of the bridge, yielding reconstructed rows) stays local to `sqlite.rs` — the trait boundary makes that swap non-invasive.

### 4.2 MySQL connection-holding tradeoff (resolves B2)

Today `fetch_rows` fully drains via `conn.exec(...).await` into `Vec<Row>` **first** — freeing the connection from the result-pending state — **then** iterates the owned Vec against the slow async sink (`exec_mysql.rs:137-152` → `184-219`). Streaming keeps the `QueryResult` (which pins `&mut Conn` result-pending) open across **every** `sink(..).await` for the whole slow-client download. Net: streaming cuts client memory but **pins the connection non-recyclable for the stream's full lifetime** — strictly more connection-hold-time than today, and weaker than PG (which documents cancel-on-drop/discard).

This is the correct trade under ADR-0006 (bounded memory is the hard constraint; MySQL has no cursor to give both), and it is locked **with** a serve-lane requirement, not silently:

> **Serve-lane requirement (M4):** a MySQL streaming request owns a **dedicated pooled connection** for the stream's full lifetime and **discards/resets it on early drop** (LIMIT reached / deadline / client-gone), mirroring PG's cancel-on-drop discard note (`stream.rs:120-124`). The M4 gate asserts **connection release under a slow-consumer test**, not merely that the `Vec<Row>` is gone.

---

## 5. Build order M2 → M7 (smallest-change, reuse-first; every gate `=_bag`- and bounded-memory-blocking)

**M2 — trait + generic core + SQLite adapter; prove on the trusted oracle.**
Add `sf-sql/src/backend.rs` (`SqlBackend`, `BranchStream`, `RawTuple`) + `Dialect::probe_sql` + `backend/sqlite.rs` (cap-1 `Result<RawTuple>` bridge; move `declared_codes`/`declared_char_pads`/`storage_class_code`/`lexical_typed`/`char_pad` in verbatim). Extract `exec.rs` orchestration into `exec_core.rs` generic over `B`, with the **corrected** reconstruct→dedup→(buffer|slice)→sort→slice sequence (§2). SQLite entry points delegate to `run::<SqliteBackend>`; add the `block_on` sync shim for existing sync callers. `exec_pg.rs`/`exec_mysql.rs` stay parallel; delete nothing yet.
**Gate:** full SQLite `=_bag` differential + W3C RDB2RDF floor + `sf-bench constant_memory` byte-identical/green — validates the generic core against the in-process oracle before any live-DB change; **AND** confirm AFIT + GAT + generic sink monomorphize and yield a `Send` future on the pinned toolchain (the one novel-language-feature risk).

**M3 — Postgres onto the core; delete the PG loop.**
Add `backend/pg.rs` (move `PgRowStream` use, `LexicalParam`, `pg_value`, `pg_xsd_code` in verbatim). Point PG entry points at `run::<PgBackend>`; delete `for_each_branch_solution_pg` + `rust_group_execute_pg`.
**Gate:** live Ontop 5.5.0 head-to-head — all 15 feature classes row-parity on PG; q9/q11/q15 hold; the **hard q12 typed-column regression test on the live PG path** (ADR Confirmation bullet 1); PG `constant_memory` bench.

**M4 — MySQL onto the core (Value-streaming); close the latent gaps.**
Add `backend/mysql.rs` on the `exec_iter` + `row.take::<Value>` + `mysql_value_to_string` loop (retires `fetch_rows`; **`mysql_for_each` not used**). MySQL thereby gains q9 `rust_group`, q15 dedup, and ORDER-expression keys. Point MySQL entry points at `run::<MysqlBackend>`; delete `for_each_solution_mysql`. Add the `Backend::Mysql` arm to `sf-serve/src/lib.rs` + `dialect()` + the §4.2 dedicated-connection serve-lane rule.
**Gate:** MySQL added to the differential **with an explicit `=_bag` case covering an integer, a `DATETIME`, and a non-UTF-8 `BLOB` column** proving parity with the `fetch_rows` baseline (guards A1); assert q9 now passes; assert the client `Vec<Row>` buffer is gone **and** connection release under a slow-consumer test (guards B2); MySQL `constant_memory` bench (packet-bounded caveat asserted, not cursor-grade); land a MySQL typed-bind test (or an explicit test documenting the v1 string-passthrough gap).

**M5 — full cutover + `sf-serve` collapse.**
Delete residual `exec.rs`/`exec_pg.rs`/`exec_mysql.rs` orchestration; collapse `sf-serve` dispatch to uniform `run::<B>` and drop the SQLite `spawn_blocking` special-case (the adapter now owns SQLite's blocking).
**Gate:** entire `=_bag` differential + all-three live head-to-head green; net LOC accounting recorded (~2159 lines of triplicated `exec*.rs` → one generic core + three adapters).

**M6 — backend-cost proof + shared datatype module.**
Demonstrate ADR Confirmation bullet 3: adding a backend = one `Dialect` entry + `probe_sql` arm + one ~100-line adapter, no core change (walk it as a doc/PoC, no new backend shipped). Factor the reusable §10 helpers (`natural_xsd`/`char_pad_len`/`storage_class_code`) into a shared `sf-sql` datatype module so `sqlite.rs` and any future client-typed backend share them, keeping adapter bodies thin (addresses C2's optional structural note).
**Gate:** SQLite adapter body re-measured against the ~160-line budget (§2); "add-a-backend" checklist reviewed against the actual diff surface; full differential still green.

**M7 — `constant_memory` sign-off + fallback close-out.**
Re-run `sf-bench constant_memory` per backend through the new boundary (ADR Confirmation bullet 2): constant engine memory + bounded first-result latency under growing source data for PG (cursor), SQLite (cursor), MySQL (packet-bounded, documented). Record the `sqlx` fallback as **shelved** — native adapters proven bounded-memory in M3/M4; the trait boundary keeps that swap local if ever revisited.
**Gate:** `constant_memory` green on all three (with MySQL's packet-bounded scope stated); horizon `adr-0024-executor-backend-abstraction-horizon` closed; ADR-0024 Confirmation bullets 1–3 all discharged.

---

## 6. Refutations resolved

Every adversarial finding below is addressed *in this design* (no deferrals). Severity from the review; disposition is FIXED (design changed to close the gap) or REBUTTED-WITH-EVIDENCE + ACCEPTED-NOTE.

| # | Sev | Finding | Disposition |
|---|---|---|---|
| **A1** | **blocker** | MySQL adapter reusing `mysql_for_each` is a term-recon / 3-valued-logic regression (lossy `from_utf8_lossy` → BOUND vs `None`; lost `Value::Date/Time` typed formatting), mis-framed as memory-only. | **FIXED.** §2 forbids `mysql_for_each`; adapter built on `exec_iter` + `row.take::<Value>(i)` + `mysql_value_to_string` **verbatim**, preserving None-on-non-UTF-8 and typed temporal lexical forms. M4 gate adds an explicit integer + `DATETIME` + non-UTF-8 `BLOB` `=_bag` case vs the `fetch_rows` baseline. **Blocker closed.** |
| **A2** | minor | SQLite closure can hard-error mid-row (non-UTF-8 text `exec.rs:314-318`; BLOB-in-non-hexBinary `319-321`); sending only `RawTuple` would turn a channel close into silent EOF truncation. | **FIXED.** Channel payload is `Result<RawTuple>`; a marshalling `Err` is `blocking_send`-forwarded and surfaced by `next_row` as a hard error. `None` means clean EOF only. Specified in `BranchStream` doc + §2 sqlite row. |
| **A3** | minor | Prose "DISTINCT → streaming OFFSET/LIMIT → ORDER buffer.push" slices before sorting; wrong per SPARQL §15 and `exec.rs:462-511`. | **FIXED.** §2 core pseudocode restated verbatim to `exec.rs:439-511`: reconstruct → dedup → if ordered {buffer, defer} else {streaming OFFSET/LIMIT} → after loop sort THEN slice. |
| **A4** | minor | Generic `build_catalog` with `?` on `column_names` would hard-fail queries the three current variants keep alive by omitting a source (`exec.rs:48` etc.). | **FIXED.** §1/§2: core swallows per-source `column_names` errors (`if let Ok(names)`, omit source, fall back to raw identifier); `?` reserved for non-recoverable failures. |
| **B1 / C3** | major / minor | "Bounded by shape / repaired by construction" overstates MySQL — `mysql_async` has no server-side cursor; it is packet-bounded, not cursor-grade. | **FIXED (scoped).** §4 differentiates: trait guarantees only "no engine-side full-result buffer + one `RawTuple` in flight" universally; cursor-grade credited to PG (`query_raw`) + SQLite (lazy cursor) only; MySQL carries the `stream.rs:176-178` packet-bounded caveat forward into §4.2 and the M4/M7 gates. |
| **B2** | major | Streaming MySQL pins `&mut Conn` (result-pending) across every `sink().await` — strictly worse connection-holding than `fetch_rows` (which drains first). | **FIXED (accepted-with-mitigation).** §4.2 states the tradeoff explicitly and adds an M4 serve-lane requirement: dedicated pooled connection per MySQL stream, discard/reset on early drop (mirroring PG cancel-on-drop); M4 gate asserts connection release under a slow-consumer test. |
| **B3** | minor | SQLite bridge silently relocates per-row `reconstruct()` from the blocking pool onto the reactor. | **REBUTTED + ACCEPTED-NOTE.** §4.1: the PG path already runs `reconstruct()` on the reactor per row (`exec_pg.rs:275`) — it is the shipped, accepted, bounded norm; SQLite's all-in-`spawn_blocking` is forced by `!Send`+sync, not a term-gen-off-reactor policy. Relocation recorded explicitly (not sold as one hand-off); local mitigation (reconstruct on the blocking side) named if starvation ever appears. |
| **C1** | major | `column_names(probe_sql: &str)` forces the core to reproduce the MySQL `LIMIT 0` vs no-limit probe divergence — a **third** SQL-gen site, breaking "two places." | **FIXED.** §1.1: probe construction moves into `Dialect::probe_sql` (place #1, already owns `quote_ident`), unified to `SELECT * FROM {} LIMIT 0` (prepare-only, metadata-neutral for SQLite/PG). Seam stays string-based; invariant restored. |
| **C2** | minor | SQLite adapter is ~150-170 lines, not ~110; the sync→async bridge fattens it. | **FIXED.** §2 table re-labels `sqlite.rs` **~160**, noting the sync→async bridge (not marshalling) as the cause; M6 extracts shared §10 helpers into an `sf-sql` datatype module to keep the body thin, with M6 gate re-measuring against the budget. |

**Unresolved blockers: 0.** The single blocker (A1) is closed by forbidding `mysql_for_each` and reusing `mysql_value_to_string` verbatim, guarded by a dedicated M4 differential case. All three majors (B1, B2, C1) are closed by scoping the bounded-memory claim, adding the MySQL serve-lane connection rule + slow-consumer gate, and relocating probe SQL into `dialect.rs`.

**Design-lock compliance:** no code under `crates/` modified; every signature, line anchor, and the three decisive structural facts (`PgRowStream` `'static`; `SqliteRowStream<'stmt>` borrowed+sync; `mysql_async` no server cursor + `&mut Conn` borrow + `mysql_for_each` lossy decode) are read from source at HEAD `1e178ff`.
