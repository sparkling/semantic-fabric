//! Micro-benchmarks for the M4 wave-1 perf quick wins: ORDER BY over mixed term
//! kinds (`cmp_term`'s `Term::to_string()` fallback), `COUNT`/`SUM(DISTINCT ?v)`
//! dedup (`rust_agg`), and a GROUP BY forced onto the Rust-level `rust_group` path
//! (ADR-0025 C.6 / M3 fix 1 routing) — three shapes the five GTFS `obda_select_*`
//! queries (`obda_latency.rs`) don't naturally exercise. A small, self-contained,
//! file-backed SQLite fixture (ADR-0006), same `TempDir` pattern as
//! `obda_latency.rs`, engineered so each query provably reaches the intended
//! routing decision (asserted once at setup, not per-iteration).
//!
//! `term_gen_batch` (M4 wave-2) is a fourth, unrelated-shape group: single-thread
//! vs batched-parallel term generation, added alongside these when `exec_core`'s
//! batch restructure landed — see that group's own doc comment.

use criterion::{criterion_group, criterion_main, Criterion};
use rayon::prelude::*;
use rusqlite::Connection;
use sf_core::ir::{Template, TermMap, TermSpec, TriplesMap};
use sf_sparql::{exec, parse_and_translate_with, Tbox};
use sf_sql::{Dialect, TableSchema};
use tempfile::TempDir;

/// Row count for the `ITEM` fixture table — large enough to make sorting /
/// dedup / grouping cost visible (per-query wall-clock, criterion `obda_select_1x`
/// sibling scale).
const N: i64 = 10_000;

/// A minimal R2RML mapping over one `ITEM` table, engineered to hit three
/// specific engine routing decisions — NOT the GTFS workload's five queries,
/// which don't naturally produce these shapes:
///
/// * `:linkTo` (IRI template) / `:labelText` (plain literal column) — a UNION of
///   the two binds `?v` to MIXED term kinds, for the ORDER BY sort bench
///   (`exec_core::cmp_term`).
/// * `:valA` (explicit `xsd:integer`) / `:valU` (same `val` column, no declared
///   datatype) — the `TermSpec` mismatch across the UNION's two arms defeats
///   `try_sql_group_over_union`'s cross-arm type-identity check (`iq/lower.rs`),
///   forcing `COUNT(DISTINCT ?v)` / `SUM(DISTINCT ?v)` onto the Rust-level
///   `rust_group`/`rust_agg` path instead of a pushed-down SQL
///   `COUNT(DISTINCT col)` (ADR-0025 Tier-2 gap 3).
/// * `:valU` alone, `AVG`-aggregated: an undeclared-datatype operand is not
///   PROVABLY `xsd:double`, so M3 fix 1's `avg_needs_exact_decimal` routes even
///   this single-branch GROUP BY onto `rust_group` (ADR-0025 C.6).
const MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix : <http://example.org/bench#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

<#item> a rr:TriplesMap ;
    rr:logicalTable [ rr:tableName "ITEM" ] ;
    rr:subjectMap [ rr:template "http://example.org/bench/item/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate :grp ;
        rr:objectMap [ rr:column "grp" ; rr:datatype xsd:integer ] ] ;
    rr:predicateObjectMap [ rr:predicate :valA ;
        rr:objectMap [ rr:column "val" ; rr:datatype xsd:integer ] ] ;
    rr:predicateObjectMap [ rr:predicate :valU ;
        rr:objectMap [ rr:column "val" ] ] ;
    rr:predicateObjectMap [ rr:predicate :linkTo ;
        rr:objectMap [ rr:template "http://example.org/bench/ref/{id}" ; rr:termType rr:IRI ] ] ;
    rr:predicateObjectMap [ rr:predicate :labelText ;
        rr:objectMap [ rr:column "label" ] ] .
"#;

/// (i) ORDER BY over a variable bound to MIXED term kinds — one UNION arm an
/// IRI, the other a plain literal.
const Q_ORDER_MIXED_KIND: &str = "
PREFIX : <http://example.org/bench#>
SELECT ?v WHERE {
  { ?s :linkTo ?v } UNION { ?s :labelText ?v }
} ORDER BY ?v";

/// (ii) `COUNT(DISTINCT ?v)`/`SUM(DISTINCT ?v)` forced onto `rust_group` by a
/// cross-arm datatype mismatch (`:valA` declared `xsd:integer`, `:valU`
/// undeclared) — `val = id % 1000`, so each of the 1000 distinct values repeats
/// 10x per arm (20x combined): real dedup work, not a no-op DISTINCT.
const Q_DISTINCT_AGG: &str = "
PREFIX : <http://example.org/bench#>
SELECT (COUNT(DISTINCT ?v) AS ?c) (SUM(DISTINCT ?v) AS ?s) WHERE {
  { ?a :valA ?v } UNION { ?b :valU ?v }
}";

/// (iii) GROUP BY + AVG over an undeclared-datatype operand — single-branch, but
/// M3 fix 1 still routes it onto `rust_group` (`avg_needs_exact_decimal`).
const Q_GROUP_AVG_RUST: &str = "
PREFIX : <http://example.org/bench#>
SELECT ?g (AVG(?v) AS ?avg) WHERE { ?s :grp ?g ; :valU ?v } GROUP BY ?g";

struct Fixture {
    _dir: TempDir,
    conn: Connection,
    maps: Vec<TriplesMap>,
    schemas: Vec<TableSchema>,
}

fn fixture() -> Fixture {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("micro.db");
    let conn = Connection::open(&path).expect("open sqlite");
    conn.execute_batch(
        "CREATE TABLE ITEM (id INTEGER PRIMARY KEY, grp INTEGER, val INTEGER, label TEXT);
         PRAGMA synchronous=OFF; PRAGMA journal_mode=MEMORY;",
    )
    .expect("create schema");
    {
        let tx = conn.unchecked_transaction().expect("tx");
        {
            let mut stmt = tx
                .prepare("INSERT INTO ITEM VALUES (?1,?2,?3,?4)")
                .expect("prepare insert");
            for id in 0..N {
                stmt.execute(rusqlite::params![
                    id,
                    id % 100,
                    id % 1000,
                    format!("label-{id}")
                ])
                .expect("insert row");
            }
        }
        tx.commit().expect("commit");
    }
    let maps = sf_mapping::parse_r2rml(MAPPING_TTL).expect("parse mapping");
    let schemas = vec![sf_sql::introspect::introspect_sqlite(&conn, "ITEM").expect("introspect")];
    Fixture {
        _dir: dir,
        conn,
        maps,
        schemas,
    }
}

fn translate(fx: &Fixture, sparql: &str) -> sf_sparql::Plan {
    parse_and_translate_with(
        sparql,
        &fx.maps,
        Dialect::Sqlite,
        &Tbox::default(),
        &fx.schemas,
    )
    .unwrap_or_else(|e| panic!("translate {sparql}: {e}"))
}

fn run(fx: &Fixture, sparql: &str) -> usize {
    exec::select(&translate(fx, sparql), &fx.conn)
        .unwrap_or_else(|e| panic!("exec {sparql}: {e}"))
        .rows
        .len()
}

/// Fail loudly (not silently benchmark the wrong path) if the cross-arm spec
/// mismatch / undeclared-datatype routing tricks above ever stop forcing
/// `rust_group` — a translator change that fixed this would make the "before"
/// half of the receipt meaningless without anyone noticing.
fn assert_routes_to_rust_group(fx: &Fixture, sparql: &str) {
    assert!(
        translate(fx, sparql).rust_group.is_some(),
        "routing assumption broken: {sparql} no longer routes to rust_group"
    );
}

fn bench_order_mixed_kind(c: &mut Criterion) {
    let fx = fixture();
    // Sanity: both UNION arms contributed (mixed IRI/literal kinds actually reach
    // the sort, not just one homogeneous arm).
    assert_eq!(run(&fx, Q_ORDER_MIXED_KIND), (2 * N) as usize);
    c.bench_function("micro_order_mixed_kind", |b| {
        b.iter(|| run(&fx, Q_ORDER_MIXED_KIND));
    });
}

fn bench_distinct_agg(c: &mut Criterion) {
    let fx = fixture();
    assert_routes_to_rust_group(&fx, Q_DISTINCT_AGG);
    assert_eq!(run(&fx, Q_DISTINCT_AGG), 1); // implicit grouping — one result row
    c.bench_function("micro_distinct_agg", |b| {
        b.iter(|| run(&fx, Q_DISTINCT_AGG));
    });
}

fn bench_group_avg_rust(c: &mut Criterion) {
    let fx = fixture();
    assert_routes_to_rust_group(&fx, Q_GROUP_AVG_RUST);
    assert_eq!(run(&fx, Q_GROUP_AVG_RUST), 100); // grp = id % 100 => 100 groups
    c.bench_function("micro_group_avg_rust", |b| {
        b.iter(|| run(&fx, Q_GROUP_AVG_RUST));
    });
}

// --- M4 wave-2: term-gen batch parallelism (ADR-0006 correction note) --------
//
// Single-thread vs batched-parallel term generation over ~100k synthetic rows —
// the receipt for the correction note's chunked-dispatch claim. Isolates the
// SAME mechanism `sf_sparql::exec_core::reconstruct_batch` uses (buffer,
// `rayon::par_chunks` at a `current_num_threads()`-derived chunk size, collect
// in order — never one rayon task per row) but applied directly to the public
// `sf_core::term::generate` primitive, which is the dominant per-row cost every
// non-constant term map (`exec_core::derived_term`) delegates to. A real
// `rr:template` IRI — the ADR's own example of where term-gen's allocation cost
// concentrates — not a toy synthetic work unit.
//
// `TERM_GEN_*` mirror `exec_core`'s private consts of the same name by value
// (not importable across the crate boundary); a change to either side should
// keep both in sync. Values went through TWO revisions from an initial
// 1000/128 guess, both caught by measurement, not assumed:
//
// 1) Throughput: at 1000 rows/batch (100 fresh `par_chunks` calls for 100k
//    rows) batched measured ~1.8x SLOWER than sequential, because a streaming
//    design can't buffer the whole result before dispatching (unlike a
//    one-shot `par_chunks` over the full dataset, which measured ~2.7x
//    faster) — each batch pays its own fresh dispatch overhead, and 1000
//    rows' worth of work didn't cover it. Sweeping found the throughput
//    break-even between 2000-5000 rows; 10 000 landed comfortably past it
//    (~1.6-1.7x faster).
// 2) Memory: `sf-bench`'s OWN `constant_memory` peak-heap invariant test (a
//    SEPARATE, independent constraint this restructure must also keep
//    passing) ruled 10 000 out — its `mem_ratio` tolerance of `4.0` measured
//    9.05 at batch=10 000 and 5.44 at batch=5000, because each buffered,
//    reconstructed row costs far more than its raw bytes (small-map
//    allocator overhead dominates, not the term data itself) — see
//    `exec_core::TERM_GEN_BATCH_SIZE`'s doc comment. 3000 is therefore the
//    memory-constrained final value, not the throughput-optimal one.

/// Row count for the batch-parallelism receipt — large enough that dispatch
/// overhead (or its absence) actually shows up in the wall-clock.
const TERM_GEN_N: usize = 100_000;

const TERM_GEN_BATCH_SIZE: usize = 4_000;
const TERM_GEN_MIN_PARALLEL_ROWS: usize = 2_000;
const TERM_GEN_MIN_CHUNK_ROWS: usize = 128;

/// A one-column source row, borrowing its value — same shape `sf-sql` result
/// rows use (a name/value lookup), minimal enough to build 100k of cheaply.
struct IdRow<'a>(&'a str);

impl sf_core::Row for IdRow<'_> {
    fn value(&self, column: &str) -> Option<&str> {
        (column == "id").then_some(self.0)
    }
}

/// An `rr:template` IRI term map (`http://example.org/item/{id}`) plus 100k
/// synthetic row values — real per-row work: percent-encoding-aware template
/// expansion into a fresh IRI, not a no-op.
fn term_gen_fixture() -> (TermMap, Vec<String>) {
    let template = Template::parse("http://example.org/item/{id}").expect("parse template");
    let term_map = TermMap::Template(template, TermSpec::iri());
    let ids: Vec<String> = (0..TERM_GEN_N).map(|i| i.to_string()).collect();
    (term_map, ids)
}

/// The baseline: term-gen inline, one row at a time, no batching — what
/// `exec_core`'s loop did before the M4 wave-2 restructure (and what a batch
/// below `TERM_GEN_MIN_PARALLEL_ROWS` still does today).
fn term_gen_sequential(term_map: &TermMap, ids: &[String]) -> Vec<Option<sf_core::Term>> {
    ids.iter()
        .map(|id| {
            sf_core::term::generate(term_map, &IdRow(id))
                .expect("term generation must not fail on a well-formed template")
        })
        .collect()
}

/// The new shape: `TERM_GEN_BATCH_SIZE`-sized batches, each parallel-mapped via
/// `par_chunks` at a `current_num_threads()`-derived chunk size (floored at
/// `TERM_GEN_MIN_CHUNK_ROWS` so a chunk never degenerates to one row per task —
/// the shape the M4 wave-2 correction note measured ~2x slower), chunks
/// collected in order and flattened — exactly `exec_core::reconstruct_batch`'s
/// algorithm, applied to `sf_core::term::generate` instead of the private
/// `reconstruct`. A batch below `TERM_GEN_MIN_PARALLEL_ROWS` skips rayon
/// entirely (a fresh `par_chunks` call's own overhead would dominate it).
fn term_gen_batched_parallel(term_map: &TermMap, ids: &[String]) -> Vec<Option<sf_core::Term>> {
    let mut out = Vec::with_capacity(ids.len());
    for batch in ids.chunks(TERM_GEN_BATCH_SIZE) {
        if batch.len() < TERM_GEN_MIN_PARALLEL_ROWS {
            out.extend(
                batch
                    .iter()
                    .map(|id| sf_core::term::generate(term_map, &IdRow(id)).expect("term-gen")),
            );
            continue;
        }
        let chunk_size =
            (batch.len() / rayon::current_num_threads().max(1)).max(TERM_GEN_MIN_CHUNK_ROWS);
        let chunked: Vec<Vec<Option<sf_core::Term>>> = batch
            .par_chunks(chunk_size)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|id| sf_core::term::generate(term_map, &IdRow(id)).expect("term-gen"))
                    .collect()
            })
            .collect();
        out.extend(chunked.into_iter().flatten());
    }
    out
}

fn bench_term_gen_batch_parallel(c: &mut Criterion) {
    let (term_map, ids) = term_gen_fixture();

    // Correctness cross-check, once, before timing: both paths must produce the
    // identical term sequence, in order (the same property
    // `batch_reconstruct_tests` locks at the `exec_core` level).
    assert_eq!(
        term_gen_sequential(&term_map, &ids),
        term_gen_batched_parallel(&term_map, &ids),
        "batched-parallel term-gen must match sequential, in order"
    );

    eprintln!(
        "\nADR-0006 M4 wave-2 term-gen batch parallelism ({TERM_GEN_N} rows, {} rayon threads, \
         batch={TERM_GEN_BATCH_SIZE}):",
        rayon::current_num_threads()
    );

    let mut group = c.benchmark_group("micro_term_gen_batch");
    group.bench_function("sequential", |b| {
        b.iter(|| term_gen_sequential(&term_map, &ids));
    });
    group.bench_function("batched_parallel", |b| {
        b.iter(|| term_gen_batched_parallel(&term_map, &ids));
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_order_mixed_kind,
    bench_distinct_agg,
    bench_group_avg_rust,
    bench_term_gen_batch_parallel
);
criterion_main!(benches);
