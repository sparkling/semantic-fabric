//! Public, opt-in result provenance; no external database is accessed.
use std::sync::Arc;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::Value;
use sf_serve::{router, Backend, BearerQueryAdmission, QueryAdmission, ServeConfig};
use tower::ServiceExt;

#[path = "lineage/graph.rs"]
mod graph;
#[path = "lineage/more.rs"]
mod more;
mod support;

const FORMAT: &str = "application/vnd.semantic-fabric.lineage+json-seq";
const TOKEN: &str = "test-only-lineage-credential-123456789";
const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<http://example.test/People> a rr:TriplesMap ;
 rr:logicalTable [ rr:tableName "people" ] ;
 rr:subjectMap [ rr:template "http://example.test/person/{id}" ] ;
 rr:predicateObjectMap [ rr:predicate <http://example.test/name> ;
 rr:objectMap [ rr:column "name" ] ] .
"#;

fn config() -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE people(id INTEGER PRIMARY KEY, name TEXT); \
        INSERT INTO people VALUES(1,'Alice'),(2,'Alice'),(3,'Bob');",
    )
    .unwrap();
    let mut cfg = support::serve_config(Backend::sqlite(conn), MAPPING);
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    cfg
}

fn request(query: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, FORMAT)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::from(query.to_owned()))
        .unwrap()
}

async fn records(cfg: Arc<ServeConfig>, query: &str) -> Vec<Value> {
    let response = router(cfg).oneshot(request(query)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], FORMAT);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(!text.contains(TOKEN));
    text.split('\u{1e}')
        .filter(|part| !part.is_empty())
        .map(|part| serde_json::from_str(part).unwrap())
        .collect()
}

#[tokio::test]
async fn lineage_preserves_projected_bags_and_never_invents_primary_key_identity() {
    let rows = records(
        Arc::new(config()),
        "SELECT ?name WHERE { ?s <http://example.test/name> ?name }",
    )
    .await;
    assert_eq!(rows.len(), 5, "header, three actual solutions, completion");
    assert_eq!(rows[0]["mappingId"], "http://example.test/People");
    assert_eq!(rows[0]["sourceId"], 0);
    let mut values = vec![];
    for (ordinal, row) in rows[1..4].iter().enumerate() {
        assert_eq!(row["ordinal"], ordinal);
        values.push(
            row["result"]["results"]["bindings"][0]["name"]["value"]
                .as_str()
                .unwrap(),
        );
        assert_eq!(row["provenance"]["@type"], "prov:Bundle");
        let prov = row["provenance"].to_string();
        assert!(prov.contains("prov:Activity") && prov.contains("prov:used"));
        assert!(!prov.contains("rowKey") && !prov.contains("wasDerivedFrom"));
        assert!(!prov.contains("Alice") && !prov.contains("Bob"));
        let quads = oxjsonld::JsonLdParser::new()
            .for_slice(prov.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .expect("bundle is valid JSON-LD/RDF");
        assert!(quads
            .iter()
            .any(|quad| quad.predicate.as_str() == "http://www.w3.org/ns/prov#used"));
        assert!(quads
            .iter()
            .any(|quad| quad.predicate.as_str() == "urn:semantic-fabric:lineage:mappingId"));
    }
    values.sort();
    assert_eq!(values, ["Alice", "Alice", "Bob"]);
    assert_eq!(
        rows[4],
        serde_json::json!({"type":"complete","solutions":3})
    );
}

#[tokio::test]
async fn lineage_only_describes_final_solutions_after_distinct_and_slice() {
    let rows = records(
        Arc::new(config()),
        "SELECT DISTINCT ?name WHERE { ?s <http://example.test/name> ?name } LIMIT 1 OFFSET 1",
    )
    .await;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1]["ordinal"], 0);
    assert_eq!(rows[2]["solutions"], 1);
}

#[tokio::test]
async fn empty_result_has_no_fabricated_provenance_activity() {
    let rows = records(
        Arc::new(config()),
        "SELECT ?name WHERE { <http://example.test/person/999> <http://example.test/name> ?name }",
    )
    .await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["solutions"], 0);
    assert!(!serde_json::to_string(&rows)
        .unwrap()
        .contains("prov:Activity"));
}

#[tokio::test]
async fn rejects_unproved_operators_and_ambiguous_negotiation_instead_of_fabricating_origins() {
    let cfg = Arc::new(config());
    for query in [
        "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://example.test/name> ?name }",
        "SELECT * WHERE { VALUES ?name { \"Alice\" } }",
        "SELECT * WHERE { ?s <http://example.test/name> ?n OPTIONAL { ?x <http://example.test/name> ?y } }",
        "SELECT * WHERE { ?s <http://example.test/name> ?n FILTER(?n = \"Alice\") }",
        "ASK { ?s <http://example.test/name> ?n }",
        "CONSTRUCT { ?s <http://example.test/name> ?n } WHERE { ?s <http://example.test/name> ?n FILTER(?n = \"Alice\") }",
    ] {
        let response = router(cfg.clone()).oneshot(request(query)).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED, "{query}");
    }
    for (first, second) in [
        (FORMAT, "application/sparql-results+json"),
        ("application/sparql-results+json", FORMAT),
        (FORMAT, FORMAT),
        (
            "application/vnd.semantic-fabric.lineage+json-seq;q=0",
            "application/json",
        ),
    ] {
        let mut req = request("SELECT ?n WHERE { ?s <http://example.test/name> ?n }");
        req.headers_mut()
            .insert(header::ACCEPT, first.parse().unwrap());
        req.headers_mut()
            .append(header::ACCEPT, second.parse().unwrap());
        assert_eq!(
            router(cfg.clone()).oneshot(req).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn lineage_requires_the_same_authentication_and_preserves_ordinary_format() {
    let cfg = Arc::new(config());
    let query = "SELECT ?name WHERE { ?s <http://example.test/name> ?name }";
    let mut unauthorized = request(query);
    unauthorized.headers_mut().remove(header::AUTHORIZATION);
    assert_eq!(
        router(cfg.clone())
            .oneshot(unauthorized)
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let mut standard = request(query);
    standard.headers_mut().insert(
        header::ACCEPT,
        "application/sparql-results+json".parse().unwrap(),
    );
    let response = router(cfg).oneshot(standard).await.unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["results"]["bindings"].as_array().unwrap().len(), 3);
    assert!(json.get("provenance").is_none());
}

#[tokio::test]
async fn independent_runtime_snapshots_differ_but_cache_hits_keep_identity() {
    let cfg = Arc::new(config());
    let query = "SELECT ?name WHERE { ?s <http://example.test/name> ?name }";
    let first = records(cfg.clone(), query).await;
    let cached = records(cfg, query).await;
    assert_eq!(first[0], cached[0]);
    assert_ne!(
        first[1]["provenance"]["@id"],
        cached[1]["provenance"]["@id"]
    );
    let independent = records(Arc::new(config()), query).await;
    assert_ne!(first[0]["snapshot"], independent[0]["snapshot"]);
    assert_ne!(first[0]["logicalPlan"], independent[0]["logicalPlan"]);
}

#[tokio::test]
async fn every_provenance_byte_including_completion_obeys_the_response_budget() {
    let query = "SELECT ?name WHERE { ?s <http://example.test/name> ?name }";
    let response = router(Arc::new(config()))
        .oneshot(request(query))
        .await
        .unwrap();
    let size = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .len() as u64;
    for (limit, succeeds) in [(size, true), (size - 1, false), (32, false)] {
        let mut cfg = config();
        cfg.query_limits = sf_core::query_control::QueryLimits::new(10000, 10000, 10000, limit);
        let response = router(Arc::new(cfg)).oneshot(request(query)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result = response.into_body().collect().await;
        assert_eq!(result.is_ok(), succeeds, "byte limit {limit}");
        if let Err(error) = result {
            assert_eq!(error.to_string(), "result stream failed");
        }
    }
}
