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
/// schemas.
///
/// **Process isolation (fixture-collision fix).** Each bench process creates its
/// own private database `sf_bench_<pid>` and runs the whole fixture there. Two
/// concurrent `cargo bench` invocations therefore get distinct databases and can
/// never touch each other's tables — the previous design created the six GTFS
/// tables in the *shared* scratch DB (`dbname=$USER`) under global names and tore
/// them down with a global `DROP TABLE`, so a second run's setup/teardown would
/// yank tables out from under a live run (→ `relation "TRIPS" does not exist`
/// crashes and distorted timings). A per-process schema would not have sufficed:
/// `introspect_postgres` (sf-sql) scopes its `information_schema` lookups by table
/// name only, so a sibling process's same-named tables in another schema would
/// double-count columns. A per-process **database** isolates the catalog cleanly
/// without touching sf-sql. `Drop` only ever targets THIS process's own DB.
struct PgFixture {
    rt: tokio::runtime::Runtime,
    client: tokio_postgres::Client,
    /// `host=… port=… user=…` (no `dbname`) — used to reach the default scratch DB
    /// for the admin `CREATE DATABASE` / `DROP DATABASE` of our private DB.
    admin_conn_str: String,
    /// This process's private database name (`sf_bench_<pid>`).
    db_name: String,
    maps: Vec<TriplesMap>,
    schemas: Vec<TableSchema>,
    counts: workload::RowCounts,
}

/// Connect to PG, spawning the connection driver task on the current runtime.
async fn pg_connect(conn_str: &str) -> Result<tokio_postgres::Client, tokio_postgres::Error> {
    let (client, conn) = tokio_postgres::connect(conn_str, NoTls).await?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    Ok(client)
}

impl PgFixture {
    /// Connect, create this process's private DB, populate the GTFS tables at
    /// `scale`, introspect. Returns `None` if PG is unreachable (groups skipped).
    fn new(scale: u32) -> Option<Self> {
        let admin_conn_str = pg_conn_str();
        let db_name = format!("sf_bench_{}", std::process::id());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio rt");

        // 1. Admin-connect to the default scratch DB and (re)create our private DB.
        //    `WITH (FORCE)` clears any stale same-pid DB left by a crashed prior run.
        let setup = rt.block_on(async {
            let admin = pg_connect(&admin_conn_str).await?;
            let _ = admin
                .batch_execute(&format!(
                    r#"DROP DATABASE IF EXISTS "{db_name}" WITH (FORCE)"#
                ))
                .await;
            admin
                .batch_execute(&format!(r#"CREATE DATABASE "{db_name}""#))
                .await?;
            Ok::<_, tokio_postgres::Error>(())
        });
        if let Err(e) = setup {
            eprintln!("\n[bench] PostgreSQL unavailable ({e}) — skipping PG shootout groups");
            return None;
        }

        // 2. Connect to the private DB and build the fixture inside it. The trailing
        //    `dbname=` keyword overrides any earlier value (libpq last-wins).
        let fixture_conn_str = format!("{admin_conn_str} dbname={db_name}");
        let client = match rt.block_on(pg_connect(&fixture_conn_str)) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "\n[bench] could not open private DB {db_name} ({e}) — skipping PG groups"
                );
                return None;
            }
        };
        let counts = rt
            .block_on(async {
                client.batch_execute(workload::PG_SCHEMA_SQL).await?;
                workload::generate_pg(&client, scale).await
            })
            .expect("PG fixture setup");
        let schemas = rt
            .block_on(driver::introspect_pg_all(&client))
            .expect("introspect PG");
        let maps = driver::mapping().expect("parse mapping");
        eprintln!(
            "\n[bench] live PG fixture @{scale}x loaded (db={db_name}): \
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
            admin_conn_str,
            db_name,
            maps,
            schemas,
            counts,
        })
    }
}

impl Drop for PgFixture {
    fn drop(&mut self) {
        let _ = self.counts.total();
        let admin_conn_str = self.admin_conn_str.clone();
        let db_name = self.db_name.clone();
        // Drop ONLY this process's private DB. `WITH (FORCE)` terminates our own
        // still-open fixture connection so the drop succeeds; a concurrent run owns
        // a different `sf_bench_<pid>` and is never touched.
        let _ = self.rt.block_on(async move {
            let admin = pg_connect(&admin_conn_str).await?;
            admin
                .batch_execute(&format!(
                    r#"DROP DATABASE IF EXISTS "{db_name}" WITH (FORCE)"#
                ))
                .await?;
            Ok::<_, tokio_postgres::Error>(())
        });
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
