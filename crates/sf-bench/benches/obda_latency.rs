//! GTFS-Madrid OBDA latency benches (ADR-0005): per-query wall-clock on the five
//! representative SELECT queries, plus first-result + total latency on the
//! streaming CONSTRUCT dump, at 1x and 10x. Live SPARQL→SQL over a file-backed
//! SQLite source — no materialisation (ADR-0006).
//!
//! The `obda_select_pg_flat_1x` / `obda_select_pg_tree_1x` groups (ADR-0023
//! shootout) run against a **live PostgreSQL server** (`localhost:5432`, trust
//! auth, user=$USER). If no server is reachable the groups are skipped with a
//! warning — they do not fail the bench run.
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
use tokio_postgres::NoTls;

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

/// Resolve the PG connection string: `SF_PG_URL` env var if set, else the
/// local trust-auth default (`host=localhost port=5432 user=$USER`).
fn pg_conn_str() -> String {
    std::env::var("SF_PG_URL").unwrap_or_else(|_| {
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned());
        format!("host=localhost port=5432 user={user}")
    })
}

/// PostgreSQL bench fixture. Holds a live client + the parsed mapping + introspected
/// schemas. The six GTFS tables are created in the scratch DB for this run and
/// torn down in `Drop` (best-effort).
struct PgFixture {
    rt: tokio::runtime::Runtime,
    client: tokio_postgres::Client,
    maps: Vec<TriplesMap>,
    schemas: Vec<TableSchema>,
    counts: workload::RowCounts,
}

impl PgFixture {
    /// Connect, create + populate the GTFS tables at `scale`, introspect.
    /// Returns `None` if PG is unreachable (bench groups are skipped).
    fn new(scale: u32) -> Option<Self> {
        let conn_str = pg_conn_str();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio rt");
        let client = match rt.block_on(async {
            let (client, conn) = tokio_postgres::connect(&conn_str, NoTls).await?;
            tokio::spawn(async move {
                let _ = conn.await;
            });
            Ok::<_, tokio_postgres::Error>(client)
        }) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("\n[bench] PostgreSQL unavailable ({e}) — skipping PG shootout groups");
                return None;
            }
        };
        let counts = rt
            .block_on(async {
                // Idempotent schema setup: drop-if-exists + recreate.
                client.batch_execute(workload::PG_SCHEMA_SQL).await?;
                workload::generate_pg(&client, scale).await
            })
            .expect("PG fixture setup");
        let schemas = rt
            .block_on(driver::introspect_pg_all(&client))
            .expect("introspect PG");
        let maps = driver::mapping().expect("parse mapping");
        eprintln!(
            "\n[bench] live PG fixture @{scale}x loaded ({conn_str}): \
             AGENCY {} CALENDAR {} ROUTES {} STOPS {} TRIPS {} STOP_TIMES {} = {} rows",
            counts.agency,
            counts.calendar,
            counts.routes,
            counts.stops,
            counts.trips,
            counts.stop_times,
            counts.total(),
        );
        Some(PgFixture {
            rt,
            client,
            maps,
            schemas,
            counts,
        })
    }
}

impl Drop for PgFixture {
    fn drop(&mut self) {
        let _ = self.counts.total();
        let _ = self
            .rt
            .block_on(async { self.client.batch_execute(workload::PG_DROP_SQL).await });
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

/// ADR-0023 shootout: flat unfold path baseline, explicit oracle arm.
/// After the M8 default flip, `run_select` routes through the tree path.
/// This group pins the flat path via `run_select_flat` so the shootout comparison
/// reflects two genuinely different translation pipelines.
fn bench_select_queries_flat(c: &mut Criterion) {
    let fx = fixture(1);
    let mut group = c.benchmark_group("obda_select_flat_1x");
    for (name, sparql) in workload::queries() {
        group.bench_function(name, |b| {
            b.iter(|| driver::run_select_flat(&fx.maps, &fx.conn, &fx.schemas, sparql).unwrap());
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

/// ADR-0023 PG shootout — flat unfold path on live PostgreSQL @1x.
/// Skipped gracefully if PG is unreachable.
fn bench_select_queries_pg_flat(c: &mut Criterion) {
    let Some(fx) = PgFixture::new(1) else { return };
    let mut group = c.benchmark_group("obda_select_pg_flat_1x");
    for (name, sparql) in workload::queries() {
        group.bench_function(name, |b| {
            b.iter(|| {
                fx.rt
                    .block_on(driver::run_select_pg_flat(
                        &fx.maps,
                        &fx.client,
                        &fx.schemas,
                        sparql,
                    ))
                    .unwrap()
            });
        });
    }
    group.finish();
}

/// ADR-0023 PG shootout — operator-tree (IQ) path on live PostgreSQL @1x.
/// Skipped gracefully if PG is unreachable. Paired with `obda_select_pg_flat_1x`
/// for a true flat-vs-tree comparison on a live PG source.
fn bench_select_queries_pg_tree(c: &mut Criterion) {
    let Some(fx) = PgFixture::new(1) else { return };
    let mut group = c.benchmark_group("obda_select_pg_tree_1x");
    for (name, sparql) in workload::queries() {
        group.bench_function(name, |b| {
            b.iter(|| {
                fx.rt
                    .block_on(driver::run_select_pg(
                        &fx.maps,
                        &fx.client,
                        &fx.schemas,
                        sparql,
                    ))
                    .unwrap()
            });
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
    bench_select_queries_flat,
    bench_select_queries_tree,
    bench_select_queries_pg_flat,
    bench_select_queries_pg_tree,
    bench_construct_dump
);
criterion_main!(benches);
