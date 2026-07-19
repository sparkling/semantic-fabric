//! End-to-end tests for the SPARQL 1.2 Protocol endpoint (ADR-0019 G8), driven
//! in-process via `tower::ServiceExt::oneshot` (no real socket). The SQLite suite
//! runs on an in-memory fixture; the PostgreSQL variant gate-skips when no server
//! is reachable on localhost:5432.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_serve::{introspect_sqlite_all, router, Backend, ServeConfig};
use sf_sparql::Tbox;
use tower::ServiceExt;

const CREATE_SQL: &str = r#"
CREATE TABLE "People" ("id" INTEGER PRIMARY KEY, "name" TEXT, "age" INTEGER);
INSERT INTO "People" VALUES (1, 'Alice', 30), (2, 'Bob', 25);
"#;

const MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#People> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "People" ] ;
  rr:subjectMap [ rr:template "http://ex/person/{id}" ; rr:class ex:Person ] ;
  rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] ;
  rr:predicateObjectMap [ rr:predicate ex:age ;  rr:objectMap [ rr:column "age" ] ] .
"#;

fn sqlite_config() -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(CREATE_SQL).unwrap();
    let schema = introspect_sqlite_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(MAPPING_TTL).unwrap();
    ServeConfig::new(Backend::sqlite(conn), maps, Tbox::default(), schema)
}

/// A fixture with `n` generated People rows, for exercising the bounded-memory
/// streaming path across many response chunks (one `CHUNK` is 16 KiB).
fn big_sqlite_config(n: usize) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE \"People\" (\"id\" INTEGER PRIMARY KEY, \"name\" TEXT, \"age\" INTEGER);
         WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < {n})
         INSERT INTO \"People\" SELECT x, 'Person' || x, x % 100 FROM c;"
    ))
    .unwrap();
    let schema = introspect_sqlite_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(MAPPING_TTL).unwrap();
    ServeConfig::new(Backend::sqlite(conn), maps, Tbox::default(), schema)
}

/// POST a raw `application/sparql-query` body, asking for `accept`.
fn post_query(query: &str, accept: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, accept)
        .body(Body::from(query.to_owned()))
        .unwrap()
}

async fn send(cfg: Arc<ServeConfig>, req: Request<Body>) -> (StatusCode, String, String) {
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

#[tokio::test]
async fn select_returns_json_bindings() {
    let cfg = Arc::new(sqlite_config());
    let req = post_query(
        "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
        "application/sparql-results+json",
    );
    let (status, ctype, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        ctype.starts_with("application/sparql-results+json"),
        "ctype={ctype}"
    );
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let bindings = json["results"]["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), 2, "two People rows: {body}");
    let names: Vec<&str> = bindings
        .iter()
        .map(|b| b["name"]["value"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"Alice") && names.contains(&"Bob"),
        "names={names:?}"
    );
}

#[tokio::test]
async fn select_via_get_url_param() {
    let cfg = Arc::new(sqlite_config());
    let qs = form_urlencoded::Serializer::new(String::new())
        .append_pair("query", "SELECT ?age WHERE { ?s <http://ex/age> ?age }")
        .finish();
    let req = Request::builder()
        .method("GET")
        .uri(format!("/sparql?{qs}"))
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(Body::empty())
        .unwrap();
    let (status, _ctype, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["results"]["bindings"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn select_returns_xml() {
    let cfg = Arc::new(sqlite_config());
    let req = post_query(
        "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
        "application/sparql-results+xml",
    );
    let (status, ctype, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        ctype.starts_with("application/sparql-results+xml"),
        "ctype={ctype}"
    );
    assert!(body.contains("<sparql"), "xml body:\n{body}");
    assert!(
        body.contains("Alice") && body.contains("Bob"),
        "xml body:\n{body}"
    );
}

#[tokio::test]
async fn select_returns_csv() {
    let cfg = Arc::new(sqlite_config());
    let req = post_query(
        "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
        "text/csv",
    );
    let (status, ctype, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ctype.starts_with("text/csv"), "ctype={ctype}");
    // Header row + one row per binding.
    assert!(body.contains("name"), "csv body:\n{body}");
    assert!(
        body.contains("Alice") && body.contains("Bob"),
        "csv body:\n{body}"
    );
}

#[tokio::test]
async fn select_returns_tsv() {
    let cfg = Arc::new(sqlite_config());
    let req = post_query(
        "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
        "text/tab-separated-values",
    );
    let (status, ctype, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        ctype.starts_with("text/tab-separated-values"),
        "ctype={ctype}"
    );
    assert!(
        body.contains("Alice") && body.contains("Bob"),
        "tsv body:\n{body}"
    );
}

/// Many rows exercise the chunked streaming body across the 16 KiB `CHUNK`
/// boundary (the bounded-memory path), and assert every row is delivered intact.
#[tokio::test]
async fn select_streams_many_rows() {
    let n = 3000;
    let cfg = Arc::new(big_sqlite_config(n));
    let req = post_query(
        "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
        "application/sparql-results+json",
    );
    let (status, _ctype, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::OK);
    // The serialised body comfortably exceeds one CHUNK, so it streamed in pieces.
    assert!(body.len() > 16 * 1024, "body should span many chunks");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["results"]["bindings"].as_array().unwrap().len(), n);
}

#[tokio::test]
async fn ask_returns_boolean() {
    let cfg = Arc::new(sqlite_config());
    let (s1, _c, b_true) = send(
        cfg.clone(),
        post_query(
            "ASK { ?s <http://ex/name> \"Alice\" }",
            "application/sparql-results+json",
        ),
    )
    .await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&b_true).unwrap()["boolean"],
        serde_json::Value::Bool(true)
    );

    let (s2, _c, b_false) = send(
        cfg,
        post_query(
            "ASK { ?s <http://ex/name> \"Nobody\" }",
            "application/sparql-results+json",
        ),
    )
    .await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&b_false).unwrap()["boolean"],
        serde_json::Value::Bool(false)
    );
}

#[tokio::test]
async fn construct_returns_turtle() {
    let cfg = Arc::new(sqlite_config());
    let req = post_query(
        "CONSTRUCT { ?s <http://ex/label> ?name } WHERE { ?s <http://ex/name> ?name }",
        "text/turtle",
    );
    let (status, ctype, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ctype.starts_with("text/turtle"), "ctype={ctype}");
    assert!(body.contains("http://ex/label"), "turtle body:\n{body}");
    assert!(
        body.contains("Alice") && body.contains("Bob"),
        "turtle body:\n{body}"
    );
}

#[tokio::test]
async fn construct_returns_ntriples() {
    let cfg = Arc::new(sqlite_config());
    let req = post_query(
        "CONSTRUCT { ?s <http://ex/label> ?name } WHERE { ?s <http://ex/name> ?name }",
        "application/n-triples",
    );
    let (status, ctype, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ctype.starts_with("application/n-triples"), "ctype={ctype}");
    assert!(
        body.contains("<http://ex/label>"),
        "n-triples body:\n{body}"
    );
    // N-Triples writes one absolute-IRI statement per line, terminated by " .".
    assert!(
        body.lines().filter(|l| l.trim_end().ends_with('.')).count() >= 2,
        "n-triples body:\n{body}"
    );
}

#[tokio::test]
async fn construct_returns_jsonld() {
    let cfg = Arc::new(sqlite_config());
    let req = post_query(
        "CONSTRUCT { ?s <http://ex/label> ?name } WHERE { ?s <http://ex/name> ?name }",
        "application/ld+json",
    );
    let (status, ctype, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ctype.starts_with("application/ld+json"), "ctype={ctype}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.is_array() || json.is_object(), "json-ld body:\n{body}");
    assert!(
        body.contains("Alice") && body.contains("Bob"),
        "json-ld body:\n{body}"
    );
}

#[tokio::test]
async fn deferred_feature_returns_501() {
    let cfg = Arc::new(sqlite_config());
    let req = post_query(
        "SELECT * WHERE { SERVICE <http://example.org/sparql> { ?s ?p ?o } }",
        "application/sparql-results+json",
    );
    let (status, _ctype, _body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn malformed_query_returns_400() {
    let cfg = Arc::new(sqlite_config());
    let req = post_query(
        "SELECT ?x WHERE { this is not sparql",
        "application/sparql-results+json",
    );
    let (status, _ctype, _body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn missing_query_param_returns_400() {
    let cfg = Arc::new(sqlite_config());
    let req = Request::builder()
        .method("GET")
        .uri("/sparql")
        .body(Body::empty())
        .unwrap();
    let (status, _ctype, _body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---- PostgreSQL variant: gate-skips when no server on localhost:5432 ----------

mod pg {
    use super::*;
    use tokio_postgres::NoTls;

    fn base_conn() -> String {
        std::env::var("SF_PG_URL").unwrap_or_else(|_| {
            let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned());
            format!("host=localhost port=5432 user={user}")
        })
    }

    /// A bounded pool over `conn_str` sized `max_size` (mirrors
    /// `sf_serve::run::open_backend`'s PG branch — ADR-0010 §C stream-lane pool,
    /// ADR-0027, M4 wave-2 finding 2).
    fn pg_pool_sized(conn_str: &str, max_size: usize) -> deadpool_postgres::Pool {
        let pg_config: tokio_postgres::Config = conn_str.parse().expect("valid PG conninfo");
        let manager = deadpool_postgres::Manager::new(pg_config, NoTls);
        deadpool_postgres::Pool::builder(manager)
            .max_size(max_size)
            .runtime(deadpool_postgres::Runtime::Tokio1)
            .build()
            .expect("build test PG pool")
    }

    fn pg_pool(conn_str: &str) -> deadpool_postgres::Pool {
        pg_pool_sized(conn_str, 16)
    }

    #[tokio::test]
    async fn pg_select_and_construct() {
        let conn_str = base_conn();
        // A plain admin connection drives DDL/seed/cleanup; the endpoint itself is
        // exercised over a real pool (`Backend::Pg` is a `deadpool_postgres::Pool`,
        // not a single shared client).
        let Ok((client, connection)) = tokio_postgres::connect(&conn_str, NoTls).await else {
            eprintln!("SKIP pg_select_and_construct: no PostgreSQL on localhost:5432");
            return;
        };
        tokio::spawn(async move {
            let _ = connection.await;
        });
        // Isolated table (unique per process) so the run is self-cleaning.
        let table = format!("sf_serve_people_{}", std::process::id());
        // Two named rows plus a bulk tail, so the async PG streamer (select_each_pg
        // / construct_each_pg) is exercised across many response chunks.
        let n = 3000;
        client
            .batch_execute(&format!(
                "DROP TABLE IF EXISTS \"{table}\"; \
                 CREATE TABLE \"{table}\" (\"id\" INTEGER PRIMARY KEY, \"name\" TEXT, \"age\" INTEGER); \
                 INSERT INTO \"{table}\" VALUES (1, 'Alice', 30), (2, 'Bob', 25); \
                 INSERT INTO \"{table}\" SELECT g, 'Person' || g, g % 100 FROM generate_series(3, {n}) AS g;"
            ))
            .await
            .unwrap();

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
        let maps = sf_mapping::parse_r2rml(&mapping_ttl).unwrap();
        let schema = sf_serve::introspect_pg_all(&client).await.unwrap();
        // Same conninfo as the admin connection (no dbname override) so the pool's
        // connections see the table the admin connection just created.
        let pool = pg_pool(&conn_str);
        let cfg = Arc::new(ServeConfig::new(
            Backend::Pg(pool),
            maps,
            Tbox::default(),
            schema,
        ));

        let (s_sel, _c, body) = send(
            cfg.clone(),
            post_query(
                "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
                "application/sparql-results+json",
            ),
        )
        .await;
        assert_eq!(s_sel, StatusCode::OK, "{body}");
        assert!(
            body.len() > 16 * 1024,
            "PG SELECT body should span many chunks"
        );
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["results"]["bindings"].as_array().unwrap().len(), n);

        let (s_con, ctype, turtle) = send(
            cfg,
            post_query(
                "CONSTRUCT { ?s <http://ex/name> ?name } WHERE { ?s <http://ex/name> ?name }",
                "text/turtle",
            ),
        )
        .await;
        assert_eq!(s_con, StatusCode::OK);
        assert!(ctype.starts_with("text/turtle"));
        assert!(turtle.contains("Alice"), "{turtle}");

        // Best-effort cleanup.
        let _ = client
            .batch_execute(&format!("DROP TABLE IF EXISTS \"{table}\""))
            .await;
    }

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
        const ROWS: i64 = 300_000;
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
        let maps = sf_mapping::parse_r2rml(&mapping_ttl).unwrap();
        let schema = sf_serve::introspect_pg_all(&client).await.unwrap();

        const N: usize = 16;

        /// Fire `n` concurrent SELECTs at a fresh endpoint over `pool`, asserting
        /// every response is a complete `200`, and return the wall-clock elapsed.
        async fn run_concurrent(
            pool: deadpool_postgres::Pool,
            maps: Vec<sf_core::ir::TriplesMap>,
            schema: Vec<sf_sql::TableSchema>,
            n: usize,
            rows: i64,
        ) -> std::time::Duration {
            let cfg = Arc::new(ServeConfig::new(
                Backend::Pg(pool),
                maps,
                Tbox::default(),
                schema,
            ));
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
        let single_conn_elapsed =
            run_concurrent(pool1, maps.clone(), schema.clone(), N, ROWS).await;

        let pool16 = pg_pool_sized(&conn_str, 16);
        let pooled_elapsed = run_concurrent(pool16, maps, schema, N, ROWS).await;

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

    /// The ADR-0010 §C admission-control addition ([`sf_serve::acquire_pg`], not
    /// public — exercised only through the endpoint): a `max_size=1` pool with a
    /// short wait timeout sheds a second concurrent request as `503` +
    /// `Retry-After` while the first still holds the only connection, rather than
    /// queueing it indefinitely or failing with a generic `500`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pg_pool_exhaustion_sheds_503_with_retry_after() {
        let conn_str = base_conn();
        let Ok((client, connection)) = tokio_postgres::connect(&conn_str, NoTls).await else {
            eprintln!(
                "SKIP pg_pool_exhaustion_sheds_503_with_retry_after: \
                 no PostgreSQL on localhost:5432"
            );
            return;
        };
        tokio::spawn(async move {
            let _ = connection.await;
        });

        let table = format!("sf_serve_pool_503_{}", std::process::id());
        const ROWS: i64 = 300_000;
        client
            .batch_execute(&format!(
                "DROP TABLE IF EXISTS \"{table}\"; \
                 CREATE TABLE \"{table}\" (\"id\" INTEGER PRIMARY KEY, \"name\" TEXT); \
                 INSERT INTO \"{table}\" SELECT g, 'Person' || g \
                 FROM generate_series(1, {ROWS}) AS g;"
            ))
            .await
            .expect("seed table");

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
        let maps = sf_mapping::parse_r2rml(&mapping_ttl).unwrap();
        let schema = sf_serve::introspect_pg_all(&client).await.unwrap();

        // One connection, a short wait: a second concurrent request has no
        // chance of acquiring one before the first (a slow full-table dump)
        // finishes.
        let pg_config: tokio_postgres::Config = conn_str.parse().unwrap();
        let manager = deadpool_postgres::Manager::new(pg_config, NoTls);
        let pool = deadpool_postgres::Pool::builder(manager)
            .max_size(1)
            .wait_timeout(Some(std::time::Duration::from_millis(50)))
            .runtime(deadpool_postgres::Runtime::Tokio1)
            .build()
            .unwrap();
        let cfg = Arc::new(ServeConfig::new(
            Backend::Pg(pool),
            maps,
            Tbox::default(),
            schema,
        ));

        let cfg1 = cfg.clone();
        let first = tokio::spawn(async move {
            router(cfg1)
                .oneshot(post_query(
                    "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
                    "application/sparql-results+json",
                ))
                .await
                .unwrap()
        });
        // Give the first request a head start so it actually holds the pool's
        // one connection before the second competes for it.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let resp2 = router(cfg.clone())
            .oneshot(post_query(
                "SELECT ?name WHERE { ?s <http://ex/name> ?name }",
                "application/sparql-results+json",
            ))
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            resp2.headers().get(header::RETRY_AFTER).unwrap(),
            "1",
            "shed response must carry Retry-After (ADR-0010 §C)"
        );
        let body2 = String::from_utf8(
            resp2
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(body2.contains("exhausted"), "{body2}");

        let resp1 = first.await.expect("first request task join");
        assert_eq!(
            resp1.status(),
            StatusCode::OK,
            "the request that held the connection must still succeed"
        );
        // Drop the body WITHOUT draining it (cancel-on-drop, ADR-0010 §C) instead
        // of streaming all 300k rows: the background streaming task otherwise
        // blocks forever on channel backpressure once its buffer fills, holding
        // the size-1 pool's one connection and deadlocking the DROP TABLE cleanup
        // below against the still-open cursor's table lock (mirrors
        // `mysql_release.rs`'s drop-the-body release check).
        drop(resp1);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let _ = client
            .batch_execute(&format!("DROP TABLE IF EXISTS \"{table}\""))
            .await;
    }
}

// ---- SQLite concurrent-reader pool: file-backed, no gate-skip -------------

/// A unique file-backed SQLite path under the OS temp dir (never `:memory:`,
/// which forces [`Backend::sqlite_pool_from_path`] to a pool of one regardless
/// of `pool_size` — see its doc). Mirrors `run.rs`'s private `temp_db_path`
/// unit-test helper; duplicated here rather than shared because an integration
/// test (this file) cannot reach a unit-test-only helper in `src/run.rs`.
fn unique_sqlite_path(tag: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sf_serve_endpoint_{tag}_{}_{unique}.db",
        std::process::id()
    ))
}

/// F2c SQLite-pool RECEIPT (ADR-0010 status-correction part 2, mirrors
/// `pg::pg_pool_concurrency_receipt` above): N=16 concurrent SELECT requests
/// over a `pool_size=1` [`Backend::sqlite_pool_from_path`] pool (the OLD
/// single-`Mutex` shape every file-backed SQLite source got before this pass —
/// exactly what `Backend::sqlite`'s pool-of-one still gives `:memory:`) vs the
/// real `pool_size=4` default. Correctness: all 32 responses (16 old-shape +
/// 16 new-shape) must be `200 OK` carrying the full SELECT payload — pooling
/// must never lose or corrupt data, only parallelise the reads. No gate-skip:
/// unlike the PG receipts, this needs no external server.
///
/// `worker_threads = 20` (> N), NOT this file's usual 8: `SqliteOwnedBackend::
/// column_names` (`sf-sql/src/backend/sqlite.rs`) takes its `std::sync::Mutex`
/// lock INLINE in an `async fn`, unlike its sibling `open_branch` which does the
/// equivalent lock only inside a dedicated `spawn_blocking` thread. Under
/// `pool_size=1` + high concurrency this can wedge every core worker thread
/// simultaneously (each blocked entering `column_names` for a different
/// request) while the one connection-holding request — parked on its own
/// `spawn_blocking` thread — can never get its result channel drained, since
/// no worker thread remains free to poll the draining task: a genuine,
/// pre-existing deadlock, confirmed via `sample`(1) at `N=16, worker_threads=8`
/// (hangs indefinitely) vs `N=8` (completes; provably safe since `N - 1 <
/// worker_threads` bounds the worst case at one free worker). Out of this
/// wave's file scope to fix (`sf-sql`, not `sf-serve`); tracked as a follow-up.
/// Sizing this test's OWN runtime above `N` sidesteps it without masking it —
/// the pool_size=1-vs-4 comparison below is unaffected by the worker count, and
/// production's default (unbounded `Builder::new_multi_thread()`, no explicit
/// `worker_threads`) remains exposed to it until fixed upstream.
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

    /// Fire `n` concurrent SELECTs at a fresh endpoint over a `pool_size`-sized
    /// SQLite pool opened on `path`, asserting every response is a complete
    /// `200` carrying all `rows` bindings, and return the wall-clock elapsed.
    async fn run_concurrent(
        path: &std::path::Path,
        pool_size: usize,
        n: usize,
        rows: usize,
    ) -> std::time::Duration {
        let maps = sf_mapping::parse_r2rml(MAPPING_TTL).unwrap();
        let (backend, schema) = Backend::sqlite_pool_from_path(path.to_str().unwrap(), pool_size)
            .expect("open sqlite pool");
        let cfg = Arc::new(ServeConfig::new(backend, maps, Tbox::default(), schema));
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

// --- Round-2 coverage: 6 previously-untested HTTP branches -----------------

#[tokio::test]
async fn post_form_urlencoded_body_is_accepted() {
    let cfg = Arc::new(sqlite_config());
    let query = "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?p ex:name ?n }";
    let encoded = form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>();
    let req = Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(Body::from(format!("query={encoded}")))
        .unwrap();
    let (status, _, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Alice") || body.contains("Bob"));
}

#[tokio::test]
async fn post_unsupported_content_type_returns_415() {
    let cfg = Arc::new(sqlite_config());
    let req = Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from("SELECT * WHERE { ?s ?p ?o }"))
        .unwrap();
    let (status, _, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert!(body.contains("unsupported Content-Type"));
}

#[tokio::test]
async fn oversized_query_returns_413() {
    let mut cfg = sqlite_config();
    cfg.max_query_len = 16; // shrink the cap well below any real query
    let cfg = Arc::new(cfg);
    let req = post_query(
        "PREFIX ex: <http://ex/> SELECT ?n WHERE { ?p ex:name ?n }",
        "application/sparql-results+json",
    );
    let (status, _, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(body.contains("exceeds the 16-byte cap"));
}

#[tokio::test]
async fn post_sparql_query_body_non_utf8_returns_400() {
    let cfg = Arc::new(sqlite_config());
    let req = Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .body(Body::from(vec![0xff, 0xfe, 0x00, 0x01])) // invalid UTF-8
        .unwrap();
    let (status, _, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("not valid UTF-8"));
}

#[tokio::test]
async fn ask_query_exceeding_timeout_returns_504() {
    // Deterministic (not machine-speed-dependent): `Backend::Sqlite` guards its
    // connection with a plain blocking `std::sync::Mutex` (confirmed in
    // `sf_serve::lib`). Hold that lock from a background OS thread for longer
    // than `cfg.timeout` — `ask_sqlite_owned` genuinely blocks trying to
    // acquire it, so `tokio::time::timeout` wrapping the ASK task reliably
    // elapses before any response is sent (ASK collects a single boolean,
    // unlike a streamed SELECT/CONSTRUCT whose 200 status line commits before
    // any deadline is ever checked mid-body — why ASK is the clean way to
    // reach 504 here). A workload-dependent "make the SQL itself slow" query
    // is inherently flaky across machines; this is not.
    let mut cfg = sqlite_config();
    cfg.timeout = std::time::Duration::from_millis(20);
    let Backend::Sqlite(pool) = &cfg.backend else {
        unreachable!()
    };
    let conn = pool.pick();
    let hold = std::thread::spawn(move || {
        let _guard = conn.lock().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
    });
    // Give the background thread a moment to actually acquire the lock before
    // firing the request (avoids a race where the request's lock attempt wins).
    std::thread::sleep(std::time::Duration::from_millis(20));
    let cfg = Arc::new(cfg);
    let req = post_query("ASK { ?s ?p ?o }", "application/sparql-results+json");
    let (status, _, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert!(body.contains("request timeout"));
    hold.join().unwrap();
}

#[tokio::test]
async fn ask_query_backend_sql_error_returns_500() {
    // A genuine runtime SQL failure (not a compile-time / translate-time one):
    // the ServeConfig's `schema` still declares the "age" column (so translate
    // succeeds), but the LIVE connection's table no longer has it — a schema
    // drift between compile-time metadata and the actual DB state, exactly the
    // shape `SparqlError::Sql` -> 500 exists to cover. Cleaner than any hack:
    // it's a real, reachable production scenario (concurrent DDL change).
    let cfg = sqlite_config();
    if let Backend::Sqlite(pool) = &cfg.backend {
        pool.pick()
            .lock()
            .unwrap()
            .execute_batch("ALTER TABLE \"People\" RENAME COLUMN \"age\" TO \"age_renamed\";")
            .unwrap();
    }
    let cfg = Arc::new(cfg);
    let req = post_query(
        "PREFIX ex: <http://ex/> ASK { ?p ex:age ?a }",
        "application/sparql-results+json",
    );
    let (status, _, body) = send(cfg, req).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body.is_empty());
}
