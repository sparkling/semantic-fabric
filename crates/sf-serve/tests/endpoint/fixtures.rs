//! Shared fixtures for the endpoint acceptance modules.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_core::query_control::QueryLimits;
use sf_serve::{router, Backend, ServeConfig, SqlitePool, DEFAULT_QUERY_LIMITS};
use tower::ServiceExt;

use crate::support;

const CREATE_SQL: &str = r#"
CREATE TABLE "People" ("id" INTEGER PRIMARY KEY, "name" TEXT, "age" INTEGER);
INSERT INTO "People" VALUES (1, 'Alice', 30), (2, 'Bob', 25);
"#;

pub(crate) const MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#People> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "People" ] ;
  rr:subjectMap [ rr:template "http://ex/person/{id}" ; rr:class ex:Person ] ;
  rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] ;
  rr:predicateObjectMap [ rr:predicate ex:age ;  rr:objectMap [ rr:column "age" ] ] .
"#;

/// Measured minimal source work admitting the 20 000-row all-names SELECT with
/// every other limit at its default; one unit less refuses. Pinned by
/// `admission::source_work_limit_alone_refuses_20000_row_stream`.
pub(crate) const ROWS_20000_MIN_SOURCE_WORK: u64 = 2_666_684;

/// Explicit finite source-work allowance for the 20 000-row pooling/locking
/// fixtures: the measured minimum plus 1/32 headroom. Serving defaults are
/// unchanged; only these fixtures opt in.
pub(crate) const FIXTURE_20000_ROW_SOURCE_WORK: u64 =
    ROWS_20000_MIN_SOURCE_WORK + ROWS_20000_MIN_SOURCE_WORK / 32;

const _: () = assert!(ROWS_20000_MIN_SOURCE_WORK > DEFAULT_QUERY_LIMITS.max_source_work());

pub(crate) fn sqlite_config() -> ServeConfig {
    sqlite_config_with_pool().0
}

pub(crate) fn sqlite_config_with_pool() -> (ServeConfig, SqlitePool) {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(CREATE_SQL).unwrap();
    let backend = Backend::sqlite(conn);
    let Backend::Sqlite(pool) = &backend else {
        unreachable!("fixture is SQLite")
    };
    let pool = pool.clone();
    (support::serve_config(backend, MAPPING_TTL), pool)
}

/// A fixture with `n` generated People rows, for exercising the bounded-memory
/// streaming path across many response chunks (one `CHUNK` is 16 KiB).
pub(crate) fn big_sqlite_config(n: usize) -> ServeConfig {
    big_sqlite_config_with_limits(n, DEFAULT_QUERY_LIMITS)
}

/// [`big_sqlite_config`] under explicit request-wide query limits.
pub(crate) fn big_sqlite_config_with_limits(n: usize, limits: QueryLimits) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE \"People\" (\"id\" INTEGER PRIMARY KEY, \"name\" TEXT, \"age\" INTEGER);
         WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < {n})
         INSERT INTO \"People\" SELECT x, 'Person' || x, x % 100 FROM c;"
    ))
    .unwrap();
    let mut cfg = support::serve_config(Backend::sqlite(conn), MAPPING_TTL);
    cfg.query_limits = limits;
    cfg
}

/// The default finite serving limits with every named limit replaced.
pub(crate) fn limits_with(
    compiler_work: u64,
    source_work: u64,
    result_items: u64,
    serialized_bytes: u64,
) -> QueryLimits {
    let limits = QueryLimits::new(compiler_work, source_work, result_items, serialized_bytes);
    limits.with_reservation_limits(DEFAULT_QUERY_LIMITS.reservation_limits())
}

/// The default finite serving limits with only source work replaced.
pub(crate) fn source_work_limits(source_work: u64) -> QueryLimits {
    let defaults = DEFAULT_QUERY_LIMITS;
    limits_with(
        defaults.max_compiler_work(),
        source_work,
        defaults.max_result_items(),
        defaults.max_serialized_bytes(),
    )
}

/// POST the all-names SELECT and report whether the streamed body completed.
/// A completed body must carry exactly `rows` bindings.
pub(crate) async fn streams_completely(cfg: ServeConfig, rows: usize) -> bool {
    let response = router(Arc::new(cfg))
        .oneshot(post_query(
            "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
            "application/sparql-results+json",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let Ok(collected) = response.into_body().collect().await else {
        return false;
    };
    let bytes = collected.to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON");
    let bindings = json["results"]["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), rows);
    true
}

/// POST a raw `application/sparql-query` body, asking for `accept`.
pub(crate) fn post_query(query: &str, accept: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, accept)
        .body(Body::from(query.to_owned()))
        .unwrap()
}

pub(crate) async fn send(
    cfg: Arc<ServeConfig>,
    req: Request<Body>,
) -> (StatusCode, String, String) {
    let resp = router(cfg).oneshot(req).await.unwrap();
    let status = resp.status();
    let ctype = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, ctype, String::from_utf8(bytes.to_vec()).unwrap())
}

/// A unique file-backed SQLite path under the OS temp dir (never `:memory:`,
/// which forces [`Backend::sqlite_pool_from_path`] to a pool of one regardless
/// of `pool_size` — see its doc). Mirrors `run.rs`'s private `temp_db_path`
/// unit-test helper; duplicated here rather than shared because an integration
/// test cannot reach a unit-test-only helper in `src/run.rs`.
pub(crate) fn unique_sqlite_path(tag: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sf_serve_endpoint_{tag}_{}_{unique}.db",
        std::process::id()
    ))
}

pub(crate) mod pg {
    use tokio_postgres::NoTls;

    pub(crate) fn base_conn() -> String {
        std::env::var("SF_PG_URL").unwrap_or_else(|_| {
            let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned());
            format!("host=localhost port=5432 user={user}")
        })
    }

    /// A bounded pool over `conn_str` sized `max_size` (mirrors
    /// `sf_serve::run::open_backend`'s PG branch — ADR-0010 §C stream-lane pool,
    /// ADR-0027, M4 wave-2 finding 2).
    pub(crate) fn pg_pool_sized(conn_str: &str, max_size: usize) -> deadpool_postgres::Pool {
        pg_pool_sized_with_wait(conn_str, max_size, None)
    }

    pub(crate) fn pg_pool_sized_with_wait(
        conn_str: &str,
        max_size: usize,
        wait_timeout: Option<std::time::Duration>,
    ) -> deadpool_postgres::Pool {
        let mut pg_config: tokio_postgres::Config = conn_str.parse().expect("valid PG conninfo");
        pg_config.options("-csearch_path=pg_catalog,public,pg_temp");
        let manager = deadpool_postgres::Manager::from_config(
            pg_config,
            NoTls,
            deadpool_postgres::ManagerConfig {
                recycling_method: deadpool_postgres::RecyclingMethod::Custom(
                    "SELECT pg_catalog.set_config( \
                         'search_path', 'pg_catalog,public,pg_temp', false \
                     )"
                    .to_owned(),
                ),
            },
        );
        deadpool_postgres::Pool::builder(manager)
            .max_size(max_size)
            .wait_timeout(wait_timeout)
            .runtime(deadpool_postgres::Runtime::Tokio1)
            .build()
            .expect("build test PG pool")
    }
}
