# Benchmarks

Reproducible performance numbers for **semantic-fabric** on the GTFS-Madrid-Bench
OBDA / query-rewriting workload, plus a repeatable head-to-head harness against
**[Ontop](https://ontop-vkg.org/)** (the reference open-source VKG/OBDA engine)
over the *same* GTFS data.

Every number below was produced by an actual run on the machine described under
[Environment](#environment); the exact command is shown above each table. Nothing
here is estimated or extrapolated. Where two systems are not measured under
identical conditions, that is stated explicitly — see
[Measurement asymmetry](#measurement-asymmetry-read-this-before-comparing).

> **Honesty contract.** semantic-fabric's load-bearing result is the
> **constant engine memory under growing source data** invariant — a property of
> the streaming architecture, demonstrated byte-for-byte below. Absolute query
> latency is reported but is **not** a clean apples-to-apples figure against Ontop
> (different process model and backend; see the asymmetry section). Ontop is a
> mature, feature-complete system; semantic-fabric is an early engine with real
> limitations (see [Limitations](#semantic-fabric-limitations)).

---

## Environment

| | |
|---|---|
| Machine | Apple M5 Max, 36 GB RAM |
| OS | macOS 26.4 (Darwin 25.4.0), arm64 |
| Rust | rustc / cargo 1.96.0 |
| semantic-fabric | this checkout, `--release` (criterion) / default profile (test) |
| PostgreSQL | 17.7 (Homebrew) at `localhost:5432` |
| Java | OpenJDK 23.0.2 (Temurin) |
| Ontop | ontop-cli 5.5.0, PostgreSQL JDBC 42.7.4 |

semantic-fabric executes over **embedded SQLite** (its only wired backend today —
see [Limitations](#semantic-fabric-limitations)); Ontop executes over
**PostgreSQL** via JDBC. Both answer the same SPARQL over the same logical dataset.

---

## The dataset

A self-contained, cross-reference-consistent subset of the official
[GTFS-Madrid-Bench](https://github.com/oeg-upm/gtfs-bench) (six core GTFS tables;
GTFS vocabulary `http://vocab.gtfs.org/terms#`; subject IRIs under
`http://transport.linkeddata.es/madrid/metro/`). The official R2RML mapping and
relational schema are vendored verbatim under
`crates/sf-bench/vendor/gtfs-madrid-bench/`.

The dataset is produced by a **deterministic generator** parameterised by a scale
factor `s`. semantic-fabric generates it into SQLite
(`crates/sf-bench/src/workload.rs::generate`); `scripts/gen_gtfs.sql` reproduces
the **identical rows** in PostgreSQL for Ontop. Per-scale row counts:

| Table | Rows |
|---|---|
| `agency` | 2 |
| `calendar` | 3 |
| `routes` | 8·s |
| `stops` | 40·s |
| `trips` | 40·s |
| `stop_times` | 800·s |

CONSTRUCT-dump output is ≈ 5 200·s triples (dominated by `stop_times`).

The five representative SELECT queries (`scripts/ontop/q{1..5}.rq`, identical to
the SPARQL in `workload.rs::queries`):

| Query | Shape | Result rows @1x / @10x |
|---|---|---|
| Q1 | single-table BGP (routes) | 8 / 80 |
| Q2 | 2-way join (route → agency) | 8 / 80 |
| Q3 | 3-way join (stop_time → trip → route) | 800 / 8 000 |
| Q4 | pushed-down FILTER (`?short = "R0"`) | 1 / 1 |
| Q5 | OPTIONAL (NULL-safe left join, trips) | 40 / 400 |

Result cardinalities were verified **equal** between semantic-fabric and Ontop at
every scale (parity check) — the two systems return the same answers.

---

## semantic-fabric results

### Per-query OBDA latency @1x (SELECT, full result)

```bash
cargo bench -p sf-bench --bench obda_latency
```

criterion medians (default config: 3 s warm-up, 100 samples), in-process:

| Query | Median |
|---|---|
| Q1 routes BGP | 29.35 µs |
| Q2 route → agency join | 37.83 µs |
| Q3 stop_time → trip → route join | 738.2 µs |
| Q4 route FILTER | 26.69 µs |
| Q5 trip OPTIONAL headsign | 96.35 µs |

### CONSTRUCT dump — full streamed latency

From the same `obda_latency` bench (`obda_construct_dump`) and the
`constant_memory` bench (`constant_memory_dump`):

| Scale | Triples | Full dump (median) |
|---|---|---|
| 1x | 5 200 | 3.12–3.18 ms |
| 10x | 51 880 | 27.7–28.5 ms |
| 100x | 518 680 | 290.2 ms |

Latency grows linearly with result size (≈ constant throughput, ~1.8 M triples/s)
— the source does the set-work, the engine streams.

### First-result vs total latency

Printed by `cargo bench -p sf-bench --bench obda_latency` (streamed CONSTRUCT dump):

| Scale | Triples | First result | Total |
|---|---|---|---|
| 1x | 5 200 | 67.0 µs | 3.284 ms |
| 10x | 51 880 | 64.2 µs | 28.431 ms |
| 100x | 518 680 | 64.4 µs | 283.017 ms |

First-result latency stays bounded (~65 µs) while total grows with the result —
the streaming, non-materialising path.

### Constant engine memory — the differentiator

```bash
cargo bench -p sf-bench --bench constant_memory
```

Peak engine heap high-water during the streamed CONSTRUCT dump, measured by a
process-wide tracking allocator (source data lives in a file-backed SQLite DB, off
the engine heap):

| Scale | Triples | Peak engine heap | Bytes / triple |
|---|---|---|---|
| 1x | 5 200 | **129 358 B** | 24.88 |
| 10x | 51 880 | **129 358 B** | 2.49 |
| 100x | 518 680 | **129 358 B** | 0.249 |

The engine peak heap is **byte-identical (129 358 B) across a 100× growth in
source data and result size**, while bytes/triple collapses toward zero. This is
the `O(|T| + |M| + batch)` invariant — engine working set independent of source
size — demonstrated.

The same invariant is asserted as a fast unit test (default/debug profile):

```bash
cargo test -p sf-bench --test constant_memory -- --nocapture
```

| Scale | Triples | Peak engine heap |
|---|---|---|
| 1x | 5 200 | 129 358 B |
| 4x | 20 760 | 129 358 B |
| 16x | 83 000 | 129 358 B |

The test asserts peak-heap growth ≤ 4× **and** ≪ row-growth across scales (here:
exactly 1×, against a 16× row growth).

---

## Ontop comparison (real run)

These are **real Ontop 5.5.0 numbers**, captured on this machine over the same
logical GTFS dataset loaded into PostgreSQL.

### How it was run

```bash
# 1. Load the identical dataset into PostgreSQL at a scale factor
scripts/load_gtfs_postgres.sh 1            # then 10, 100, ...

# 2. Get the Ontop CLI + JDBC driver (one-time)
curl -sSLO https://github.com/ontop/ontop/releases/download/ontop-5.5.0/ontop-cli-5.5.0.zip
unzip ontop-cli-5.5.0.zip -d ontop-cli
cp /path/to/postgresql-42.7.4.jar ontop-cli/jdbc/

# 3. Run the warm-endpoint timing harness
ONTOP_HOME="$PWD/ontop-cli" scripts/run_ontop_bench.sh 1     # SCALE PORT RUNS
```

`run_ontop_bench.sh` boots Ontop's SPARQL HTTP endpoint once, warms each query 3×,
then reports the **median wall-clock of 15 timed HTTP round-trips** (`curl
%{time_total}`, `Accept: text/csv`). This is the standard warm-endpoint
measurement used by GTFS-Madrid-Bench. Mapping: `scripts/ontop/gtfs.r2rml.ttl`
(R2RML functionally identical to semantic-fabric's, table/column names lowercased
for PostgreSQL). Connection: `scripts/ontop/gtfs.properties`.

### Ontop warm per-query latency (median HTTP round-trip)

| Query | @1x | @10x |
|---|---|---|
| Q1 routes BGP | 1.96 ms | 3.07 ms |
| Q2 route → agency join | 2.02 ms | 2.42 ms |
| Q3 stop_time → trip → route join | 13.44 ms | 56.74 ms |
| Q4 route FILTER | 1.46 ms | 1.40 ms |
| Q5 trip OPTIONAL headsign | 1.75 ms | 3.20 ms |
| CONSTRUCT dump (Turtle) | 110.7 ms | 1.272 s |

(Cold `ontop query` CLI invocations, by contrast, are ~3–5 s each — dominated by
JVM + mapping bootstrap — and are *not* a meaningful per-query figure; hence the
warm endpoint.)

---

## Measurement asymmetry — read this before comparing

The semantic-fabric and Ontop latency tables above are **not measured under
identical conditions**. Do not read them as a clean head-to-head; they answer
*"how fast does each system answer this query once warm, in its native usage
mode?"*, not *"which is faster all else equal"*. The differences:

| Dimension | semantic-fabric | Ontop |
|---|---|---|
| Process model | in-process Rust library call | warm JVM HTTP SPARQL endpoint |
| Measured boundary | parse → translate → execute → collect | HTTP request → rewrite → SQL → **serialize → HTTP response** |
| Network / serialization | none (in-process) | included (HTTP + CSV/Turtle serialization) |
| Backend | embedded SQLite | PostgreSQL via JDBC |
| Timer | criterion (statistical, µs) | `curl %{time_total}` median (ms) |

Consequences:

- Ontop's per-query figures include **HTTP + result serialization overhead**
  (roughly a ~1 ms floor here, visible in Q4's flat ~1.4 ms) that
  semantic-fabric's in-process numbers do **not** pay. A fairer Ontop number would
  subtract transport; we report the end-to-end warm figure because it is what the
  harness actually measures.
- The **backends differ** (SQLite vs PostgreSQL). semantic-fabric does not yet
  have a PostgreSQL *executor* (it can emit PostgreSQL SQL via `Dialect::Postgres`
  but executes only over SQLite — see Limitations), so a same-backend race is not
  currently possible from its bench harness.
- **Both engines are virtualisers** — neither materialises the RDF; both rewrite
  SPARQL to SQL and stream. "No materialisation" is therefore *not* a
  semantic-fabric advantage over Ontop; it is common ground.

What **is** a clean, defensible comparison:

1. **Result parity** — identical answer cardinalities at every scale (verified).
2. **Constant engine memory** (semantic-fabric, demonstrated above): the native
   engine's working set is byte-constant under 100× data growth. Ontop runs on a
   JVM with a multi-hundred-MB heap; semantic-fabric is a native binary with a
   ~130 KB engine working set on this workload. Measuring Ontop's heap on the same
   axis is left as an exercise (JVM heap accounting is not comparable to a native
   allocator high-water), so this is reported as semantic-fabric's intrinsic
   property, not a subtracted delta.
3. **Deployment shape** — semantic-fabric is a single native, no-JVM, embeddable
   binary; Ontop is a JVM service. This is an architectural difference, not a
   benchmark.

---

## semantic-fabric limitations

Stated plainly so the numbers are not over-read:

- **No `serve` HTTP endpoint yet.** The SPARQL 1.2 Protocol endpoint is a scaffold
  and returns not-implemented; all semantic-fabric numbers here come from the
  in-process bench/test harness, not an HTTP server.
- **SQLite-only execution.** The engine emits PostgreSQL/MySQL SQL but only
  *executes* over embedded SQLite today; there is no wired PostgreSQL executor.
- **Property paths** are single-predicate `P+` / `P*` only (no arbitrary path
  expressions).
- **W3C RDB2RDF conformance is not 100%:** 81/82 (SQLite) and 80/81 (PostgreSQL),
  with one documented standards deviation (`R2RMLTC0002f`). Run it yourself:
  `cargo run -p sf-cli -- conformance`.
- Features outside the v1 support surface return 501 / are skipped — they are not
  silently wrong, but they are not done.

---

## Reproduce everything

```bash
# semantic-fabric (this repo)
cargo bench -p sf-bench --bench obda_latency
cargo bench -p sf-bench --bench constant_memory
cargo test  -p sf-bench --test constant_memory -- --nocapture

# Ontop head-to-head (same logical data, PostgreSQL at :5432)
scripts/load_gtfs_postgres.sh 1
ONTOP_HOME=/path/to/ontop-cli scripts/run_ontop_bench.sh 1
# repeat at scale 10, 100, ... to watch each system scale
```

Harness files:

| Path | Role |
|---|---|
| `scripts/gen_gtfs.sql` | Loads the GTFS subset into PostgreSQL, matching `workload.rs::generate`. |
| `scripts/load_gtfs_postgres.sh` | (Re)creates `gtfs_bench` and loads it at a scale factor. |
| `scripts/run_ontop_bench.sh` | Boots Ontop's warm SPARQL endpoint and times the queries. |
| `scripts/ontop/gtfs.r2rml.ttl` | R2RML mapping for Ontop (lowercased for PostgreSQL). |
| `scripts/ontop/gtfs.properties` | Ontop JDBC connection to `gtfs_bench`. |
| `scripts/ontop/q{1..5}.rq`, `dump.rq` | The SPARQL queries (identical to sf-bench). |
