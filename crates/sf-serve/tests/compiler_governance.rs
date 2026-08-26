//! Public-boundary receipts for planner resource governance.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_serve::{introspect_sqlite_all, router, Backend, ServeConfig};
use sf_sparql::Tbox;
use tower::ServiceExt;

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.com/> .

<#People> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "people" ] ;
  rr:subjectMap [ rr:template "http://example.com/person/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:name ;
    rr:objectMap [ rr:column "name" ]
  ] .
"#;

fn config() -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE people(id INTEGER PRIMARY KEY, name TEXT); \
         INSERT INTO people VALUES (1, 'Alice');",
    )
    .unwrap();
    let schema = introspect_sqlite_all(&conn).unwrap();
    let maps = sf_mapping::parse_r2rml(MAPPING).unwrap();
    ServeConfig::new(Backend::sqlite(conn), maps, Tbox::default(), schema)
}

#[tokio::test]
async fn expansion_budget_rejects_query_before_execution() {
    let mut config = config();
    config.max_compile_expansion_work = 0;
    let request = Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(Body::from(
            "SELECT ?person WHERE { ?person <http://example.com/name> ?name }",
        ))
        .unwrap();

    let response = router(Arc::new(config)).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(
        body.contains("pre-allocation memory-safety budget"),
        "{body}"
    );
}
