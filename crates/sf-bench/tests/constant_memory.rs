//! The constant-memory demonstration as a fast `cargo test` (ADR-0006, the
//! differentiator): stream a result-producing OBDA query (the CONSTRUCT dump)
//! against a **file-backed** SQLite source at growing scale factors and assert
//! the engine peak-heap high-water stays BOUNDED (≈ constant, within a small
//! fixed factor) while the streamed-triple count grows ~linearly. This shows the
//! engine working set is `O(|T| + |M| + batch)` — independent of source size:
//! the source DB does the set-work on disk, the engine just streams.
//!
//! This is the same demonstration as `benches/constant_memory.rs`, at small
//! scales so it runs in `cargo test --workspace`. The file holds a single test so
//! the process-wide allocator probe sees no cross-test interference.

use sf_bench::{driver, mem, workload};
use tempfile::TempDir;

#[global_allocator]
static GLOBAL: mem::Tracking = mem::Tracking;

/// One measured streaming run at a scale: (streamed triples, peak engine bytes).
fn measure(scale: u32) -> (u64, i64) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("gtfs.db");
    let (conn, _counts) = workload::open_source_db(&path, scale).expect("generate source");
    let maps = driver::mapping().expect("parse mapping");
    let schemas = driver::introspect(&conn).expect("introspect source");

    // Bracket the streaming window: rebaseline the high-water, stream the dump
    // through a discarding sink (bounded memory), read the window peak.
    let base = mem::reset_peak();
    let triples = driver::stream_construct_count(&maps, &conn, &schemas, workload::DUMP_QUERY)
        .expect("stream construct");
    let peak = mem::window_peak(base);
    (triples, peak)
}

#[test]
fn engine_memory_is_bounded_under_growing_source() {
    // Small, fast scales (the bench covers 1x/10x/100x).
    let scales = [1u32, 4, 16];
    let mut rows = Vec::new();
    let mut peaks = Vec::new();

    eprintln!(
        "\nADR-0006 constant-memory (test): {:>6} {:>12} {:>16}",
        "scale", "triples", "peak_engine_B"
    );
    for &s in &scales {
        let (triples, peak) = measure(s);
        eprintln!("{s:>30} {triples:>12} {peak:>16}");
        rows.push(triples);
        peaks.push(peak);
    }

    // 1) Result size grows ~linearly with scale (the workload is doing real,
    //    growing work — otherwise the memory claim would be vacuous).
    let row_ratio = rows[scales.len() - 1] as f64 / rows[0].max(1) as f64;
    assert!(
        row_ratio >= 8.0,
        "result rows must grow ~linearly with scale (16x): got {row_ratio:.1}x \
         ({} → {})",
        rows[0],
        rows[scales.len() - 1]
    );

    // 2) Engine peak heap stays bounded — grows far slower than the data. Floor
    //    each peak to damp small-allocation noise, then require the memory growth
    //    factor to be both small in absolute terms AND far below the row growth.
    let floor = 64 * 1024i64; // 64 KiB noise floor
    let eff_min = peaks.iter().copied().min().unwrap().max(floor);
    let eff_max = peaks.iter().copied().max().unwrap().max(floor);
    let mem_ratio = eff_max as f64 / eff_min as f64;

    assert!(
        mem_ratio <= 4.0,
        "engine peak heap must stay ≈ constant across scales: mem_ratio={mem_ratio:.2} \
         (peaks={peaks:?} bytes)"
    );
    assert!(
        mem_ratio <= row_ratio / 2.0,
        "engine memory must grow far slower than source data: mem_ratio={mem_ratio:.2} \
         vs row_ratio={row_ratio:.1} (peaks={peaks:?})"
    );
}
