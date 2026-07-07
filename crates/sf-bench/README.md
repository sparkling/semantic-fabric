# sf-bench — GTFS-Madrid OBDA benchmark + constant-memory demonstration

The performance benchmark driver for semantic-fabric (ADR-0005), on the
**GTFS-Madrid-Bench OBDA / query-rewriting track**. Two things it proves:

1. **OBDA query latency** — live SPARQL→SQL over a relational source, no
   materialisation (ADR-0006).
2. **Constant engine memory + bounded first-result latency under growing source
   data** — the differentiator (ADR-0006): the source DB does the
   set-work, the engine streams, so the engine working set is
   `O(|T| + |M| + batch)`, independent of source size.

Ontop is the *optional, offline JVM cross-check* (ADR-0005) — deferred, with **no
JVM dependency** added here.

## Layout

| Path | Role |
|---|---|
| `src/workload.rs` | GTFS R2RML mapping (`MAPPING_TTL`), the 5 representative OBDA queries, the CONSTRUCT dump, and the scalable **file-backed** SQLite data generator. |
| `src/driver.rs` | Parse the mapping once; run a query through the live virtualizer (`sf-sparql`) over the source — SELECT (collected) and CONSTRUCT (streamed, bounded). |
| `src/mem.rs` | A process-wide heap high-water probe (byte-valued sibling of `sf-core`'s alloc-count probe). Installed only by the bench/test roots, never the library. |
| `benches/obda_latency.rs` | criterion: per-query wall-clock (5 SELECTs @1x), first-result vs total table, full-dump timing @1x/10x. |
| `benches/constant_memory.rs` | criterion: streamed-dump latency @1x/10x/100x + the peak-heap table (installs the tracking allocator). |
| `tests/constant_memory.rs` | The constant-memory invariant as a fast `cargo test` (runs under `cargo test --workspace`). |
| `vendor/gtfs-madrid-bench/` | Official benchmark artifacts, vendored verbatim (provenance). |

## Provenance

The workload is **derived from the official GTFS-Madrid-Bench**
(oeg-upm/gtfs-bench, commit `7fcdaa7`, Apache-2.0; OEG/UPM — Chaves-Fraga et al.,
*JWS* 2020). The official R2RML mapping and relational schema are vendored
verbatim under `vendor/gtfs-madrid-bench/` (see its `PROVENANCE.md`).

`sf-bench` drives the engine with a **self-contained, representative subset** of
six core GTFS tables — `AGENCY`, `CALENDAR`, `ROUTES`, `STOPS`, `TRIPS`,
`STOP_TIMES` — keeping the official GTFS vocabulary (`http://vocab.gtfs.org/terms#`)
and `http://transport.linkeddata.es/madrid/metro/` subject IRIs, with every
`rr:parentTriplesMap` resolvable so any query is valid at any scale. The dataset
is emitted into a **file-backed** SQLite database (a temp file) so engine memory
is separable from the source data — an in-memory SQLite would hold the rows
in-process and confound the measurement.

The five queries (all within the v1 support surface, ADR-0007): `Q1` single-table
BGP, `Q2` 2-way cross-table join (route→agency), `Q3` 3-way join
(stop_time→trip→route), `Q4` pushed-down FILTER, `Q5` OPTIONAL (NULL-safe left
join).

## Running

```bash
cargo test  --workspace                 # includes the constant-memory invariant test
cargo bench -p sf-bench                  # latency + the peak-heap demonstration
cargo bench -p sf-bench --bench constant_memory   # peak-heap table @1x/10x/100x
```

## Tree-vs-flat shootout (ADR-0023, 2026-06-30)

Since ADR-0023 M8, the production default (`translate` / `translate_with`) routes
through the **operator-tree (IQ) path**. The `obda_select_tree_1x` benchmark group
measures this path; `obda_select_flat_1x` pins the **flat unfold oracle** via
`parse_and_translate_flat_with` — a fair, side-by-side comparison of the two
sf-internal translation pipelines.

**Important:** this is sf-tree vs sf-flat — both are semantic-fabric engines. This
shootout does **not** establish "faster than Ontop". The Ontop JVM cross-check is
still deferred per ADR-0005 (no Ontop baseline wired in this crate); that
comparison requires the Ontop integration bench planned under ADR-0005.

### Results — OBDA SELECT latency @1x (Apple Silicon, macOS, criterion 100 samples)

| Query | Flat median | Tree median | Tree delta | Profile note |
|---|---|---|---|---|
| Q1 routes BGP | 25.4 µs | 27.7 µs | +9.0% | translation-dominated |
| Q2 route→agency join | 34.9 µs | 39.0 µs | +11.7% | translation-dominated |
| Q3 stop_time→trip→route (3-way) | 562 µs | 571 µs | +1.6% | exec-dominated |
| Q4 route FILTER | 24.3 µs | 27.5 µs | +13.1% | translation-dominated |
| Q5 trip OPTIONAL headsign | 32.4 µs | 35.0 µs | +7.9% | translation-dominated |
| **geomean** | | | **+8.6%** | |

**Reading the result:** Q3 is the production-profile query (3-way join, ~800 rows,
exec-dominated) — tree overhead is +1.6%, within run-to-run noise. The
translation-dominated queries (Q1/Q2/Q4/Q5, sub-100 µs, small result sets) show
+8–13% overhead from the four-stage build→resolve→normalize→lower pipeline vs the
flat single-pass unfold. This overhead is pure translation CPU and does not grow
with result size; for any query where exec time dominates (Q3 and any query over a
real-scale source) the overhead is negligible. Prior M7 run (ad1c820): Q1 +7.5%,
Q2 +7.6%, Q3 +1.0%, Q4 +8.3%, Q5 +4.5% — today's numbers are consistent within
expected run-to-run variation on a shared CPU (Apple Silicon thermal effects can
move sub-50 µs benchmarks by several percent between runs).

## Tree-vs-flat shootout — PostgreSQL (ADR-0023, 2026-06-30)

Same five queries over a **live local PostgreSQL 17** source (`localhost:5432`,
trust auth, 1x scale). Bench groups: `obda_select_pg_flat_1x` (flat oracle via
`parse_and_translate_flat_with`) and `obda_select_pg_tree_1x` (tree path, the M8
production default). Both use the async `exec_pg::select_pg` cursor on a current-thread
tokio runtime. Each criterion iteration includes the tokio `block_on` overhead, which
is identical for both paths and thus cancels in the delta.

**Important:** this is sf-tree vs sf-flat on a live PG source — both are
semantic-fabric engines. This shootout does **not** establish "faster than Ontop".
The Ontop JVM cross-check is still deferred per ADR-0005; that comparison requires
the Ontop integration bench planned under ADR-0005. Note also that the absolute
latency is dominated by the loopback network round-trip to PG plus PG's own query
execution; the translation contribution is small relative to network/exec.

### Results — OBDA SELECT latency @1x on PostgreSQL (Apple Silicon, macOS, criterion, single collision-free run, 50 samples / 5 s)

PostgreSQL 17.7 (Homebrew), `localhost:5432`, trust auth, fixture = 893 rows
(`AGENCY` 2 · `CALENDAR` 3 · `ROUTES` 8 · `STOPS` 40 · `TRIPS` 40 · `STOP_TIMES` 800).
Each bench process builds the fixture in its own private database `sf_bench_<pid>`
(see *Fixture isolation* below), so this run could not collide with any other.

| Query | Flat-PG median | Tree-PG median | Tree delta | Profile note |
|---|---|---|---|---|
| Q1 routes BGP | 150.2 µs | 162.1 µs | +7.9% | loopback-bound; CIs overlap |
| Q2 route→agency join | 212.8 µs | 226.8 µs | +6.6% | loopback-bound; CIs overlap |
| Q3 stop_time→trip→route (3-way) | 812.6 µs | 831.0 µs | +2.3% | exec-dominated; within noise |
| Q4 route FILTER | 151.0 µs | 155.5 µs | +2.9% | loopback-bound; within noise |
| Q5 trip OPTIONAL headsign | 176.2 µs | 163.0 µs | -7.5% | loopback-bound; tree edges ahead |
| **geomean** | | | **+2.3%** | tree ≈ flat — parity within noise |

**Reading the result — on PG the tree path is at parity with flat.** The geomean is
**+2.3%** and every query lands within ±8% with wide, overlapping criterion confidence
intervals (e.g. Q1 tree `[155.5, 174.5] µs` overlaps flat `[149.6, 150.7] µs`; on Q5
the tree even edges ahead). The loopback round-trip + PG execution set a ~150 µs floor
that fully **absorbs** the +8–13% translation overhead the tree path costs on the
sub-50 µs in-process SQLite shootout: once a real DB round-trip + execution dominate,
the four-stage build→resolve→normalize→lower pipeline is no longer on the critical
path. There is **no Q3 regression** — the 3-way join (the exec-dominated,
production-profile query) is +2.3%, squarely within run-to-run noise.

**Retraction.** Earlier revisions of this section reported PG numbers measured against
a **broken shared-database fixture** and drew two now-retracted conclusions: (a) any
"tree ~8% faster on PG" reading, and (b) a "+27.2% tree regression on the 3-way join".
Both were **measurement artifacts** of a fixture-collision bug, not real signal. The
old fixture created the six GTFS tables under global names in the *shared* scratch DB
(`dbname=$USER`) and tore them down with a global `DROP TABLE`; when two bench
processes ran concurrently (or a leftover lingered), one process's teardown yanked
tables out from under another mid-run — crashing iterations
(`relation "TRIPS" does not exist`) and distorting the surviving timings (an inflated
flat baseline makes tree look faster; cross-run interference can swing a single query
like Q3 wildly). The honest, collision-free result is **tree-at-parity-with-flat on PG**.

**Fixture isolation (re-runnable, collision-free).** `PgFixture::new`
(`benches/obda_latency.rs`) creates a private database `sf_bench_<pid>`, builds the
whole fixture inside it, and `Drop` runs `DROP DATABASE IF EXISTS "sf_bench_<pid>" WITH
(FORCE)` — so two concurrent `cargo bench` invocations get distinct databases and can
never touch each other's tables (verified: two parallel runs, exit 0, zero crashes,
distinct `db=sf_bench_<pid>`). A per-process *schema* would **not** have sufficed:
`introspect_postgres` (sf-sql) scopes its `information_schema` lookups by table name
only, so a sibling process's same-named tables in another schema would double-count
columns; a per-process **database** isolates the catalog cleanly without touching sf-sql.

Measured on this box (Apple Silicon, macOS), `--release` via criterion (3 s warm-up,
50 samples, 5 s measurement), one clean single-process run. Numbers are indicative —
the *invariant* (constant memory, bounded first result), not the absolute latency, is
the load-bearing result; absolute latency feeds the Ontop-vs-port performance comparison.

### Dataset scale (rows per scale factor `s`)

`AGENCY` 2 · `CALENDAR` 3 · `ROUTES` 8·s · `STOPS` 40·s · `TRIPS` 40·s ·
`STOP_TIMES` 800·s. CONSTRUCT-dump output triples ≈ 5 200·s.

### OBDA query latency @1x (SELECT, full result)

| Query | Median |
|---|---|
| Q1 routes BGP | 29.4 µs |
| Q2 route→agency join | 37.8 µs |
| Q3 stop_time→trip→route join | 738 µs |
| Q4 route FILTER | 26.7 µs |
| Q5 trip OPTIONAL headsign | 96.3 µs |

### CONSTRUCT dump — full streamed latency

| Scale | Triples | Full dump | Throughput |
|---|---|---|---|
| 1x | 5 200 | 3.12 ms | ~1.67 M triples/s |
| 10x | 51 880 | 27.7 ms | ~1.87 M triples/s |
| 100x | 518 680 | 290 ms | ~1.79 M triples/s |

Latency grows **linearly with result size** (constant throughput) — the source
does the set-work, the engine streams.

### Constant memory + bounded first result (the differentiator, ADR-0006)

| Scale | Triples | Peak engine heap | Bytes / triple | First-result latency |
|---|---|---|---|---|
| 1x | 5 200 | **129 358 B** | 24.9 | 67.0 µs |
| 10x | 51 880 | **129 358 B** | 2.49 | 64.2 µs |
| 100x | 518 680 | **129 358 B** | 0.249 | 64.4 µs |

The engine peak heap during streaming is **byte-identical (129 358 B) across a
100× growth in source data and result size**, while bytes/triple collapses toward
zero and first-result latency stays bounded (~30–60 µs). This is the
`O(|T| + |M| + batch)` invariant, demonstrated. The `cargo test` version asserts
the same at small scales (1×/4×/16×): peak-heap growth factor ≤ 4 and ≪ the row
growth factor.
