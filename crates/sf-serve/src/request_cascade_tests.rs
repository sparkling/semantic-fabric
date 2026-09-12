//! Public results complement pass-level rewrite proofs. Serving quarantines
//! unverified constraints, so these tests do not assert that FK elimination fires.
use crate::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<urn:child-map> a rr:TriplesMap;
  rr:logicalTable [rr:tableName "child"];
  rr:subjectMap [rr:template "urn:child:{id}"];
  rr:predicateObjectMap [rr:predicate <urn:parent>;
    rr:objectMap [rr:template "urn:parent:{b}/{a}"]].
<urn:parent-map> a rr:TriplesMap;
  rr:logicalTable [rr:tableName "parent"];
  rr:subjectMap [rr:template "urn:parent:{a}/{b}"];
  rr:predicateObjectMap [rr:predicate <urn:name>;
    rr:objectMap [rr:column "name"]].
"#;
const TOKEN: &str = "test-only-cascade-service-principal-123456";

fn fixture(secured: bool) -> Arc<ServeConfig> {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON;
        CREATE TABLE parent(a TEXT NOT NULL,b TEXT NOT NULL,name TEXT,PRIMARY KEY(a,b));
        CREATE TABLE child(id INTEGER PRIMARY KEY,a TEXT NOT NULL,b TEXT NOT NULL,
          FOREIGN KEY(b,a) REFERENCES parent(a,b));
        INSERT INTO parent VALUES('P','Q','drop'),('R','S','keep');
        INSERT INTO child VALUES(1,'Q','P'),(2,'S','R');",
        )
        .unwrap();
    let source = IntrospectedSource::observe_sqlite(Backend::sqlite(connection)).unwrap();
    let ontology = crate::test_support::ontology(&[], &["urn:parent", "urn:name"]);
    let mut config = ServeConfig::from_authored_r2rml(source, MAPPING, ontology).unwrap();
    // This unit fixture targets compiler/executor results, not parser isolation.
    config.use_in_process_test_parser();
    config.set_query_admission(if secured {
        QueryAdmission::Bearer(BearerQueryAdmission::for_service_principal(TOKEN).unwrap())
    } else {
        QueryAdmission::UnrestrictedDevelopment
    });
    Arc::new(config)
}

async fn rows(config: Arc<ServeConfig>, query: &str) -> Vec<serde_json::Value> {
    let request = Request::post("/sparql")
        .header("content-type", "application/sparql-query")
        .header("accept", "application/sparql-results+json")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::from(query.to_owned()))
        .unwrap();
    let response = router(config).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    body["results"]["bindings"].as_array().unwrap().clone()
}

#[tokio::test]
async fn composite_parent_identity_and_filtered_optional_survive_cold_and_warm_http() {
    for secured in [false, true] {
        let config = fixture(secured);
        // The bounded-order profile must not admit source-sized global sorting.
        let unbounded = Request::post("/sparql")
            .header("content-type", "application/sparql-query")
            .header("authorization", format!("Bearer {TOKEN}"))
            .body(Body::from(
                "SELECT ?child WHERE { ?child <urn:parent> ?parent } ORDER BY ?child",
            ))
            .unwrap();
        assert_eq!(
            router(config.clone())
                .oneshot(unbounded)
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_IMPLEMENTED
        );
        // LIMIT 10 is the admitted ordering window, not a reduction in expected
        // results: both rows in the fixture must still be returned exactly.
        for _ in 0..2 {
            let joined = rows(
                config.clone(),
                "SELECT ?child ?parent ?name WHERE {
                ?child <urn:parent> ?parent . ?parent <urn:name> ?name
            } ORDER BY ?child LIMIT 10",
            )
            .await;
            assert_eq!(joined.len(), 2);
            assert_eq!(joined[0]["parent"]["value"], "urn:parent:P/Q");
            assert_eq!(joined[1]["parent"]["value"], "urn:parent:R/S");
            let optional = rows(
                config.clone(),
                "SELECT ?child ?parent ?name WHERE {
                ?child <urn:parent> ?parent . OPTIONAL {
                    ?parent <urn:name> ?name . FILTER(?name = 'keep')
                }
            } ORDER BY ?child LIMIT 10",
            )
            .await;
            assert_eq!(
                optional.len(),
                2,
                "filter must not remove the preserved child row"
            );
            assert_eq!(optional[0]["child"]["value"], "urn:child:1");
            assert!(optional[0].get("name").is_none());
            assert_eq!(optional[1]["name"]["value"], "keep");
        }
    }
}
