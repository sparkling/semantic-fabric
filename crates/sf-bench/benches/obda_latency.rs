//! GTFS-Madrid OBDA latency benches (ADR-0005): per-query wall-clock on the five
//! representative SELECT queries, plus first-result + total latency on the
//! streaming CONSTRUCT dump, at 1x and 10x. Live SPARQL→SQL over a file-backed
//! SQLite source — no materialisation (ADR-0006).
//!
//! Ontop is the optional, offline JVM cross-check (ADR-0005); it is deliberately
//! NOT a dependency here. Run `cargo bench -p sf-bench`.

use std::sync::Once;

use criterion::{criterion_group, criterion_main, Criterion};
use rusqlite::Connection;
use sf_bench::{driver, workload};
use sf_core::ir::TriplesMap;
use sf_sql::TableSchema;
use tempfile::TempDir;

static FIRST_RESULT: Once = Once::new();

/// A generated, file-backed source plus the parsed mapping, held for a bench run.
struct Fixture {
    _dir: TempDir,
    conn: Connection,
    maps: Vec<TriplesMap>,
    schemas: Vec<TableSchema>,
    counts: workload::RowCounts,
}

fn fixture(scale: u32) -> Fixture {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join(format!("gtfs-{scale}x.db"));
    let (conn, counts) = workload::open_source_db(&path, scale).expect("generate source");
    let maps = driver::mapping().expect("parse mapping");
    let schemas = driver::introspect(&conn).expect("introspect source");
    Fixture {
        _dir: dir,
        conn,
        maps,
        schemas,
        counts,
    }
}

fn bench_select_queries(c: &mut Criterion) {
    let fx = fixture(1);
    let mut group = c.benchmark_group("obda_select_1x");
    for (name, sparql) in workload::queries() {
        group.bench_function(name, |b| {
            b.iter(|| driver::run_select(&fx.maps, &fx.conn, &fx.schemas, sparql).unwrap());
        });
    }
    group.finish();
}

/// M7 tree-path benchmark: same five queries through the operator-tree (IQ) path.
/// Paired with `obda_select_1x` to compare flat vs tree translation + plan shape.
fn bench_select_queries_tree(c: &mut Criterion) {
    let fx = fixture(1);
    let mut group = c.benchmark_group("obda_select_tree_1x");
    for (name, sparql) in workload::queries() {
        group.bench_function(name, |b| {
            b.iter(|| driver::run_select_tree(&fx.maps, &fx.conn, &fx.schemas, sparql).unwrap());
        });
    }
    group.finish();
}

/// First-result latency must be measured from the value `stream_construct_timed`
/// captures at the first produced triple — criterion times the whole closure, so
/// it cannot time a partial stream. Reported once as a table (the
/// bounded-first-result claim, ADR-0006); criterion below times the full dump.
fn report_first_result_table() {
    eprintln!("\nADR-0006 first-result vs total latency (streamed CONSTRUCT dump):");
    eprintln!(
        "{:>6} {:>14} {:>18} {:>14}",
        "scale", "triples", "first_result_µs", "total_ms"
    );
    for scale in [1u32, 10, 100] {
        let fx = fixture(scale);
        let (n, first, total) =
            driver::stream_construct_timed(&fx.maps, &fx.conn, &fx.schemas, workload::DUMP_QUERY)
                .unwrap();
        eprintln!(
            "{scale:>6} {n:>14} {:>18.1} {:>14.3}",
            first.as_secs_f64() * 1e6,
            total.as_secs_f64() * 1e3
        );
    }
    eprintln!();
}

fn bench_construct_dump(c: &mut Criterion) {
    FIRST_RESULT.call_once(report_first_result_table);

    let mut group = c.benchmark_group("obda_construct_dump");
    for scale in [1u32, 10] {
        let fx = fixture(scale);
        // Full streamed dump (all triples), bounded memory.
        group.bench_function(format!("full_dump_{scale}x"), |b| {
            b.iter(|| {
                driver::stream_construct_count(
                    &fx.maps,
                    &fx.conn,
                    &fx.schemas,
                    workload::DUMP_QUERY,
                )
                .unwrap()
            });
        });
        let _ = fx.counts.total();
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_select_queries,
    bench_select_queries_tree,
    bench_construct_dump
);
criterion_main!(benches);
