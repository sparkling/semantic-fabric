use super::*;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_core::query_control::QueryLimits;
use std::path::PathBuf;
use std::time::Duration;
use tower::ServiceExt;

const SELECT: &str = "SELECT ?value WHERE { ?s <http://example.test/value> ?value }";
const ASK: &str = "ASK { ?s <http://example.test/value> ?value }";
const CONSTRUCT: &str = "CONSTRUCT { ?s <http://example.test/value> ?value } WHERE { ?s <http://example.test/value> ?value }";
const MAPPING: &str = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> a rr:TriplesMap; rr:logicalTable [rr:tableName "items"];
rr:subjectMap [rr:template "http://example.test/item/{id}"];
rr:predicateObjectMap [rr:predicate <http://example.test/value>; rr:objectMap [rr:column "value"]]."#;

struct Fixture {
    path: PathBuf,
    writer: rusqlite::Connection,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "sf-protected-http-{}-{}.db",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let writer = rusqlite::Connection::open(&path).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE items(id INTEGER, value INTEGER NOT NULL); INSERT INTO items VALUES(1,7),(2,7)").unwrap();
        Self { path, writer }
    }

    async fn config(&self) -> (crate::ServeConfig, Arc<SqliteGeneration>) {
        self.config_mapping(MAPPING).await
    }

    async fn config_mapping(&self, text: &str) -> (crate::ServeConfig, Arc<SqliteGeneration>) {
        let mapping = SourceMapping::new(
            SourceId::new(0).unwrap(),
            sf_mapping::parse_r2rml(text).unwrap(),
        );
        let ontology = crate::test_support::ontology(
            &[],
            &["http://example.test/value", "http://example.test/other"],
        );
        let (source, mapping) = build(
            self.path.to_str().unwrap().into(),
            1,
            mapping,
            &ontology,
            &budget(),
            &mut |_, _| Ok(()),
        )
        .await
        .unwrap();
        let (backend, _, _, generation) = source.into_parts();
        let crate::pg_generation::SourceGeneration::AuthoredSqlite(expected) = generation else {
            panic!("SQLite generation")
        };
        // Poison only the legacy handle's name resolution. Verified execution
        // must never use it, even though its BackendKind is still SQLite.
        let crate::Backend::Sqlite(pool) = &backend else {
            panic!("SQLite backend")
        };
        pool.pick().lock().unwrap().execute_batch("CREATE TEMP TABLE items(id INTEGER, value INTEGER); INSERT INTO items VALUES(1,999)").unwrap();
        let source = IntrospectedSource::observed_sqlite_generation(backend, expected.clone());
        let runtime = crate::RuntimeSource::admitted(source, mapping).unwrap();
        let mut config = crate::ServeConfig::from_runtime_source(runtime, ontology).unwrap();
        config.use_in_process_test_parser();
        config.set_query_admission(crate::QueryAdmission::UnrestrictedDevelopment);
        (config, expected)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}

fn budget() -> RequestBudget {
    RequestBudget::after(
        Duration::from_secs(10),
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    )
}

async fn request(
    config: Arc<crate::ServeConfig>,
    query: &'static str,
    accept: &'static str,
) -> axum::response::Response {
    crate::router(config)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sparql")
                .header(header::CONTENT_TYPE, "application/sparql-query")
                .header(header::ACCEPT, accept)
                .body(Body::from(query))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn bytes(response: axum::response::Response) -> Vec<u8> {
    assert_eq!(response.status(), StatusCode::OK);
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

#[tokio::test]
async fn public_forms_and_lineage_use_the_sealed_member_and_recover_capacity() {
    let fixture = Fixture::new();
    let (config, expected) = fixture.config().await;
    let config = Arc::new(config);
    for _ in 0..2 {
        let body =
            bytes(request(config.clone(), SELECT, "application/sparql-results+json").await).await;
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["results"]["bindings"],
            serde_json::json!([
                {"value":{"type":"literal","datatype":"http://www.w3.org/2001/XMLSchema#integer","value":"7"}},
                {"value":{"type":"literal","datatype":"http://www.w3.org/2001/XMLSchema#integer","value":"7"}}
            ])
        );
    }
    let body = bytes(request(config.clone(), ASK, "application/sparql-results+json").await).await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["boolean"],
        true
    );
    let body = bytes(request(config.clone(), CONSTRUCT, "application/n-triples").await).await;
    let text = String::from_utf8(body).unwrap();
    let mut triples: Vec<_> = text.lines().collect();
    triples.sort();
    assert_eq!(triples, [
        "<http://example.test/item/1> <http://example.test/value> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
        "<http://example.test/item/2> <http://example.test/value> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
    ]);

    for query in [SELECT, CONSTRUCT] {
        let body = bytes(request(config.clone(), query, crate::lineage::MEDIA_TYPE).await).await;
        let records: Vec<serde_json::Value> = body
            .split(|b| *b == 0x1e)
            .filter(|part| !part.is_empty())
            .map(|part| serde_json::from_slice(part).unwrap())
            .collect();
        assert_eq!(records.first().unwrap()["type"], "header");
        assert_eq!(records.last().unwrap()["type"], "complete");
    }
    expected
        .acquire(&budget())
        .await
        .unwrap()
        .finish()
        .await
        .unwrap();
}

#[tokio::test]
async fn multi_mapping_lineage_uses_the_verified_execution_wrapper() {
    let fixture = Fixture::new();
    let second = MAPPING
        .replace("<#items>", "<#other>")
        .replace("http://example.test/value", "http://example.test/other");
    let (config, expected) = fixture
        .config_mapping(&format!("{MAPPING}\n{second}"))
        .await;
    let query = "SELECT ?value WHERE { { ?s <http://example.test/value> ?value } UNION { ?s <http://example.test/other> ?value } }";
    let body = bytes(request(Arc::new(config), query, crate::lineage::MEDIA_TYPE).await).await;
    let records: Vec<serde_json::Value> = body
        .split(|b| *b == 0x1e)
        .filter(|p| !p.is_empty())
        .map(|p| serde_json::from_slice(p).unwrap())
        .collect();
    assert_eq!(records[0]["profile"], "bounded-mapping-source-v1");
    assert_eq!(
        records.last().unwrap(),
        &serde_json::json!({"type":"complete","solutions":4})
    );
    expected
        .acquire(&budget())
        .await
        .unwrap()
        .finish()
        .await
        .unwrap();
}

#[tokio::test]
async fn active_response_keeps_its_snapshot_and_warm_cache_cannot_admit_drift() {
    let fixture = Fixture::new();
    let (config, _) = fixture.config().await;
    let config = Arc::new(config);
    let first = request(config.clone(), SELECT, "application/sparql-results+json").await;
    fixture
        .writer
        .execute_batch("ALTER TABLE items ADD COLUMN extra TEXT; UPDATE items SET value=9")
        .unwrap();
    let old: serde_json::Value = serde_json::from_slice(&bytes(first).await).unwrap();
    assert!(old["results"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["value"]["value"] == "7"));
    let rejected = request(config.clone(), SELECT, "application/sparql-results+json").await;
    assert_eq!(rejected.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = rejected.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&body).contains(fixture.path.to_str().unwrap()));
    let (successor, _) = fixture.config().await;
    let new: serde_json::Value = serde_json::from_slice(
        &bytes(
            request(
                Arc::new(successor),
                SELECT,
                "application/sparql-results+json",
            )
            .await,
        )
        .await,
    )
    .unwrap();
    assert!(new["results"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["value"]["value"] == "9"));
}

#[tokio::test]
async fn rejected_ask_and_execution_error_finish_before_member_reuse() {
    let fixture = Fixture::new();
    let (mut config, expected) = fixture.config().await;
    config.query_limits = QueryLimits::new(u64::MAX, u64::MAX, 0, u64::MAX);
    let response = request(Arc::new(config), ASK, "application/sparql-results+json").await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    expected
        .acquire(&budget())
        .await
        .unwrap()
        .finish()
        .await
        .unwrap();
    let (config, expected) = fixture.config().await;
    let control = budget();
    let snapshot = config.lifecycle_runtime().lease().unwrap();
    let plan = snapshot
        .compile(expected.source_id(), SELECT, &control)
        .unwrap();
    let lease = expected.acquire(&control).await.unwrap();
    let error = lease
        .select_each(plan.plan(), &control, |_| async {
            Err(sf_sparql::Error::Sql("injected sink failure".into()))
        })
        .await;
    assert!(
        matches!(error, Err(sf_sparql::Error::Sql(message)) if message == "injected sink failure")
    );
    expected
        .acquire(&budget())
        .await
        .unwrap()
        .finish()
        .await
        .unwrap();
}

#[tokio::test]
async fn exact_binding_identity_controls_sqlite_lease_handoff() {
    let fixture = Fixture::new();
    let (config, expected) = fixture.config().await;
    let snapshot = config.lifecycle_runtime().lease().unwrap();
    let mut requirements = snapshot
        .generation_requirements([expected.source_id()])
        .unwrap();
    let identity = requirements[0].binding_identity().clone();
    let wrong = crate::binding_identity::RuntimeBindingIdentity::fresh();
    let mut leases = crate::generation::VerifiedGenerationLeases::acquire(
        std::mem::take(&mut requirements),
        &budget(),
    )
    .await
    .unwrap();
    assert!(leases.take(expected.source_id(), &wrong).is_none());
    assert!(!leases.is_empty());
    assert!(leases.matches(expected.source_id(), &identity, true));
    assert!(!leases.matches(expected.source_id(), &identity, false));
    assert!(matches!(
        leases.take(expected.source_id(), &identity),
        Some(crate::generation::VerifiedGenerationLease::Sqlite(_))
    ));
    expected
        .acquire(&budget())
        .await
        .unwrap()
        .finish()
        .await
        .unwrap();
}

#[tokio::test]
async fn abandoned_public_bodies_release_the_verified_member() {
    let fixture = Fixture::new();
    // Exceed the response channel so disconnect exercises a blocked producer.
    fixture.writer.execute_batch(
        "WITH RECURSIVE n(x) AS (VALUES(3) UNION ALL SELECT x+1 FROM n WHERE x<1000) INSERT INTO items SELECT x,7 FROM n"
    ).unwrap();
    let (config, expected) = fixture.config().await;
    let config = Arc::new(config);
    for (query, accept) in [
        (SELECT, "application/sparql-results+json"),
        (CONSTRUCT, "application/n-triples"),
        (SELECT, crate::lineage::MEDIA_TYPE),
        (CONSTRUCT, crate::lineage::MEDIA_TYPE),
    ] {
        let response = request(config.clone(), query, accept).await;
        assert_eq!(response.status(), StatusCode::OK);
        drop(response);
        tokio::time::timeout(Duration::from_secs(5), async {
            expected
                .acquire(&budget())
                .await
                .unwrap()
                .finish()
                .await
                .unwrap();
        })
        .await
        .unwrap();
    }
}
