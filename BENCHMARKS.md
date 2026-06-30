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

> **Update (2026-06-30):** the asymmetry that the original tables carried (sf
> in-process over SQLite vs Ontop HTTP over PostgreSQL) is now **removed** for the
> head-to-head. semantic-fabric ships a real `serve` SPARQL endpoint with a wired
> PostgreSQL OBDA executor, so `scripts/compare/race.sh` runs **both** engines as
> warm HTTP SPARQL endpoints over the **same** PostgreSQL `gtfs_bench` database with
> the **same** `curl %{time_total}` median-of-N timer. See
> [Ontop vs semantic-fabric — head-to-head](#ontop-vs-semantic-fabric--head-to-head-adr-0023)
> below. The older single-engine tables and the SQLite-only limitation notes are
> retained for provenance but are superseded by that run for cross-engine claims.

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

## Ontop vs semantic-fabric — head-to-head (ADR-0023)

**Run date: 2026-06-30.** The real, symmetric race the program targeted. Both
engines run as **warm HTTP SPARQL endpoints over the same PostgreSQL `gtfs_bench`
database** (scale 1), timed with the **identical** client methodology:
`curl %{time_total}`, `Accept: text/csv`, 3 warm-up calls then **median of 25**
timed round-trips per query. This removes both asymmetries the original tables
carried (process model and backend are now the same for both engines).

```bash
scripts/load_gtfs_postgres.sh 1
cargo build --release -p sf-cli
ONTOP_HOME=/path/to/ontop-cli scripts/compare/race.sh 1 25
```

- **semantic-fabric**: `target/release/semantic-fabric serve --source pg:… --mapping
  scripts/ontop/gtfs.r2rml.ttl` — native Rust HTTP SPARQL endpoint, PostgreSQL OBDA
  executor (`exec_pg`).
- **Ontop 5.5.0**: `ontop endpoint -m … -p gtfs.properties` — JVM (Tomcat) HTTP
  SPARQL endpoint, PostgreSQL via JDBC 42.7.4. Boot log confirms a genuine run:
  `Starting OntopEndpointApplication v5.5.0 using Java 23.0.2` →
  `Ontop has completed the setup and it is ready for query answering!` →
  `Ontop virtual repository initialized successfully!`
- Same R2RML mapping (`scripts/ontop/gtfs.r2rml.ttl`), same queries
  (`scripts/ontop/q{1..7}.rq`), same PostgreSQL data.

### Result (median of 25 warm HTTP round-trips, scale 1)

| Query | Shape | sf median ms | Ontop median ms | sf speedup | sf rows | ont rows | rows-match? |
|---|---|---|---|---|---|---|---|
| Q1 | routes BGP | 0.52 | 1.87 | **3.60×** | 8 | 8 | ✅ |
| Q2 | route → agency join | 0.68 | 1.65 | **2.43×** | 8 | 8 | ✅ |
| Q3 | stop_time → trip → route (3-way) | 1.41 | 12.36 | **8.77×** | 800 | 800 | ✅ |
| Q4 | pushed-down FILTER (`?short = "R0"`) | 0.52 | 1.47 | **2.83×** | 1 | 1 | ✅ |
| Q5 | OPTIONAL (left join, headsign) | 0.53 | 1.57 | **2.96×** | 40 | 40 | ✅ |
| Q6 | GROUP BY + COUNT + ORDER BY DESC | 0.83 | 1.30 | **1.57×** | 2 | 2 | ✅ |
| Q7 | ORDER BY expression (`STRLEN`) | 0.53 | 1.18 | **2.23×** | 8 | 8 | ✅ |

**Row-count parity: PASS on all 7 queries** — every query returns the identical
answer cardinality from both engines over the same data. The latency comparison is
therefore meaningful (both engines compute the same result).

### Honest reading

- **semantic-fabric is faster on every query in this run**, geomean **3.64×** over
  the canonical Q1–Q5 set and **3.01×** over all seven. There is no query where
  Ontop wins.
- The largest gap is **Q3** (the 800-row 3-way join), where Ontop's median is
  12.36 ms vs sf's 1.41 ms (**8.77×**). On the small single-table / filter / join
  queries (Q1, Q2, Q4, Q5, Q7) Ontop clusters around a ~1.2–1.9 ms floor while sf
  sits near ~0.5–0.7 ms; the gap there is dominated by per-request engine + HTTP
  overhead, not result size. The narrowest gap is **Q6** (GROUP BY/COUNT, 2 rows)
  at 1.57×.
- **Remaining asymmetry — be honest:** the backend (PostgreSQL) and the
  client-side timer are now identical, but the two HTTP servers are not the same
  stack: Ontop serves via an embedded **JVM/Tomcat** server (Spring), semantic-
  fabric via its **native Rust** server. Some of Ontop's flat ~1 ms floor is JVM/
  Tomcat request overhead rather than query work, and a JVM endpoint carries a
  multi-hundred-MB heap vs sf's native footprint. This is an architectural
  difference, not something the harness subtracts out — but it is *much* smaller
  and fairer than the original SQLite-vs-PostgreSQL gap, and it does **not**
  account for the Q3 join gap (that is genuine engine throughput on the same SQL
  backend). Ontop remains the more feature-complete system; this race covers the
  subset both engines answer identically.

### Feature-class × scale matrix (q8–q15 + q1–q7, scales 1·100·1000·10000)

**Run date: 2026-07-01.** The scale-1 / q1–q7 race above only exercises simple
BGP/join/filter/optional/groupby/orderby. This section expands the head-to-head
two ways: (1) **eight new feature-class queries** `q8`–`q15`, one per SPARQL class
ADR-0023 targets, and (2) **four data scales** (1, 100, 1000, 10000) so EXECUTION —
not the ~1 ms transport floor — dominates. Ontop is treated as the **correctness
oracle** (the reference VKG/OBDA engine); each cell records both engines' median
HTTP latency, the row counts, and a parity/error status. Loaded PostgreSQL row
counts prove the data actually grew:

| scale | agency | calendar | routes | stops | trips | stop_times |
|---|---|---|---|---|---|---|
| 1 | 2 | 3 | 8 | 40 | 40 | 800 |
| 100 | 2 | 3 | 800 | 4 000 | 4 000 | 80 000 |
| 1000 | 2 | 3 | 8 000 | 40 000 | 40 000 | 800 000 |
| 10000 | 2 | 3 | 80 000 | 400 000 | 400 000 | 8 000 000 |

```bash
scripts/load_gtfs_postgres.sh 1000              # 893k rows total
ONTOP_HOME=/path/to/ontop-cli scripts/compare/race.sh 1000 10
```

The new queries and their feature class (each verified to return a **non-empty**
result against the Ontop oracle over the mapped GTFS data):

| query | feature class | sf vs oracle (correctness) |
|---|---|---|
| q8 | **UNION** (two-arm short/long name) | ✅ correct |
| q9 | **AGG-over-UNION** (COUNT over UNION + GROUP BY) — *the ADR-0023 headline* | ❌ **sf aborts** (HTTP 200 then mid-stream error) |
| q10 | **PROPERTY PATH** (sequence `gtfs:trip/gtfs:route`) | ❌ **sf returns 0 rows** (silently wrong) |
| q11 | **MINUS** (trips with no headsign) | ❌ **sf returns 0 rows** (removes everything) |
| q12 | **FILTER EXISTS** (routes with a direction-1 trip) | ❌ **sf aborts** (HTTP 200 then mid-stream error) |
| q13 | **SUBQUERY + nested agg** (sub-SELECT COUNT joined to agency) | ✅ correct |
| q14 | **NESTED OPTIONAL** (Trip ⟕ route ⟕ shortName) | ✅ correct (but catastrophically slow — see below) |
| q15 | **DISTINCT-over-join** (distinct routes via stop_times) | ❌ **sf returns duplicates** (DISTINCT not applied) |

#### Scale 1 (median of 25 warm runs)

| query | class | sf ms | ontop ms | sf speedup | sf rows | ont rows | status |
|---|---|---|---|---|---|---|---|
| q1 | BGP | 0.72 | 1.85 | 2.57× | 8 | 8 | OK |
| q2 | join | 0.69 | 1.62 | 2.35× | 8 | 8 | OK |
| q3 | 3-way join | 1.50 | 12.03 | **8.02×** | 800 | 800 | OK |
| q4 | filter | 0.52 | 1.37 | 2.63× | 1 | 1 | OK |
| q5 | optional | 0.54 | 1.40 | 2.59× | 40 | 40 | OK |
| q6 | groupby | 0.87 | 1.40 | 1.61× | 2 | 2 | OK |
| q7 | orderby expr | 0.56 | 1.24 | 2.21× | 8 | 8 | OK |
| q8 | union | 0.69 | 1.81 | 2.62× | 2 | 2 | OK |
| q9 | **agg-over-union** | ERR | 2.11 | — | — | 2 | **SF-501** |
| q10 | **property path** | 0.28 | 12.20 | — | 0 | 800 | **SF-EMPTY** |
| q11 | **minus** | 0.84 | 1.63 | — | 0 | 14 | **SF-EMPTY** |
| q12 | **filter exists** | ERR | 1.84 | — | — | 4 | **SF-501** |
| q13 | subquery | 1.26 | 1.48 | 1.17× | 2 | 2 | OK |
| q14 | nested optional | 1.80 | 1.61 | 0.89× | 40 | 40 | OK |
| q15 | **distinct** | 1.25 | 1.43 | — | 40 | 8 | **MISMATCH** |

#### Scale 100 (median of 25 warm runs; 80 000 stop_times)

| query | sf ms | ontop ms | sf speedup | sf rows | ont rows | status |
|---|---|---|---|---|---|---|
| q1 | 1.02 | 5.37 | 5.27× | 800 | 800 | OK |
| q2 | 1.12 | 3.66 | 3.27× | 800 | 800 | OK |
| q3 | 57.39 | 594.99 | **10.37×** | 80 000 | 80 000 | OK |
| q5 | 2.55 | 13.59 | 5.33× | 4 000 | 4 000 | OK |
| q7 | 1.13 | 4.26 | 3.77× | 800 | 800 | OK |
| q8 | 1.02 | 2.17 | 2.13× | 222 | 222 | OK |
| q9 | ERR | 2.20 | — | — | 2 | **SF-501** |
| q10 | 0.21 | 645.19 | — | 0 | 80 000 | **SF-EMPTY** |
| q11 | 1.57 | 3.28 | — | 0 | 1 334 | **SF-EMPTY** |
| q12 | ERR | 4.03 | — | — | 400 | **SF-501** |
| q14 | 698.11 | 30.49 | **0.04× (Ontop 22.9×)** | 4 000 | 4 000 | OK |
| q15 | 15.90 | 14.26 | — | 4 000 | 800 | **MISMATCH** |

(q4 0.57/1.43, q6 1.48/1.58, q13 1.41/1.30 — all OK, omitted for brevity.)

#### Scale 1000 (median of 5 warm runs; 800 000 stop_times — primary big-data headline)

| query | sf ms | ontop ms | sf speedup | sf rows | ont rows | status |
|---|---|---|---|---|---|---|
| q1 | 6.01 | 27.83 | 4.63× | 8 000 | 8 000 | OK |
| q2 | 5.48 | 23.48 | 4.28× | 8 000 | 8 000 | OK |
| q3 | 554.71 | 5 979.30 | **10.78×** | 800 000 | 800 000 | OK |
| q4 | 1.07 | 2.02 | 1.89× | 1 | 1 | OK |
| q5 | 20.46 | 122.51 | 5.99× | 40 000 | 40 000 | OK |
| q6 | 5.43 | 3.58 | 0.66× (Ontop 1.5×) | 2 | 2 | OK |
| q7 | 6.61 | 33.45 | 5.06× | 8 000 | 8 000 | OK |
| q8 | 2.28 | 9.35 | 4.10× | 2 222 | 2 222 | OK |
| q9 | ERR | 8.36 | — | — | 2 | **SF-501** |
| q10 | 0.24 | 6 419.39 | — | 0 | 800 000 | **SF-EMPTY** |
| q11 | 9.37 | 26.50 | — | 0 | 13 334 | **SF-EMPTY** |
| q12 | ERR | 21.12 | — | — | 4 000 | **SF-501** |
| q13 | 4.45 | 3.36 | 0.76× (Ontop 1.3×) | 2 | 2 | OK |
| q14 | 50 766.39 | 287.75 | **0.006× (Ontop 176×)** | 40 000 | 40 000 | OK |
| q15 | 183.93 | 141.20 | — | 40 000 | 8 000 | **MISMATCH** |

#### Scale 10000 (single warm call, RUNS=1 — partial; 8 000 000 stop_times)

At this scale each warm call is slow enough that a median-of-N is impractical for
the heavy cells (Ontop q10 ≈ 99 s, Ontop q3 ≈ 93 s, **sf q14 ≈ 91 s**), so these
are **single-call** wall times — directionally representative because execution, not
transport, dominates entirely here. Everything below completed; nothing was skipped.

| query | sf ms | ontop ms | sf speedup | sf rows | ont rows | status |
|---|---|---|---|---|---|---|
| q1 | 54.74 | 399.17 | 7.29× | 80 000 | 80 000 | OK |
| q2 | 48.93 | 244.72 | 5.00× | 80 000 | 80 000 | OK |
| q3 | 5 628.84 | 92 997.67 | **16.52×** | 8 000 000 | 8 000 000 | OK |
| q4 | 12.90 | 27.27 | 2.11× | 1 | 1 | OK |
| q5 | 197.22 | 1 255.59 | 6.37× | 400 000 | 400 000 | OK |
| q6 | 44.03 | 29.98 | 0.68× (Ontop 1.5×) | 2 | 2 | OK |
| q7 | 75.10 | 354.48 | 4.72× | 80 000 | 80 000 | OK |
| q8 | 19.19 | 101.31 | 5.28× | 22 222 | 22 222 | OK |
| q9 | ERR | 52.61 | — | — | 2 | **SF-501** |
| q10 | 0.43 | 99 248.56 | — | 0 | 8 000 000 | **SF-EMPTY** |
| q11 | 53.94 | 273.12 | — | 0 | 133 334 | **SF-EMPTY** |
| q12 | ERR | 196.10 | — | — | 40 000 | **SF-501** |
| q13 | 40.48 | 18.00 | 0.44× (Ontop 2.3×) | 2 | 2 | OK |
| q14 | 91 103.85 | 2 850.51 | **0.03× (Ontop 32×)** | 400 000 | 400 000 | OK |
| q15 | 2 155.77 | 2 879.32 | — | 400 000 | 80 000 | **MISMATCH** |

### Honest reading — feature classes × scale

- **Where sf computes the same answer, it wins on execution throughput, and the win
  grows with data.** The marquee cell is **Q3** (the 3-way `stop_time→trip→route`
  join): **8.0×** at scale 1 → **10.8×** at 800 k rows → **16.5×** at **8 M rows**
  (5.6 s vs 93.0 s). Q1/Q2/Q5/Q7/Q8 hold a steady **4–7×** at scale. This is genuine
  same-backend engine throughput (both hit the identical PostgreSQL), not the JVM
  floor. On the canonical correct-answer set sf is the faster engine at every scale.

- **🔴 AGG-over-UNION (Q9) — the ADR-0023 headline feature — FAILS on the live
  endpoint.** Ontop answers it correctly (2 rows) at every scale; **sf-serve returns
  HTTP 200 then aborts the response body mid-stream** (an uncaught executor error,
  not even logged). This is the exact bug class ADR-0023's operator tree was built to
  close — and it is still open *on the path that actually serves queries.* **Root
  cause (code-traced, honest):** `sf-serve` compiles via
  `sf_sparql::translate_cached → translate_with` — the **flat unfold path** — *not*
  `translate_tree`, the ADR-0023 operator-tree path. The tree IR that closed
  agg-over-union is proven in differential unit tests (`crates/sf-conformance`) but is
  **not wired into the `serve` endpoint**, so the live head-to-head exercises the flat
  path and inherits its gaps. Wiring `translate_tree` into `serve` is the fix.

- **🔴 Four more feature classes are silently wrong on sf-serve** (Ontop is the only
  correct engine): **Q10 sequence property path** → sf returns **0 rows** (Ontop
  returns all 800/80 k/800 k/8 M); **Q11 MINUS** → sf returns **0 rows** (removes
  everything; Ontop returns the correct 14/1 334/13 334/133 334); **Q12 FILTER EXISTS**
  with a join+filter → sf **aborts mid-stream** (Ontop answers 4/400/4 000/40 000);
  **Q15 DISTINCT-over-join** → sf returns **duplicates** (40 000 rows where Ontop
  returns 8 000 distinct). Simple variants work (bare `DISTINCT`, single-predicate
  path, bare `EXISTS`, agg-without-union all return correct rows on sf), so these are
  flat-path *combination* gaps, not total absences.

- **🟠 Ontop WINS where sf's flat plan degrades.** **Q14 nested multi-scan OPTIONAL**
  is correct on both engines but sf's flat plan is **catastrophic at scale**: par at
  scale 1 (1.8 ms vs 1.6 ms) → **176× slower** at 800 k rows (50.8 s vs 0.29 s) →
  32× slower at 8 M rows (91.1 s vs 2.85 s). Ontop also edges sf on the tiny-result
  aggregates **Q6/Q13** at scale ≥ 1000 (≈1.3–2.3×) where the result is 2 rows and
  plan quality, not scan throughput, decides.

- **No ONTOP-501 — the capability gap runs the *other* way.** The premise that Ontop
  would 501 on paths/subqueries did **not** materialize: **Ontop 5.5.0 answered every
  one of q1–q15 correctly at every scale.** There is no sf capability advantage in
  this set; on the contrary, sf-serve is the engine that fails (q9/q12) or silently
  mis-answers (q10/q11/q15) five of the eight feature classes. The honest bottom line:
  **sf is materially faster on the OBDA join/scan workload it executes correctly, but
  Ontop is more correct and more robust across SPARQL feature classes on the live
  endpoint today.**

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

- ~~**No `serve` HTTP endpoint yet.**~~ **(Superseded 2026-06-30.)** `serve` is now
  a live SPARQL 1.2 Protocol endpoint (`sf-serve`); the head-to-head above runs it
  as a warm HTTP server. The *single-engine* `semantic-fabric results` tables below
  remain in-process bench/test figures (criterion), not HTTP.
- ~~**SQLite-only execution.**~~ **(Superseded 2026-06-30.)** A PostgreSQL OBDA
  executor (`exec_pg`) is now wired; `serve --source pg:…` executes over PostgreSQL
  (used by the head-to-head). The in-process `obda_latency` / `constant_memory`
  benches below still run over embedded SQLite.
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
