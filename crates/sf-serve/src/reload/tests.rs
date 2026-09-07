use super::*;
use axum::body::{Body, Bytes};
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use std::path::PathBuf;
use tower::ServiceExt;
#[path = "policy_tests.rs"]
mod policy;

const TOKEN: &str = "fixture-only-reload-bearer-000000000000";
const QUERY: &str = "SELECT ?value WHERE { ?s <http://example.test/value> ?value }";

struct Fixture {
    root: PathBuf,
    opts: Arc<ServeOptions>,
    source: PreparedSource,
    baseline: std::sync::Mutex<Option<Arc<Baseline>>>,
}

impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sf-reload-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let db = root.join("source.db");
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch("CREATE TABLE items(value TEXT); INSERT INTO items VALUES ('old');")
            .unwrap();
        std::fs::write(
            root.join("ontology.ttl"),
            "<http://example.test/value> a <http://www.w3.org/1999/02/22-rdf-syntax-ns#Property> .",
        )
        .unwrap();
        std::fs::write(root.join("mapping.ttl"), mapping("rr:column \"value\"")).unwrap();
        let opts = Arc::new(ServeOptions {
            query_admission: crate::QueryAdmission::Bearer(
                crate::BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
            ),
            source: crate::SourceRef::inline(format!("sqlite:{}", db.display())),
            mapping: crate::MappingRef::r2rml_file(root.join("mapping.ttl").to_str().unwrap()),
            additional_source: None,
            ontology_path: root.join("ontology.ttl").to_string_lossy().into_owned(),
            bind: "127.0.0.1:0".into(),
            timeout: Duration::from_secs(5),
            max_query_len: 4096,
            max_concurrent_requests: 8,
            max_source_work: 1000,
            max_result_items: 1000,
            max_order_rows: 100,
            max_order_bytes: 4096,
            max_serialized_bytes: 4096,
            pg_pool_size: 1,
            pg_pool_wait: Duration::from_secs(1),
            sqlite_pool_size: 1,
            shutdown_timeout: Duration::from_secs(1),
            reload_interval: Duration::from_secs(1),
            metrics: None,
        });
        let source = opts.source.resolve().unwrap().prepare().unwrap();
        Self {
            root,
            opts,
            source,
            baseline: std::sync::Mutex::new(None),
        }
    }

    async fn config(&self) -> Arc<ServeConfig> {
        let (config, baseline) =
            crate::startup::build_config(&self.opts, self.source.clone(), None)
                .await
                .unwrap();
        *self.baseline.lock().unwrap() = Some(Arc::new(baseline));
        Arc::new(config)
    }

    async fn refresh(&self, config: &ServeConfig) {
        let runtime = config.lifecycle_runtime();
        let expected = runtime.readiness().unwrap();
        let baseline = self.baseline.lock().unwrap().as_ref().unwrap().clone();
        if let Some(next) = refresh(
            runtime,
            baseline,
            Arc::clone(&self.opts),
            self.source.clone(),
            None,
            expected,
        )
        .await
        {
            *self.baseline.lock().unwrap() = Some(next);
        }
    }

    fn replace_mapping(&self, object: &str) {
        std::fs::write(self.root.join("mapping.ttl"), mapping(object)).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // This directory was created exclusively by this fixture, never user input.
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

fn mapping(object: &str) -> String {
    format!("@prefix rr: <http://www.w3.org/ns/r2rml#> .\n<#items> a rr:TriplesMap; rr:logicalTable [rr:tableName \"items\"]; rr:subjectMap [rr:constant <http://example.test/item>]; rr:predicateObjectMap [rr:predicate <http://example.test/value>; rr:objectMap [{object}]].")
}

fn request(body: Body, authenticated: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, "application/sparql-results+json");
    if authenticated {
        request = request.header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    }
    request.body(body).unwrap()
}

async fn value(response: axum::response::Response) -> String {
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["results"]["bindings"].as_array().unwrap().len(), 1);
    json["results"]["bindings"][0]["value"]["value"]
        .as_str()
        .unwrap()
        .into()
}

#[tokio::test]
async fn public_requests_keep_their_generation_across_a_real_rebuild() {
    let fixture = Fixture::new();
    let config = fixture.config().await;
    let (body_tx, body_rx) =
        tokio::sync::mpsc::channel::<Result<Bytes, std::convert::Infallible>>(1);
    let (polled_tx, polled_rx) = tokio::sync::oneshot::channel();
    let stream = tokio_stream::wrappers::ReceiverStream::new(body_rx);
    // A ready request pins its generation before polling this body.
    use tokio_stream::StreamExt;
    let mut polled_tx = Some(polled_tx);
    let stream = stream.map(move |item| {
        if let Some(tx) = polled_tx.take() {
            let _ = tx.send(());
        }
        item
    });
    let app = crate::router(Arc::clone(&config));
    let pending = tokio::spawn(app.oneshot(request(Body::from_stream(stream), true)));
    body_tx
        .send(Ok(Bytes::from_static(b"SELECT ")))
        .await
        .unwrap();
    polled_rx.await.unwrap();
    let old = config.runtime_readiness().unwrap().activation_id();
    fixture.replace_mapping("rr:constant \"new\"");
    // Pause at the production capture boundary before parsing/opening/building.
    // New requests must already be refused; the partially read old request survives.
    let runtime = config.lifecycle_runtime();
    let attempt = Attempt::new(
        Arc::clone(&runtime),
        fixture.baseline.lock().unwrap().as_ref().unwrap().clone(),
        runtime.readiness().unwrap(),
    );
    attempt.capture(&fixture.opts).unwrap();
    assert_eq!(
        crate::router(Arc::clone(&config))
            .oneshot(request(Body::from(QUERY), true))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    fixture.refresh(&config).await;
    assert!(config.runtime_readiness().unwrap().activation_id() > old);
    body_tx
        .send(Ok(Bytes::copy_from_slice(&QUERY.as_bytes()[7..])))
        .await
        .unwrap();
    drop(body_tx);
    assert_eq!(value(pending.await.unwrap().unwrap()).await, "old");
    assert_eq!(
        value(
            crate::router(Arc::clone(&config))
                .oneshot(request(Body::from(QUERY), true))
                .await
                .unwrap()
        )
        .await,
        "new"
    );
    assert_eq!(
        crate::router(config)
            .oneshot(request(Body::from(QUERY), false))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn invalid_files_fence_public_readiness_and_valid_rebuild_recovers() {
    let fixture = Fixture::new();
    let config = fixture.config().await;
    let initial = config.runtime_readiness().unwrap().activation_id();
    std::fs::write(fixture.root.join("mapping.ttl"), "not turtle").unwrap();
    fixture.refresh(&config).await;
    assert!(matches!(
        config.runtime_readiness().unwrap(),
        RuntimeReadiness::NotReady {
            cause: ReadinessCause::SchemaDrift,
            ..
        }
    ));
    for uri in ["/readyz", "/livez"] {
        let response = crate::router(Arc::clone(&config))
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if uri == "/readyz" {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::OK
            }
        );
    }
    assert_eq!(
        crate::router(Arc::clone(&config))
            .oneshot(request(Body::from(QUERY), true))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    fixture.replace_mapping("rr:constant \"recovered\"");
    fixture.refresh(&config).await;
    assert!(config.runtime_readiness().unwrap().activation_id() > initial);
    assert_eq!(
        value(
            crate::router(config)
                .oneshot(request(Body::from(QUERY), true))
                .await
                .unwrap()
        )
        .await,
        "recovered"
    );
}

#[tokio::test]
async fn shutdown_cannot_be_healed_by_an_old_or_new_candidate() {
    let fixture = Fixture::new();
    let config = fixture.config().await;
    let runtime = config.lifecycle_runtime();
    let expected = runtime.readiness().unwrap();
    config.begin_shutdown();
    for expected in [expected, runtime.readiness().unwrap()] {
        let inputs = SemanticInputs::capture(&fixture.opts).unwrap();
        let snapshot = crate::startup::build_snapshot(
            &fixture.opts,
            &inputs,
            fixture.source.clone(),
            None,
            |_, _| Ok(()),
        )
        .await
        .unwrap();
        assert!(runtime
            .activate_authored(
                &ReloadAuthority(()),
                AuthoredCandidate { expected, snapshot }
            )
            .is_err());
        assert!(terminal(runtime.readiness().unwrap()));
    }
}

#[tokio::test]
async fn replacing_a_database_with_equal_schema_does_not_keep_the_old_inode() {
    let fixture = Fixture::new();
    let config = fixture.config().await;
    let replacement = fixture.root.join("replacement.db");
    rusqlite::Connection::open(&replacement)
        .unwrap()
        .execute_batch("CREATE TABLE items(value TEXT); INSERT INTO items VALUES ('replacement');")
        .unwrap();
    std::fs::rename(replacement, fixture.root.join("source.db")).unwrap();
    fixture.refresh(&config).await;
    assert_eq!(
        value(
            crate::router(config)
                .oneshot(request(Body::from(QUERY), true))
                .await
                .unwrap()
        )
        .await,
        "replacement"
    );
}

#[tokio::test]
async fn unexpected_coordinator_abort_fences_readiness() {
    let fixture = Fixture::new();
    let config = fixture.config().await;
    let supervisor = Supervisor::start(
        Arc::clone(&config),
        Arc::clone(&fixture.opts),
        fixture.source.clone(),
        None,
        fixture.baseline.lock().unwrap().as_ref().unwrap().clone(),
    );
    tokio::task::yield_now().await;
    supervisor.worker.abort();
    while !supervisor.worker.is_finished() {
        tokio::task::yield_now().await;
    }
    assert!(terminal(config.runtime_readiness().unwrap()));
}

#[cfg(unix)]
#[test]
fn semantic_fifo_is_rejected_without_waiting_for_a_writer() {
    let fixture = Fixture::new();
    let path = fixture.root.join("mapping.ttl");
    std::fs::remove_file(&path).unwrap();
    let c_path = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    let before = std::time::Instant::now();
    assert!(SemanticInputs::capture(&fixture.opts).is_err());
    assert!(before.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn schema_drift_fences_before_remaining_candidate_validation() {
    let fixture = Fixture::new();
    let config = fixture.config().await;
    let runtime = config.lifecycle_runtime();
    let expected = runtime.readiness().unwrap();
    let attempt = Attempt::new(
        Arc::clone(&runtime),
        fixture.baseline.lock().unwrap().as_ref().unwrap().clone(),
        expected,
    );
    attempt.capture(&fixture.opts).unwrap();
    assert_eq!(
        runtime.readiness().unwrap(),
        expected,
        "unchanged polling remains ready"
    );
    rusqlite::Connection::open(fixture.root.join("source.db"))
        .unwrap()
        .execute_batch("ALTER TABLE items RENAME COLUMN value TO changed;")
        .unwrap();
    let source = crate::run::open_backend(fixture.source.clone(), 1, Duration::from_secs(1), 1)
        .await
        .unwrap();
    attempt
        .observe(
            &mut Default::default(),
            sf_core::SourceId::new(0).unwrap(),
            &source,
        )
        .unwrap();
    assert_eq!(
        crate::router(Arc::clone(&config))
            .oneshot(request(Body::from(QUERY), true))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    fixture.refresh(&config).await;
    assert!(matches!(
        runtime.readiness().unwrap(),
        RuntimeReadiness::NotReady { .. }
    ));
    fixture.replace_mapping("rr:column \"changed\"");
    fixture.refresh(&config).await;
    assert_eq!(
        value(
            crate::router(config)
                .oneshot(request(Body::from(QUERY), true))
                .await
                .unwrap()
        )
        .await,
        "old"
    );
}

#[tokio::test]
async fn timeout_retains_the_worker_and_late_panic_is_terminal() {
    let fixture = Fixture::new();
    let config = fixture.config().await;
    let runtime = config.lifecycle_runtime();
    let attempt = Attempt::new(
        Arc::clone(&runtime),
        fixture.baseline.lock().unwrap().as_ref().unwrap().clone(),
        runtime.readiness().unwrap(),
    );
    let (release, blocked) = std::sync::mpsc::channel::<()>();
    let worker = tokio::task::spawn_blocking(move || -> BuildResult {
        blocked.recv().unwrap();
        panic!("test-only late candidate failure");
    });
    let waiting_runtime = Arc::clone(&runtime);
    let waiting = tokio::spawn(async move {
        finish_attempt(
            &waiting_runtime,
            &attempt,
            worker,
            Duration::from_millis(10),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while matches!(runtime.readiness().unwrap(), RuntimeReadiness::Ready { .. }) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        !waiting.is_finished(),
        "timeout must retain actual worker ownership"
    );
    release.send(()).unwrap();
    assert!(waiting.await.unwrap().is_none());
    assert!(terminal(runtime.readiness().unwrap()));
    fixture.refresh(&config).await;
    assert!(
        terminal(runtime.readiness().unwrap()),
        "later candidate cannot heal a worker panic"
    );
}
