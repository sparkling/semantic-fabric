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

    #[tokio::test]
    async fn pg_select_and_construct() {
        let conn_str = base_conn();
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
        let client = Arc::new(client);
        let cfg = Arc::new(ServeConfig::new(
            Backend::Pg(client.clone()),
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
    let Backend::Sqlite(conn) = &cfg.backend else {
        unreachable!()
    };
    let conn = conn.clone();
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
    if let Backend::Sqlite(conn) = &cfg.backend {
        conn.lock()
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
