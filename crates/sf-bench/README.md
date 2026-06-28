# sf-bench — GTFS-Madrid OBDA benchmark + constant-memory demonstration

The performance benchmark driver for semantic-fabric (ADR-0005), on the
**GTFS-Madrid-Bench OBDA / query-rewriting track**. Two things it proves:

1. **OBDA query latency** — live SPARQL→SQL over a relational source, no
   materialisation (ADR-0006).
2. **Constant engine memory + bounded first-result latency under growing source
   data** — the differentiator (ADR-0006 / ADR-0013): the source DB does the
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

## Captured results

Measured on this box (Apple Silicon, macOS), `--release` via criterion
(`--measurement-time 2 --warm-up-time 1`). Numbers are indicative — the
*invariant* (constant memory, bounded first result), not the absolute latency, is
the load-bearing result; absolute latency feeds the Path-B objective (ADR-0013).

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
