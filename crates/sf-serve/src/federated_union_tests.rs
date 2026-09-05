use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_core::query_control::QueryLimits;
use sf_core::{SourceId, SourceMapping};
use sf_sparql::{Epoch, Tbox};
use tower::ServiceExt;

use crate::{
    router, Backend, IntrospectedSource, RuntimeSnapshot, RuntimeSource, ServeConfig, SqlitePool,
};

const UNION_DISTINCT_VARS: &str = "SELECT ?s ?left ?right WHERE { \
    { ?s <http://example.test/left> ?left } UNION \
    { ?s <http://example.test/right> ?right } }";
const UNION_SAME_VAR: &str = "SELECT ?s ?value WHERE { \
    { ?s <http://example.test/left> ?value } UNION \
    { ?s <http://example.test/right> ?value } }";
const UNION_REVERSED: &str = "SELECT ?s ?left ?right WHERE { \
    { ?s <http://example.test/right> ?right } UNION \
    { ?s <http://example.test/left> ?left } }";
static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

struct DbFile(PathBuf);

impl DbFile {
    fn new(label: &str, values: &[&str]) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sf_federated_union_{label}_{}_{unique}_{}.db",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let connection = rusqlite::Connection::open(&path).expect("create SQLite fixture");
        connection
            .execute_batch("CREATE TABLE items(value TEXT NOT NULL);")
            .expect("create fixture table");
        for value in values {
            connection
                .execute("INSERT INTO items(value) VALUES (?1)", [value])
                .expect("insert fixture value");
        }
        drop(connection);
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn execute(&self, sql: &str) {
        rusqlite::Connection::open(&self.0)
            .expect("open fixture writer")
            .execute_batch(sql)
            .expect("mutate fixture");
    }
}

impl Drop for DbFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn mapping(predicate: &str) -> String {
    format!(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subjectMap [ rr:constant <http://example.test/item> ] ;
  rr:predicateObjectMap [
    rr:predicate <{predicate}> ;
    rr:objectMap [ rr:column "value" ]
  ] ."#
    )
}

fn runtime_source(
    index: usize,
    predicate: &str,
    values: &[&str],
) -> (RuntimeSource, SqlitePool, DbFile) {
    let file = DbFile::new(&format!("{index}"), values);
    let (backend, schema) =
        Backend::sqlite_pool_from_path(file.path().to_str().expect("UTF-8 temp path"), 1)
            .expect("open read-only SQLite source");
    let Backend::Sqlite(pool) = &backend else {
        unreachable!("SQLite fixture")
    };
    let pool = pool.clone();
    let source_id = SourceId::new(index).unwrap();
    let source_mapping = SourceMapping::new(
        source_id,
        sf_mapping::parse_r2rml(&mapping(predicate)).expect("parse mapping"),
    );
    (
        RuntimeSource::new(
            IntrospectedSource::unchecked(backend, schema),
            source_mapping,
        ),
        pool,
        file,
    )
}

fn config(
    predicates: [&str; 2],
    values: [&[&str]; 2],
) -> (ServeConfig, [SqlitePool; 2], [DbFile; 2]) {
    let (left, left_pool, left_file) = runtime_source(0, predicates[0], values[0]);
    let (right, right_pool, right_file) = runtime_source(1, predicates[1], values[1]);
    let config = ServeConfig::new_federated([left, right], Tbox::default()).unwrap();
    (config, [left_pool, right_pool], [left_file, right_file])
}

fn request(query: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(Body::from(query.to_owned()))
        .unwrap()
}

async fn json(config: Arc<ServeConfig>, query: &str) -> serde_json::Value {
    let response = router(config).oneshot(request(query)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn two_file_sources_preserve_union_bags_and_unbound_domains() {
    let (initial, _pools, _files) = config(
        ["http://example.test/left", "http://example.test/right"],
        [&["same"], &["same"]],
    );
    let running = Arc::new(initial);

    let unbound = json(running.clone(), UNION_DISTINCT_VARS).await;
    let rows = unbound["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .any(|row| row.get("left").is_some() && row.get("right").is_none()));
    assert!(rows
        .iter()
        .any(|row| row.get("right").is_some() && row.get("left").is_none()));

    let duplicate = json(running, UNION_SAME_VAR).await;
    let rows = duplicate["results"]["bindings"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        2,
        "UNION ALL must retain cross-source duplicates"
    );
    assert!(rows.iter().all(|row| row["value"]["value"] == "same"));

    let (reversed_config, _pools, _files) = config(
        ["http://example.test/left", "http://example.test/right"],
        [&["left"], &["right"]],
    );
    let reversed = json(Arc::new(reversed_config), UNION_REVERSED).await;
    let rows = reversed["results"]["bindings"].as_array().unwrap();
    assert!(rows[0].get("right").is_some());
    assert!(rows[1].get("left").is_some());
}

#[tokio::test]
async fn ambiguous_absent_and_modifier_shapes_reject_before_source_io() {
    for (predicates, query) in [
        (
            ["http://example.test/left", "http://example.test/left"],
            UNION_SAME_VAR,
        ),
        (
            ["http://example.test/left", "http://example.test/right"],
            "SELECT ?s WHERE { { ?s <http://example.test/left> ?v } UNION { ?s <http://example.test/absent> ?v } }",
        ),
        (
            ["http://example.test/left", "http://example.test/right"],
            "SELECT ?s WHERE { { ?s <http://example.test/left> ?v } UNION { ?s <http://example.test/right> ?v } } ORDER BY ?s",
        ),
    ] {
        let (config, pools, _files) = config(predicates, [&["one"], &["two"]]);
        let _held_left = pools[0].pick_owned().acquire().await.unwrap();
        let _held_right = pools[1].pick_owned().acquire().await.unwrap();
        let io = Arc::new(AtomicUsize::new(0));
        for pool in &pools {
            let io = io.clone();
            pool.set_admission_pending_observer(move || {
                io.fetch_add(1, Ordering::SeqCst);
            });
        }
        let response = router(Arc::new(config))
            .oneshot(request(query))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
        assert_eq!(io.load(Ordering::SeqCst), 0, "rejection touched a pool");
    }
}

#[tokio::test]
async fn zero_exact_and_adjacent_result_budgets_are_shared_across_fragments() {
    for maximum in [0, 1] {
        let (mut limited, _pools, _files) = config(
            ["http://example.test/left", "http://example.test/right"],
            [&["same"], &["same"]],
        );
        limited.query_limits = QueryLimits::new(u64::MAX, u64::MAX, maximum, u64::MAX);
        let response = router(Arc::new(limited))
            .oneshot(request(UNION_SAME_VAR))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .into_body()
                .collect()
                .await
                .unwrap_err()
                .to_string(),
            "result stream failed"
        );
    }

    for maximum in [2, 3] {
        let (mut admitted, _pools, _files) = config(
            ["http://example.test/left", "http://example.test/right"],
            [&["same"], &["same"]],
        );
        admitted.query_limits = QueryLimits::new(u64::MAX, u64::MAX, maximum, u64::MAX);
        assert_eq!(
            json(Arc::new(admitted), UNION_SAME_VAR).await["results"]["bindings"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}

#[tokio::test]
async fn second_source_acquisition_timeout_occurs_before_success_commit_and_recovers() {
    let (mut config, pools, _files) = config(
        ["http://example.test/left", "http://example.test/right"],
        [&["one"], &["two"]],
    );
    config.timeout = Duration::from_millis(250);
    let config = Arc::new(config);
    let held = pools[1].pick_owned().acquire().await.unwrap();
    let pending = Arc::new(AtomicUsize::new(0));
    let observed = pending.clone();
    pools[1].set_admission_pending_observer(move || {
        observed.fetch_add(1, Ordering::SeqCst);
    });

    let response = router(config.clone())
        .oneshot(request(UNION_SAME_VAR))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(pending.load(Ordering::SeqCst), 1);
    drop(held);

    let recovered = tokio::time::timeout(Duration::from_secs(2), json(config, UNION_SAME_VAR))
        .await
        .expect("first-source lease and request capacity were released");
    assert_eq!(
        recovered["results"]["bindings"].as_array().unwrap().len(),
        2
    );
}

#[tokio::test]
async fn source_failure_is_redacted_and_releases_capacity_for_recovery() {
    let (mut config, _pools, files) = config(
        ["http://example.test/left", "http://example.test/right"],
        [&["one"], &["two"]],
    );
    config.set_max_concurrent_requests(1).unwrap();
    let config = Arc::new(config);
    files[1].execute("DROP TABLE items;");

    let response = router(config.clone())
        .oneshot(request(UNION_SAME_VAR))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let error = response.into_body().collect().await.unwrap_err();
    assert_eq!(error.to_string(), "result stream failed");

    files[1].execute("CREATE TABLE items(value TEXT NOT NULL); INSERT INTO items VALUES ('two');");
    let recovered = tokio::time::timeout(Duration::from_secs(2), json(config, UNION_SAME_VAR))
        .await
        .expect("released request and source capacity");
    assert_eq!(
        recovered["results"]["bindings"].as_array().unwrap().len(),
        2
    );
}

#[tokio::test]
async fn in_flight_union_pins_both_old_source_bindings_across_activation() {
    let (config, _old_pools, _old_files) = config(
        ["http://example.test/left", "http://example.test/right"],
        [&["old-left"], &["old-right"]],
    );
    let config = Arc::new(config);
    let old_lease = config.runtime_lease().unwrap();
    let old_snapshot = old_lease.weak_snapshot();
    let response = router(config.clone())
        .oneshot(request(UNION_SAME_VAR))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let (new_left, _pool, _new_left_file) =
        runtime_source(0, "http://example.test/left", &["new-left"]);
    let (new_right, _pool, _new_right_file) =
        runtime_source(1, "http://example.test/right", &["new-right"]);
    let expected = config.runtime_readiness().unwrap();
    config
        .activate_snapshot(
            expected,
            RuntimeSnapshot::new(Epoch(1), Tbox::default(), vec![new_left, new_right]).unwrap(),
        )
        .unwrap();
    drop(old_lease);
    assert!(
        old_snapshot.upgrade().is_some(),
        "response pins old snapshot"
    );

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let old: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let old = old["results"]["bindings"].as_array().unwrap();
    assert!(old.iter().any(|row| row["value"]["value"] == "old-left"));
    assert!(old.iter().any(|row| row["value"]["value"] == "old-right"));
    assert!(old_snapshot.upgrade().is_none());

    let new = json(config, UNION_SAME_VAR).await;
    let new = new["results"]["bindings"].as_array().unwrap();
    assert!(new.iter().any(|row| row["value"]["value"] == "new-left"));
    assert!(new.iter().any(|row| row["value"]["value"] == "new-right"));
}

#[test]
fn duplicate_ids_and_missing_activation_sources_fail_closed() {
    let (left, _pool, _file) = runtime_source(0, "http://example.test/left", &["one"]);
    let (duplicate, _pool, _file) = runtime_source(0, "http://example.test/right", &["two"]);
    assert!(ServeConfig::new_federated([left, duplicate], Tbox::default()).is_err());

    let (config, _pools, _files) = config(
        ["http://example.test/left", "http://example.test/right"],
        [&["one"], &["two"]],
    );
    let expected = config.runtime_readiness().unwrap();
    let (only, _pool, _file) = runtime_source(0, "http://example.test/left", &["new"]);
    let candidate = RuntimeSnapshot::single(Epoch(1), Tbox::default(), only);
    assert!(matches!(
        config.activate_snapshot(expected, candidate),
        Err(crate::ActivationError::CandidateMissingSource { source_id })
            if source_id == SourceId::new(1).unwrap()
    ));
}
