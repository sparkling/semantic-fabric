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

#[path = "profile_test_fixture.rs"]
mod profile_fixture;
use profile_fixture::Fixture;

#[tokio::test]
async fn generated_profile_and_subject_policy_survive_mapping_rebuild() {
    let fixture = Fixture::with_profile(crate::QueryShapeProfile::GeneratedSelectAsk);
    let config = fixture.config().await;
    assert_eq!(
        config.query_shape_profile(),
        crate::QueryShapeProfile::GeneratedSelectAsk
    );
    let mut identities = Vec::new();
    for expected in ["old", "new"] {
        let denied = crate::router(Arc::clone(&config))
            .oneshot(request(Body::from(QUERY), false))
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert!(!denied.headers().contains_key("x-semantic-fabric-profile"));
        let admitted = crate::router(Arc::clone(&config))
            .oneshot(request(Body::from(QUERY), true))
            .await
            .unwrap();
        identities.push(admitted.headers()["x-semantic-fabric-profile"].clone());
        assert_eq!(value(admitted).await, expected);
        if expected == "old" {
            fixture.replace_mapping("rr:constant \"new\"");
            fixture.refresh(&config).await;
        }
    }
    assert_ne!(identities[0], identities[1]);
}

#[tokio::test]
async fn failed_protected_candidate_fences_the_exact_generation_cause() {
    use crate::pg_generation::{authored::generation_error, PgGenerationError};
    for error in [
        PgGenerationError::SchemaDrift,
        PgGenerationError::CapabilityDrift,
        PgGenerationError::SourceUnavailable,
    ] {
        let fixture = Fixture::new();
        let config = fixture.config().await;
        let runtime = config.lifecycle_runtime();
        let expected = runtime.readiness().unwrap();
        let baseline = fixture.baseline.lock().unwrap().clone().unwrap();
        let attempt = Attempt::new(Arc::clone(&runtime), baseline, expected);
        let error = generation_error(error);
        let cause = rejection_cause(&error);
        let worker = tokio::task::spawn_blocking(move || Err(error));
        assert!(
            finish_attempt(&runtime, &attempt, worker, Duration::from_secs(1))
                .await
                .is_none()
        );
        assert!(
            matches!(runtime.readiness().unwrap(), RuntimeReadiness::NotReady { cause: actual, .. } if actual == cause)
        );
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
    // This hand-opened source is unverified, so the verified baseline also
    // refuses it as a downgrade; either way the fence precedes validation.
    let _ = attempt.observe(
        &mut Default::default(),
        sf_core::SourceId::new(0).unwrap(),
        &source,
    );
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

/// A source admitted on the sealed SQLite generation must never be republished
/// unverified. An FTS5 table makes the builder decline, so ordinary startup's
/// fallback would open the source unverified; reload must refuse it instead.
#[tokio::test]
async fn verified_source_never_reloads_unverified() {
    let fixture = Fixture::new();
    let config = fixture.config().await;
    let runtime = config.lifecycle_runtime();
    let baseline = fixture.baseline.lock().unwrap().clone().unwrap();
    assert!(
        baseline.is_verified(sf_core::SourceId::new(0).unwrap()),
        "ordinary file-backed SQLite starts on the verified generation"
    );
    rusqlite::Connection::open(fixture.root.join("source.db"))
        .unwrap()
        .execute_batch("CREATE VIRTUAL TABLE notes USING fts5(body);")
        .unwrap();
    fixture.refresh(&config).await;
    assert!(
        matches!(
            runtime.readiness().unwrap(),
            RuntimeReadiness::NotReady {
                cause: ReadinessCause::CapabilityDrift,
                ..
            }
        ),
        "a declined verified source must stay fenced, not republish unverified"
    );
    assert_eq!(
        crate::router(Arc::clone(&config))
            .oneshot(request(Body::from(QUERY), true))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    fixture.refresh(&config).await;
    assert!(
        matches!(
            runtime.readiness().unwrap(),
            RuntimeReadiness::NotReady { .. }
        ),
        "a later refresh of the same shape still refuses"
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
