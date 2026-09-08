//! End-to-end receipts for request-wide query-budget policy.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_core::query_control::QueryLimits;
use sf_serve::{router, Backend, BearerQueryAdmission, QueryAdmission, ServeConfig};
use tower::ServiceExt;

#[path = "query_budget/graph_inventory.rs"]
mod graph_inventory;
#[path = "query_budget/null_terms.rs"]
mod null_terms;
#[path = "query_budget/ordinary_identity.rs"]
mod ordinary_identity;
#[path = "query_budget/path_identity.rs"]
mod path_identity;
mod support;

const TOKEN: &str = "test-only-compiler-budget-credential-123456";
const SELECT: &str = "SELECT ?value WHERE { ?item <http://example.test/value> ?value }";
const CLONING: &str =
    "SELECT ?x WHERE { VALUES ?x { 1 2 3 } FILTER EXISTS { VALUES ?inside { 7 } } }";
const PRODUCTS: &str = "SELECT ?a ?b ?c WHERE { VALUES ?a { 0 1 2 3 } \
                       VALUES ?b { 0 1 2 3 } VALUES ?c { 0 1 2 3 } }";

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subjectMap [ rr:template "http://example.test/item/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:value ;
    rr:objectMap [ rr:column "value" ]
  ] .
"#;

fn mapping_product_config(work: u64) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE items (id INTEGER PRIMARY KEY); INSERT INTO items VALUES (1);",
    )
    .unwrap();
    let mapping = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subject ex:item ;
  rr:predicateObjectMap [ rr:predicate ex:a, ex:b ; rr:object "one", "two", "three" ] .
"#;
    let mut cfg = support::serve_config(Backend::sqlite(conn), mapping);
    cfg.query_limits = QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX);
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    cfg
}

#[tokio::test]
async fn mapping_products_charge_even_candidates_that_cannot_match() {
    let query = "SELECT ?s ?o WHERE { ?s <http://example.test/absent> ?o }";
    let response = router(Arc::new(mapping_product_config(query.len() as u64 + 5)))
        .oneshot(authenticated(query))
        .await
        .unwrap();
    assert_budget_problem(response).await;
    let response = router(Arc::new(mapping_product_config(100_000)))
        .oneshot(authenticated(query))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["results"]["bindings"], serde_json::json!([]));
}

#[tokio::test]
async fn mapping_products_preserve_all_six_exact_triples() {
    let query = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";
    let response = router(Arc::new(mapping_product_config(100_000)))
        .oneshot(authenticated(query))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut triples: Vec<_> = json["results"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| ["s", "p", "o"].map(|v| row[v]["value"].as_str().unwrap().to_owned()))
        .collect();
    triples.sort();
    let mut expected: Vec<_> = ["a", "b"]
        .into_iter()
        .flat_map(|p| {
            ["one", "two", "three"].map(|o| {
                [
                    "http://example.test/item".to_owned(),
                    format!("http://example.test/{p}"),
                    o.to_owned(),
                ]
            })
        })
        .collect();
    expected.sort();
    assert_eq!(triples, expected);
}

fn config(limits: QueryLimits) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().expect("open fixture");
    conn.execute_batch(
        "CREATE TABLE items (id INTEGER PRIMARY KEY, value TEXT NOT NULL); \
         INSERT INTO items VALUES (1, 'one'), (2, 'two');",
    )
    .expect("seed fixture");
    let mut config = support::serve_config(Backend::sqlite(conn), MAPPING);
    config.query_limits = limits;
    config
}

fn path_config(work: u64) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE edges (s INTEGER, o INTEGER); INSERT INTO edges VALUES (1, 2);",
    )
    .unwrap();
    let mapping = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Edges> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "edges" ] ;
  rr:subjectMap [ rr:template "http://example.test/node/{s}" ] ;
  rr:predicateObjectMap [ rr:predicate ex:a, ex:b, ex:c, ex:d, ex:e, ex:f ;
    rr:objectMap [ rr:template "http://example.test/node/{o}" ] ] .
"#;
    let mut cfg = support::serve_config(Backend::sqlite(conn), mapping);
    cfg.query_limits = QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX);
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    cfg
}

#[tokio::test]
async fn negated_path_mapping_searches_obey_compiler_allowance() {
    let query = "SELECT ?s ?o WHERE { ?s !<urn:absent> ?o }";
    let response = router(Arc::new(path_config(query.len() as u64)))
        .oneshot(authenticated(query))
        .await
        .unwrap();
    assert_budget_problem(response).await;
}

#[tokio::test]
async fn negated_path_preserves_six_exact_duplicate_pairs() {
    let query = "SELECT ?s ?o WHERE { ?s !<urn:absent> ?o }";
    let response = router(Arc::new(path_config(100_000)))
        .oneshot(authenticated(query))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let pair = serde_json::json!({
        "s": {"type": "uri", "value": "http://example.test/node/1"},
        "o": {"type": "uri", "value": "http://example.test/node/2"}
    });
    assert_eq!(
        json["results"]["bindings"],
        serde_json::json!(vec![pair; 6])
    );
}

fn request(query: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(Body::from(query.to_owned()))
        .expect("static request")
}

async fn route(limits: QueryLimits, query: &str) -> axum::response::Response {
    router(Arc::new(config(limits)))
        .oneshot(request(query))
        .await
        .expect("route request")
}

async fn assert_budget_problem(response: axum::response::Response) {
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/problem+json"
    );
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect problem")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).expect("problem JSON");
    assert_eq!(json["code"], "query-budget-exceeded");
    assert_eq!(json["status"], 429);
    let text = std::str::from_utf8(&body).unwrap();
    for private in [TOKEN, "example.test", "SELECT", "items"] {
        assert!(!text.contains(private));
    }
}

fn authenticated(query: &str) -> Request<Body> {
    let mut req = request(query);
    req.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().unwrap(),
    );
    req
}

fn protected(work: u64) -> ServeConfig {
    let mut cfg = config(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX));
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    cfg
}

async fn assert_values(response: axum::response::Response) {
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut values: Vec<_> = json["results"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["value"]["value"].as_str().unwrap())
        .collect();
    values.sort();
    assert_eq!(values, ["one", "two"]);
}

#[tokio::test]
async fn authenticated_cold_and_warm_cache_obey_compiler_allowance() {
    let mut cfg = Arc::new(protected(10_000));
    // The same immutable runtime/cache survives all requests and limit changes.
    for warm in [false, true] {
        if warm {
            Arc::get_mut(&mut cfg).unwrap().query_limits =
                QueryLimits::new(SELECT.len() as u64, u64::MAX, u64::MAX, u64::MAX);
        }
        assert_values(
            router(cfg.clone())
                .oneshot(authenticated(SELECT))
                .await
                .unwrap(),
        )
        .await;
    }
    Arc::get_mut(&mut cfg)
        .expect("response released configuration")
        .query_limits = QueryLimits::new(0, u64::MAX, u64::MAX, u64::MAX);
    assert_budget_problem(router(cfg).oneshot(authenticated(SELECT)).await.unwrap()).await;
    assert_budget_problem(
        router(Arc::new(protected(0)))
            .oneshot(authenticated(SELECT))
            .await
            .unwrap(),
    )
    .await;
}

#[tokio::test]
async fn compiler_input_allowance_counts_decoded_utf8_not_form_encoding() {
    // A single VALUES leaf has no branch product: isolate the decoded-input floor.
    let query = "SELECT ?value WHERE { VALUES ?value { \"one\" \"two\" } } # café";
    let wire = form_urlencoded::Serializer::new(String::new())
        .append_pair("query", query)
        .finish();
    for method in ["GET", "POST"] {
        for (work, accepted) in [(query.len() as u64 - 1, false), (query.len() as u64, true)] {
            let req = Request::builder()
                .method(method)
                .uri(if method == "GET" {
                    format!("/sparql?{wire}")
                } else {
                    "/sparql".into()
                })
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(if method == "GET" {
                    Body::empty()
                } else {
                    Body::from(wire.clone())
                })
                .unwrap();
            let response = router(Arc::new(protected(work)))
                .oneshot(req)
                .await
                .unwrap();
            if accepted {
                assert_values(response).await;
            } else {
                assert_budget_problem(response).await;
            }
        }
    }
}

#[tokio::test]
async fn authentication_precedes_zero_compiler_allowance() {
    let response = router(Arc::new(protected(0)))
        .oneshot(request(SELECT))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn all_query_forms_and_lineage_reject_zero_compiler_allowance() {
    for query in [
        SELECT,
        "ASK { ?s ?p ?o }",
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
        "DESCRIBE <http://example.test/item/1>",
    ] {
        for accept in ["*/*", "application/vnd.semantic-fabric.lineage+json-seq"] {
            let mut req = authenticated(query);
            req.headers_mut()
                .insert(header::ACCEPT, accept.parse().unwrap());
            assert_budget_problem(router(Arc::new(protected(0))).oneshot(req).await.unwrap()).await;
        }
    }
}

#[tokio::test]
async fn zero_compiler_work_is_a_pre_response_429() {
    let response = route(
        QueryLimits::new(0, u64::MAX, u64::MAX, u64::MAX),
        "SELECT ?value WHERE { ?item <http://example.test/value> ?value }",
    )
    .await;
    assert_budget_problem(response).await;
}

#[tokio::test]
async fn compiler_clone_work_cannot_spend_only_its_input_allowance() {
    let query = CLONING;
    let response = router(Arc::new(protected(query.len() as u64)))
        .oneshot(authenticated(query))
        .await
        .unwrap();
    assert_budget_problem(response).await;
}

#[tokio::test]
async fn compiler_products_cannot_spend_only_their_input_allowance() {
    let response = router(Arc::new(protected(PRODUCTS.len() as u64)))
        .oneshot(authenticated(PRODUCTS))
        .await
        .unwrap();
    assert_budget_problem(response).await;
}

#[tokio::test]
async fn compiler_products_preserve_every_exact_public_tuple() {
    let response = router(Arc::new(protected(100_000)))
        .oneshot(authenticated(PRODUCTS))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut tuples: Vec<_> = json["results"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            ["a", "b", "c"].map(|v| row[v]["value"].as_str().unwrap().parse::<u8>().unwrap())
        })
        .collect();
    tuples.sort();
    let expected: Vec<_> = (0..4)
        .flat_map(|a| (0..4).flat_map(move |b| (0..4).map(move |c| [a, b, c])))
        .collect();
    assert_eq!(tuples, expected);
}

#[tokio::test]
async fn compiler_clone_work_preserves_exact_public_results_and_avoids_hit_replay() {
    for protected_profile in [false, true] {
        let mut cfg = Arc::new(if protected_profile {
            protected(10_000)
        } else {
            config(QueryLimits::new(10_000, u64::MAX, u64::MAX, u64::MAX))
        });
        for warm in [false, true] {
            if warm {
                Arc::get_mut(&mut cfg).unwrap().query_limits =
                    QueryLimits::new(CLONING.len() as u64, u64::MAX, u64::MAX, u64::MAX);
            }
            let response = router(cfg.clone())
                .oneshot(authenticated(CLONING))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let mut values: Vec<_> = json["results"]["bindings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["x"]["value"].as_str().unwrap())
                .collect();
            values.sort();
            assert_eq!(values, ["1", "2", "3"]);
        }
    }
}

#[tokio::test]
async fn ask_result_limit_is_a_pre_response_429() {
    let response = route(
        QueryLimits::new(u64::MAX, 100, 0, u64::MAX),
        "ASK { ?item <http://example.test/value> ?value }",
    )
    .await;
    assert_budget_problem(response).await;
}

#[tokio::test]
async fn ask_serialized_byte_limit_is_a_pre_response_429() {
    let response = route(
        QueryLimits::new(u64::MAX, 100, 1, 0),
        "ASK { ?item <http://example.test/value> ?value }",
    )
    .await;
    assert_budget_problem(response).await;
}

async fn assert_post_handoff_failure(limits: QueryLimits) {
    let response = route(
        limits,
        "SELECT ?value WHERE { ?item <http://example.test/value> ?value }",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let error = response
        .into_body()
        .collect()
        .await
        .expect_err("stream must terminate at its budget");
    assert_eq!(error.to_string(), "result stream failed");
}

#[tokio::test]
async fn select_source_limit_is_deterministically_post_handoff() {
    for _ in 0..8 {
        assert_post_handoff_failure(QueryLimits::new(u64::MAX, 0, u64::MAX, u64::MAX)).await;
    }
}

#[tokio::test]
async fn select_result_limit_is_a_redacted_post_handoff_failure() {
    assert_post_handoff_failure(QueryLimits::new(u64::MAX, 100, 1, u64::MAX)).await;
}

#[tokio::test]
async fn serializer_header_counts_toward_the_post_handoff_byte_limit() {
    assert_post_handoff_failure(QueryLimits::new(u64::MAX, 100, 2, 0)).await;
}
