//! Result-format negotiation, streaming, and PostgreSQL streaming tests.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sf_serve::{IntrospectedSource, ServeConfig};

use crate::fixtures::{big_sqlite_config, post_query, send, sqlite_config};
use crate::support;

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

// ---- PostgreSQL variant: gate-skips when no server on localhost:5432 ----------

mod pg {
    use super::*;
    use tokio_postgres::NoTls;

    use crate::fixtures::pg::{base_conn, pg_pool_sized};

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
        let ontology = support::ontology_for_mapping(&maps);
        // Same conninfo as the admin connection (no dbname override) so the pool's
        // connections see the table the admin connection just created.
        let pool = pg_pool_sized(&conn_str, 1);
        {
            let shadow = pool.get().await.expect("acquire hostile fixture session");
            shadow
                .batch_execute(&format!(
                    "CREATE TEMP TABLE \"{table}\" (\"id\" INTEGER PRIMARY KEY, \
                     \"name\" TEXT, \"age\" INTEGER); \
                     INSERT INTO pg_temp.\"{table}\" \
                     VALUES (999999, 'HOSTILE_SHADOW', 999999);"
                ))
                .await
                .expect("seed same-name temporary shadow relation");
        }
        let source = IntrospectedSource::observe_postgres(pool).await.unwrap();
        let mut cfg = ServeConfig::from_authored_r2rml(source, &mapping_ttl, ontology).unwrap();
        cfg.set_parser_runtime(support::parser_runtime());
        cfg.set_query_admission(sf_serve::QueryAdmission::UnrestrictedDevelopment);
        let cfg = Arc::new(cfg);

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
        assert!(!body.contains("HOSTILE_SHADOW"));

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
