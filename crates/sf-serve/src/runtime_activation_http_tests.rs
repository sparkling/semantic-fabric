use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_core::{SourceId, SourceMapping};
use sf_sparql::Epoch;
use tokio_stream::StreamExt;
use tower::ServiceExt;

use crate::{
    introspect_sqlite_all, router, Backend, IntrospectedSource, ReadinessCause, RuntimeSnapshot,
    RuntimeSource, ServeConfig,
};

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#people> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "People" ] ;
  rr:subjectMap [ rr:template "http://example.test/person/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:name ;
    rr:objectMap [ rr:column "name" ]
  ] .
"#;

fn source_parts(name: &str) -> (IntrospectedSource, SourceMapping) {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE \"People\" (\"id\" INTEGER PRIMARY KEY, \"name\" TEXT NOT NULL);\
         INSERT INTO \"People\" VALUES (1, '{}');",
        name.replace('\'', "''")
    ))
    .unwrap();
    let schema = introspect_sqlite_all(&conn).unwrap();
    let source_id = SourceId::new(0).unwrap();
    let mapping = SourceMapping::new(source_id, sf_mapping::parse_r2rml(MAPPING).unwrap());
    (
        IntrospectedSource::unchecked(Backend::sqlite(conn), schema),
        mapping,
    )
}

fn runtime_source(name: &str) -> RuntimeSource {
    let (source, mapping) = source_parts(name);
    RuntimeSource::new(source, mapping)
}

fn config(name: &str) -> ServeConfig {
    let (source, mapping) = source_parts(name);
    ServeConfig::new(
        source,
        mapping,
        crate::test_support::ontology(&[], &["http://example.test/name"]),
    )
    .unwrap()
}

fn request(body: Body) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(body)
        .unwrap()
}

async fn query_name(config: Arc<ServeConfig>) -> String {
    let response = router(config)
        .oneshot(request(Body::from(
            "SELECT ?name WHERE { ?s <http://example.test/name> ?name }",
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    json["results"]["bindings"][0]["name"]["value"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn new_http_requests_switch_whole_snapshots_after_activation() {
    let config = Arc::new(config("Alice"));
    assert_eq!(query_name(config.clone()).await, "Alice");

    let expected = config.runtime_readiness().unwrap();
    let activated = config
        .activate_snapshot(
            expected,
            RuntimeSnapshot::single(
                Epoch(1),
                crate::test_support::ontology(&[], &["http://example.test/name"]),
                runtime_source("Bob"),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(activated > expected.activation_id());
    assert_eq!(query_name(config).await, "Bob");
}

#[tokio::test]
async fn not_ready_rejects_before_polling_the_request_body() {
    let config = Arc::new(config("Alice"));
    let expected = config.runtime_readiness().unwrap();
    config
        .mark_runtime_not_ready(expected, ReadinessCause::SchemaDrift)
        .unwrap();

    let polled = Arc::new(AtomicBool::new(false));
    let observed = polled.clone();
    let stream =
        tokio_stream::iter([Ok::<_, Infallible>(Bytes::from_static(b"query"))]).map(move |chunk| {
            observed.store(true, Ordering::SeqCst);
            chunk
        });
    let response = router(config)
        .oneshot(request(Body::from_stream(stream)))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::RETRY_AFTER], "1");
    assert!(!polled.load(Ordering::SeqCst));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], "source-unavailable");
}

#[tokio::test]
async fn lineage_uses_the_request_pinned_snapshot_across_reload_and_cache_hits() {
    const FORMAT: &str = "application/vnd.semantic-fabric.lineage+json-seq";
    const QUERY: &str = "SELECT ?name WHERE { ?s <http://example.test/name> ?name }";
    let config = Arc::new(config("Alice"));
    let (polled_tx, polled_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let mut gates = Some((polled_tx, release_rx));
    let stream = tokio_stream::once(()).then(move |_| {
        let (polled, release) = gates.take().unwrap();
        async move {
            polled.send(()).unwrap();
            release.await.unwrap();
            Ok::<_, Infallible>(Bytes::from_static(QUERY.as_bytes()))
        }
    });
    let mut req = request(Body::from_stream(stream));
    req.headers_mut()
        .insert(header::ACCEPT, FORMAT.parse().unwrap());
    let old = tokio::spawn(router(config.clone()).oneshot(req));
    tokio::time::timeout(std::time::Duration::from_secs(2), polled_rx)
        .await
        .unwrap()
        .unwrap();
    let expected = config.runtime_readiness().unwrap();
    config
        .activate_snapshot(
            expected,
            RuntimeSnapshot::single(
                Epoch(1),
                crate::test_support::ontology(&[], &["http://example.test/name"]),
                runtime_source("Bob"),
            )
            .unwrap(),
        )
        .unwrap();
    release_tx.send(()).unwrap();
    let old = old
        .await
        .unwrap()
        .unwrap()
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let old = std::str::from_utf8(&old).unwrap();
    assert!(old.contains("Alice") && !old.contains("Bob"));
    let mut req = request(Body::from(QUERY));
    req.headers_mut()
        .insert(header::ACCEPT, FORMAT.parse().unwrap());
    let new = router(config)
        .oneshot(req)
        .await
        .unwrap()
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let new = std::str::from_utf8(&new).unwrap();
    assert!(new.contains("Bob") && !new.contains("Alice"));
    let header = |text: &str| {
        serde_json::from_str::<serde_json::Value>(text.split('\u{1e}').nth(1).unwrap()).unwrap()
    };
    assert_ne!(header(old)["snapshot"], header(new)["snapshot"]);
    assert_ne!(header(old)["logicalPlan"], header(new)["logicalPlan"]);
}

fn typed_source(integer: bool) -> (IntrospectedSource, SourceMapping) {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(if integer {
        "CREATE TABLE People(id INTEGER PRIMARY KEY, name INTEGER NOT NULL); \
         INSERT INTO People VALUES (1, 7), (2, 7);"
    } else {
        "CREATE TABLE People(id INTEGER PRIMARY KEY, name TEXT NOT NULL); \
         INSERT INTO People VALUES (1, 'Bob'), (2, 'Bob');"
    })
    .unwrap();
    (
        IntrospectedSource::observe_sqlite(Backend::sqlite(conn)).unwrap(),
        SourceMapping::new(
            SourceId::new(0).unwrap(),
            sf_mapping::parse_r2rml(MAPPING).unwrap(),
        ),
    )
}

fn typed_ontology(integer: bool) -> crate::SemanticOntology {
    let datatype = if integer { "integer" } else { "string" };
    crate::SemanticOntology::from_turtle(&format!(
        "@prefix ex: <http://example.test/> . \
         @prefix owl: <http://www.w3.org/2002/07/owl#> . \
         @prefix sh: <http://www.w3.org/ns/shacl#> . \
         @prefix xsd: <http://www.w3.org/2001/XMLSchema#> . \
         ex:name a owl:DatatypeProperty . \
         ex:shape sh:property [ sh:path ex:name; sh:datatype xsd:{datatype} ] ."
    ))
    .unwrap()
}

async fn assert_typed_bag(response: axum::response::Response, integer: bool) {
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["head"]["vars"], serde_json::json!(["name"]));
    let term = if integer {
        serde_json::json!({"type":"literal", "value":"7",
            "datatype":"http://www.w3.org/2001/XMLSchema#integer"})
    } else {
        serde_json::json!({"type":"literal", "value":"Bob"})
    };
    assert_eq!(
        json["results"]["bindings"],
        serde_json::json!([
            {"name":term.clone()}, {"name":term}
        ])
    );
}

#[tokio::test]
async fn semantic_admission_and_warm_plans_stay_with_the_request_generation() {
    use sf_core::query_control::UncontrolledQueryControl;

    const QUERY: &str = "SELECT ?name WHERE { ?s <http://example.test/name> ?name }";
    let source_id = SourceId::new(0).unwrap();
    let (source, mapping) = typed_source(true);
    let config = Arc::new(ServeConfig::new(source, mapping, typed_ontology(true)).unwrap());
    let old = config.runtime_lease().unwrap();
    let cold = old
        .compile(source_id, QUERY, &UncontrolledQueryControl)
        .unwrap();
    let warm = old
        .compile(source_id, QUERY, &UncontrolledQueryControl)
        .unwrap();
    // A shared plan address proves cache reuse, not merely equal query results.
    assert!(std::ptr::eq(cold.plan(), warm.plan()));
    for _ in 0..2 {
        let response = router(config.clone())
            .oneshot(request(Body::from(QUERY)))
            .await
            .unwrap();
        assert_typed_bag(response, true).await;
    }

    let (polled_tx, polled_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let mut gates = Some((polled_tx, release_rx));
    let stream = tokio_stream::once(()).then(move |_| {
        let (polled, release) = gates.take().unwrap();
        async move {
            polled.send(()).unwrap();
            release.await.unwrap();
            Ok::<_, Infallible>(Bytes::from_static(QUERY.as_bytes()))
        }
    });
    let held = tokio::spawn(router(config.clone()).oneshot(request(Body::from_stream(stream))));
    tokio::time::timeout(std::time::Duration::from_secs(2), polled_rx)
        .await
        .unwrap()
        .unwrap();

    let before = config.runtime_readiness().unwrap();
    let (source, mapping) = typed_source(false);
    let invalid = RuntimeSnapshot::single(
        Epoch(1),
        typed_ontology(true),
        RuntimeSource::new(source, mapping),
    );
    assert!(
        matches!(invalid, Err(crate::SnapshotError::SemanticAdmission {
        cause: crate::SemanticAdmissionError::Violations { count }, ..
    }) if count > 0)
    );
    assert_eq!(config.runtime_readiness().unwrap(), before);
    config
        .mark_runtime_not_ready(before, ReadinessCause::SchemaDrift)
        .unwrap();
    let refused = router(config.clone())
        .oneshot(request(Body::from(QUERY)))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = refused.into_body().collect().await.unwrap().to_bytes();
    let problem: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(problem["code"], "source-unavailable");

    let (source, mapping) = typed_source(false);
    let successor = RuntimeSnapshot::single(
        Epoch(1),
        typed_ontology(false),
        RuntimeSource::new(source, mapping),
    )
    .unwrap();
    let old_digests = old.snapshot().registry().digests(source_id).unwrap();
    let new_digests = successor.registry().digests(source_id).unwrap();
    assert_eq!(old_digests.mapping(), new_digests.mapping());
    assert_ne!(old_digests.ontology(), new_digests.ontology());
    assert_ne!(
        old_digests.semantic_admission(),
        new_digests.semantic_admission()
    );
    config
        .activate_snapshot(config.runtime_readiness().unwrap(), successor)
        .unwrap();
    let new = config.runtime_lease().unwrap();
    assert_ne!(old.activation_id(), new.activation_id());
    let new_cold = new
        .compile(source_id, QUERY, &UncontrolledQueryControl)
        .unwrap();
    let new_warm = new
        .compile(source_id, QUERY, &UncontrolledQueryControl)
        .unwrap();
    assert!(std::ptr::eq(new_cold.plan(), new_warm.plan()));
    assert!(!std::ptr::eq(cold.plan(), new_cold.plan()));
    assert!(new.prepare_execution(warm).is_err());
    assert!(old.prepare_execution(new_warm).is_err());

    // Successor requests complete while the earlier request still pins its lease.
    for _ in 0..2 {
        let response = router(config.clone())
            .oneshot(request(Body::from(QUERY)))
            .await
            .unwrap();
        assert_typed_bag(response, false).await;
    }
    release_tx.send(()).unwrap();
    let response = tokio::time::timeout(std::time::Duration::from_secs(2), held)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_typed_bag(response, true).await;
    let old_warm = old
        .compile(source_id, QUERY, &UncontrolledQueryControl)
        .unwrap();
    assert!(std::ptr::eq(cold.plan(), old_warm.plan()));
}
