//! Pool and lock concurrency acceptance tests (SQLite and PostgreSQL).

use std::sync::Arc;

use axum::http::StatusCode;
use sf_core::query_control::QueryLimits;
use sf_serve::{Backend, IntrospectedSource, ServeConfig};

use crate::fixtures::{big_sqlite_config_with_limits, post_query, send, source_work_limits};
use crate::fixtures::{unique_sqlite_path, FIXTURE_20000_ROW_SOURCE_WORK, MAPPING_TTL};
use crate::support;

/// F2c SQLite-pool RECEIPT (ADR-0010 status-correction part 2, mirrors
/// `pg::pg_pool_concurrency_receipt` below): N=16 concurrent SELECT requests
/// over a `pool_size=1` [`Backend::sqlite_pool_from_path`] pool (the OLD
/// single-`Mutex` shape every file-backed SQLite source got before this pass —
/// exactly what `Backend::sqlite`'s pool-of-one still gives `:memory:`) vs the
/// real `pool_size=4` default. Correctness: all 32 responses (16 old-shape +
/// 16 new-shape) must be `200 OK` carrying the full SELECT payload — pooling
/// must never lose or corrupt data, only parallelise the reads. No gate-skip:
/// unlike the PG receipts, this needs no external server.
///
/// `worker_threads = 20` (> N), NOT this file's usual 8: `SqliteOwnedBackend::
/// column_names` (`sf-sql/src/backend/sqlite.rs`) USED TO take its
/// `std::sync::Mutex` lock INLINE in an `async fn`, unlike its sibling
/// `open_branch` which does the equivalent lock only inside a dedicated
/// `spawn_blocking` thread. Under `pool_size=1` + high concurrency this could
/// wedge every core worker thread simultaneously (each blocked entering
/// `column_names` for a different request) while the one connection-holding
/// request — parked on its own `spawn_blocking` thread — could never get its
/// result channel drained, since no worker thread remained free to poll the
/// draining task: a genuine, pre-existing deadlock, confirmed via `sample`(1) at
/// `N=16, worker_threads=8` (hung indefinitely) vs `N=8` (completed; provably
/// safe since `N - 1 < worker_threads` bounds the worst case at one free
/// worker). FIXED (F2d): `column_names` now locks inside `spawn_blocking` like
/// `open_branch`, verified by the dedicated `column_names_spawn_blocking_
/// deadlock_regression` test below. This test's `worker_threads = 20` sizing
/// stays regardless — its purpose is the pool_size=1-vs-4 throughput
/// comparison below, not the deadlock, and de-risking it against worker count
/// costs nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 20)]
async fn sqlite_pool_concurrency_receipt() {
    let path = unique_sqlite_path("pool_receipt");
    const ROWS: usize = 20_000;
    {
        // A plain read-write connection seeds the fixture; run_concurrent below
        // reopens it read-only through the real pool constructor under test.
        let conn = rusqlite::Connection::open(&path).expect("create fixture db");
        conn.execute_batch(&format!(
            "CREATE TABLE \"People\" (\"id\" INTEGER PRIMARY KEY, \"name\" TEXT, \"age\" INTEGER);
             WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < {ROWS})
             INSERT INTO \"People\" SELECT x, 'Person' || x, x % 100 FROM c;"
        ))
        .expect("seed fixture db");
    }

    const N: usize = 16;

    /// A `pool_size`-sized SQLite pool opened on `path` under `limits`.
    fn open_pool_config(
        path: &std::path::Path,
        pool_size: usize,
        limits: QueryLimits,
    ) -> ServeConfig {
        let (backend, _schema) = Backend::sqlite_pool_from_path(path.to_str().unwrap(), pool_size)
            .expect("open sqlite pool");
        let mut cfg = support::serve_config(backend, MAPPING_TTL);
        cfg.query_limits = limits;
        cfg
    }

    /// Fire `n` concurrent SELECTs at a fresh endpoint over a `pool_size`-sized
    /// SQLite pool opened on `path`, asserting every response is a complete
    /// `200` carrying all `rows` bindings, and return the wall-clock elapsed.
    async fn run_concurrent(
        path: &std::path::Path,
        pool_size: usize,
        n: usize,
        rows: usize,
    ) -> std::time::Duration {
        // The 20 000-row stream exceeds the default source-work limit; this
        // explicit finite allowance is pinned by the admission.rs regression.
        let limits = source_work_limits(FIXTURE_20000_ROW_SOURCE_WORK);
        let cfg = Arc::new(open_pool_config(path, pool_size, limits));
        let start = std::time::Instant::now();
        let mut handles = Vec::with_capacity(n);
        for _ in 0..n {
            let cfg = cfg.clone();
            handles.push(tokio::spawn(async move {
                send(
                    cfg,
                    post_query(
                        "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
                        "application/sparql-results+json",
                    ),
                )
                .await
            }));
        }
        for h in handles {
            let (status, _ctype, body) = h.await.expect("request task join");
            assert_eq!(status, StatusCode::OK, "{body}");
            let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
            assert_eq!(
                json["results"]["bindings"].as_array().unwrap().len(),
                rows,
                "every concurrent response must carry the full result set"
            );
        }
        start.elapsed()
    }

    let pool1_elapsed = run_concurrent(&path, 1, N, ROWS).await;
    let pool4_elapsed = run_concurrent(&path, 4, N, ROWS).await;

    eprintln!(
        "SQLite pool concurrency ({N} concurrent SELECTs, {ROWS} rows each): \
         pool_size=1 (OLD-equivalent)={pool1_elapsed:?} pool_size=4 (NEW default)={pool4_elapsed:?}"
    );
    assert!(
        pool4_elapsed < pool1_elapsed,
        "a 4-connection read pool should beat a single connection under 16-way \
         concurrency: pool_size=1={pool1_elapsed:?} pool_size=4={pool4_elapsed:?}"
    );

    let _ = std::fs::remove_file(&path);
}

/// F2d regression (P0 deadlock, RED-first): reproduces the mechanism documented
/// on [`sqlite_pool_concurrency_receipt`] directly, at half that test's scale
/// (`worker_threads = 4`, `N = 8` — the same 2x-oversubscription ratio
/// empirically confirmed to hang at `worker_threads = 8`/`N = 16`). Every one of
/// `N` concurrent SELECTs over a pool-of-1 SQLite backend calls `column_names`
/// (the `run_branches` catalog probe, `sf-sparql/src/exec_core.rs`) before its
/// `open_branch`, so a big enough result set gives the first winner's
/// `spawn_blocking` cursor thread long enough to hold the connection `Mutex`
/// while the other `N - 1` callers pile into `column_names`'s lock — wedging
/// every worker thread if that lock is taken inline instead of inside its own
/// `spawn_blocking`. Wrapped in a 30s `tokio::time::timeout` so a pre-fix
/// regression is a clean test FAILURE, not a hung CI job: confirmed RED (times
/// out) against the unfixed adapter, GREEN (completes in well under a second)
/// against the fix in `sf-sql/src/backend/sqlite.rs`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn column_names_spawn_blocking_deadlock_regression() {
    const N: usize = 8;
    const ROWS: usize = 20_000;
    // The 20 000-row stream exceeds the default source-work limit; this
    // explicit finite allowance is pinned by the admission.rs regression.
    let limits = source_work_limits(FIXTURE_20000_ROW_SOURCE_WORK);
    let cfg = Arc::new(big_sqlite_config_with_limits(ROWS, limits));

    let run = async {
        let mut handles = Vec::with_capacity(N);
        for _ in 0..N {
            let cfg = cfg.clone();
            handles.push(tokio::spawn(async move {
                send(
                    cfg,
                    post_query(
                        "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
                        "application/sparql-results+json",
                    ),
                )
                .await
            }));
        }
        for h in handles {
            let (status, _ctype, body) = h.await.expect("request task join");
            assert_eq!(status, StatusCode::OK, "{body}");
        }
    };

    tokio::time::timeout(std::time::Duration::from_secs(30), run)
        .await
        .expect(
            "column_names deadlock regression: N=8 concurrent SELECTs over a \
             pool-of-1 SQLite backend hung past 30s under worker_threads=4 — \
             SqliteOwnedBackend::column_names is locking its Mutex inline again",
        );
}

// ---- PostgreSQL variant: gate-skips when no server on localhost:5432 ----------

mod pg {
    use super::*;
    use tokio_postgres::NoTls;

    use crate::fixtures::pg::{base_conn, pg_pool_sized};

    /// M4 wave-2 finding 2 RECEIPT: N=16 concurrent SELECT requests over a
    /// `max_size=1` pool (behaviourally identical to the OLD design — every
    /// request serialises through the one PG connection a `max_size=1` pool
    /// hands out) vs the real `max_size=16` pool (ADR-0010 §C stream-lane pool).
    /// A pool-size comparison isolates the one variable under test instead of
    /// diffing across the whole lib.rs change. Correctness: all 32 responses
    /// (16 old-shape + 16 new-shape) must be `200 OK` with the full 16-row
    /// SELECT payload — pooling must not lose or corrupt data, only parallelise
    /// the connection.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn pg_pool_concurrency_receipt() {
        let conn_str = base_conn();
        let Ok((client, connection)) = tokio_postgres::connect(&conn_str, NoTls).await else {
            eprintln!("SKIP pg_pool_concurrency_receipt: no PostgreSQL on localhost:5432");
            return;
        };
        tokio::spawn(async move {
            let _ = connection.await;
        });

        let table = format!("sf_serve_pool_receipt_{}", std::process::id());
        const ROWS: i64 = 30_000;
        client
            .batch_execute(&format!(
                "DROP TABLE IF EXISTS \"{table}\"; \
                 CREATE TABLE \"{table}\" (\"id\" INTEGER PRIMARY KEY, \"name\" TEXT); \
                 INSERT INTO \"{table}\" SELECT g, 'Person' || g \
                 FROM generate_series(1, {ROWS}) AS g;"
            ))
            .await
            .expect("seed big table for concurrency receipt");

        let mapping_ttl = format!(
            r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#People> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "{table}" ] ;
  rr:subjectMap [ rr:template "http://ex/person/{{id}}" ; rr:class ex:Person ] ;
  rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] .
"#
        );
        const N: usize = 16;

        /// Fire `n` concurrent SELECTs at a fresh endpoint over `pool`, asserting
        /// every response is a complete `200`, and return the wall-clock elapsed.
        async fn run_concurrent(
            pool: deadpool_postgres::Pool,
            mapping_ttl: &str,
            n: usize,
            rows: i64,
        ) -> std::time::Duration {
            let maps = sf_mapping::parse_r2rml(mapping_ttl).unwrap();
            let ontology = support::ontology_for_mapping(&maps);
            let source = IntrospectedSource::observe_postgres(pool).await.unwrap();
            let mut cfg = ServeConfig::from_authored_r2rml(source, mapping_ttl, ontology).unwrap();
            cfg.set_parser_runtime(support::parser_runtime());
            cfg.set_query_admission(sf_serve::QueryAdmission::UnrestrictedDevelopment);
            let cfg = Arc::new(cfg);
            let start = std::time::Instant::now();
            let mut handles = Vec::with_capacity(n);
            for _ in 0..n {
                let cfg = cfg.clone();
                handles.push(tokio::spawn(async move {
                    send(
                        cfg,
                        post_query(
                            "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
                            "application/sparql-results+json",
                        ),
                    )
                    .await
                }));
            }
            for h in handles {
                let (status, _ctype, body) = h.await.expect("request task join");
                assert_eq!(status, StatusCode::OK, "{body}");
                let json: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
                assert_eq!(
                    json["results"]["bindings"].as_array().unwrap().len(),
                    rows as usize,
                    "every concurrent response must carry the full result set"
                );
            }
            start.elapsed()
        }

        let pool1 = pg_pool_sized(&conn_str, 1);
        let single_conn_elapsed = run_concurrent(pool1, &mapping_ttl, N, ROWS).await;

        let pool16 = pg_pool_sized(&conn_str, 16);
        let pooled_elapsed = run_concurrent(pool16, &mapping_ttl, N, ROWS).await;

        eprintln!(
            "PG pool concurrency ({N} concurrent SELECTs, {ROWS} rows each): \
             max_size=1 (OLD-equivalent)={single_conn_elapsed:?} max_size=16 (NEW)={pooled_elapsed:?}"
        );
        assert!(
            pooled_elapsed < single_conn_elapsed,
            "a 16-connection pool should beat a single connection under 16-way \
             concurrency: max_size=1={single_conn_elapsed:?} max_size=16={pooled_elapsed:?}"
        );

        let _ = client
            .batch_execute(&format!("DROP TABLE IF EXISTS \"{table}\""))
            .await;
    }
}
