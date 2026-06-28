//! Constant-memory demonstration bench (ADR-0006, the differentiator): stream the
//! result-producing CONSTRUCT dump over a file-backed source at 1x/10x/100x and
//! report the engine peak-heap high-water alongside the streamed-triple count.
//! Result rows grow ~linearly with scale while the engine working set stays
//! bounded — `O(|T| + |M| + batch)`, independent of source size.
//!
//! This bench installs [`sf_bench::mem::Tracking`] as the global allocator so it
//! can report the peak; the hard *assertion* lives in `tests/constant_memory.rs`
//! (so it also runs under `cargo test --workspace`). Run `cargo bench -p sf-bench`.

use std::sync::Once;

use criterion::{criterion_group, criterion_main, Criterion};
use sf_bench::{driver, mem, workload};
use tempfile::TempDir;

#[global_allocator]
static GLOBAL: mem::Tracking = mem::Tracking;

static REPORT: Once = Once::new();

/// One-shot peak-heap table (printed once, before the timed runs).
fn report_peak_table() {
    eprintln!("\nADR-0006 constant-memory demonstration (streamed CONSTRUCT dump):");
    eprintln!(
        "{:>6}  {:>14}  {:>16}  {:>14}",
        "scale", "triples", "peak_engine_B", "bytes/triple"
    );
    for scale in [1u32, 10, 100] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gtfs.db");
        let (conn, _counts) = workload::open_source_db(&path, scale).unwrap();
        let maps = driver::mapping().unwrap();
        let schemas = driver::introspect(&conn).unwrap();

        let base = mem::reset_peak();
        let triples = driver::stream_construct_count(&maps, &conn, &schemas, workload::DUMP_QUERY).unwrap();
        let peak = mem::window_peak(base);

        let per = if triples > 0 {
            peak as f64 / triples as f64
        } else {
            0.0
        };
        eprintln!("{scale:>6}  {triples:>14}  {peak:>16}  {per:>14.3}");
    }
    eprintln!();
}

fn bench_streamed_dump(c: &mut Criterion) {
    REPORT.call_once(report_peak_table);

    let mut group = c.benchmark_group("constant_memory_dump");
    for scale in [1u32, 10, 100] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("gtfs.db");
        let (conn, _counts) = workload::open_source_db(&path, scale).unwrap();
        let maps = driver::mapping().unwrap();
        let schemas = driver::introspect(&conn).unwrap();
        group.bench_function(format!("stream_{scale}x"), |b| {
            b.iter(|| {
                driver::stream_construct_count(&maps, &conn, &schemas, workload::DUMP_QUERY).unwrap()
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_streamed_dump);
criterion_main!(benches);
