//! `sf-bench` — performance benchmark driver (ADR-0005): the GTFS-Madrid-Bench
//! OBDA / query-rewriting track. SOTA target (ADR-0006): match or beat Ontop
//! query latency while holding constant engine memory and bounded first-result
//! latency under growing source data (the streaming invariant). Materialisation
//! benchmarks (KROWN) do not apply (ADR-0002).
//!
//! Three pieces wire ADR-0005/0006 into runnable form:
//!
//! * [`workload`] — the GTFS R2RML mapping, the five representative OBDA queries,
//!   and the scalable, **file-backed** SQLite data generator (source data on
//!   disk, off the engine heap).
//! * [`driver`] — parse the mapping once, then run a query through the live
//!   virtualizer (`sf-sparql`) over the source: latency (full + first-result) on
//!   the SELECT path, and a **streaming, bounded-memory** CONSTRUCT path.
//! * [`mem`] — a process-wide heap high-water probe (the byte-valued sibling of
//!   `sf-core`'s alloc-count probe) used by the constant-memory demonstration.
//!
//! `criterion` benches (`benches/`) drive latency; the constant-memory invariant
//! is also a fast `cargo test` (`tests/constant_memory.rs`) so it runs in
//! `cargo test --workspace`.

pub mod driver;
pub mod mem;
pub mod workload;

use std::time::{SystemTime, UNIX_EPOCH};

use sf_core::Result;

pub use workload::RowCounts;

/// A benchmark scenario (e.g. GTFS-Madrid scale factor 100).
#[derive(Debug, Clone)]
pub struct Scenario {
    pub name: String,
    pub scale_factor: u32,
}

impl Scenario {
    /// A named GTFS-Madrid scenario at the given scale factor.
    pub fn new(name: impl Into<String>, scale_factor: u32) -> Self {
        Self {
            name: name.into(),
            scale_factor,
        }
    }
}

/// Run a GTFS-Madrid OBDA scenario end to end: generate the dataset into a
/// file-backed source, parse the mapping, then execute every representative query
/// plus the streaming CONSTRUCT — live SPARQL→SQL over the relational source, no
/// materialisation (ADR-0005/0006). A smoke driver over the full path; the
/// quantitative latency/memory numbers come from the `criterion` benches and the
/// constant-memory test. The temp source DB is removed on completion.
pub fn run_obda_scenario(scenario: &Scenario) -> Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "sf-bench-{}-{nanos}.db",
        scenario.scale_factor
    ));

    let result = (|| -> driver::DResult<()> {
        let (conn, _counts) = workload::open_source_db(&path, scenario.scale_factor)?;
        let maps = driver::mapping()?;
        let schemas = driver::introspect(&conn)?;
        for (_name, sparql) in workload::queries() {
            let _ = driver::run_select(&maps, &conn, &schemas, sparql)?;
        }
        let _ = driver::stream_construct_count(&maps, &conn, &schemas, workload::DUMP_QUERY)?;
        Ok(())
    })();

    let _ = std::fs::remove_file(&path);
    result.map_err(|e| sf_core::Error::Mapping(format!("OBDA scenario failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_1x_runs_end_to_end() {
        run_obda_scenario(&Scenario::new("gtfs-madrid-1x", 1)).expect("1x scenario runs");
    }
}
