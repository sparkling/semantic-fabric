//! G3 ordinary-mode coherence for file-backed SQLite (user decision 2A,
//! option 1: fail closed without a reload dependency).
//!
//! Ordinary startup observes the schema once and compiles against it. A
//! schema change on another connection must never let a later request answer
//! against the new table as if it were the old one: that request must refuse
//! with a typed problem instead of returning HTTP 200 with wrong values.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

const MAPPING: &str = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#people> a rr:TriplesMap; rr:logicalTable [rr:tableName "people"];
rr:subjectMap [rr:template "http://example.test/person/{id}"];
rr:predicateObjectMap [rr:predicate <http://example.test/age>; rr:objectMap [rr:column "age"]]."#;
const SELECT: &str = "SELECT ?age WHERE { ?p <http://example.test/age> ?age }";

struct Fixture {
    root: std::path::PathBuf,
    writer: rusqlite::Connection,
    opts: crate::run::ServeOptions,
}

impl Fixture {
    fn new(journal: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "sf-ordinary-g3-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let db = root.join("source.db");
        let writer = rusqlite::Connection::open(&db).unwrap();
        writer
            .execute_batch(&format!(
                "PRAGMA journal_mode={journal}; \
                 CREATE TABLE people(id INTEGER PRIMARY KEY, age INTEGER NOT NULL); \
                 INSERT INTO people VALUES(1,30),(2,25);"
            ))
            .unwrap();
        std::fs::write(
            root.join("ontology.ttl"),
            "<http://example.test/age> a <http://www.w3.org/1999/02/22-rdf-syntax-ns#Property> .",
        )
        .unwrap();
        std::fs::write(root.join("mapping.ttl"), MAPPING).unwrap();
        // ORDINARY mode: no --require-verified-generation, and reload
        // disabled, so recovery cannot come from a background supervisor.
        let opts = crate::run::ServeOptions {
            query_shape_profile: crate::QueryShapeProfile::Ordinary,
            query_admission: crate::QueryAdmission::UnrestrictedDevelopment,
            source: crate::SourceRef::inline(format!("sqlite:{}", db.display())),
            mapping: crate::MappingRef::r2rml_file(root.join("mapping.ttl").to_str().unwrap()),
            additional_source: None,
            ontology_path: root.join("ontology.ttl").to_string_lossy().into_owned(),
            bind: "127.0.0.1:0".into(),
            timeout: Duration::from_secs(5),
            max_query_len: 4096,
            max_concurrent_requests: 8,
            max_compiler_work: crate::DEFAULT_QUERY_LIMITS.max_compiler_work(),
            max_source_work: crate::DEFAULT_QUERY_LIMITS.max_source_work(),
            max_result_items: 1000,
            max_order_rows: 100,
            max_order_bytes: 4096,
            max_serialized_bytes: 1 << 20,
            pg_pool_size: 1,
            pg_pool_wait: Duration::from_secs(1),
            sqlite_pool_size: 1,
            shutdown_timeout: Duration::from_secs(1),
            reload_interval: Duration::ZERO,
            require_verified_generation: false,
            metrics: None,
        };
        Self { root, writer, opts }
    }

    /// The real ordinary startup path, exactly as `sf-cli serve` runs it.
    async fn config(&self) -> Arc<crate::ServeConfig> {
        let source = self.opts.source.resolve().unwrap().prepare().unwrap();
        let (mut config, _) = crate::startup::build_config(&self.opts, source, None)
            .await
            .unwrap();
        config.use_in_process_test_parser();
        Arc::new(config)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Created exclusively by this fixture.
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

async fn select(config: Arc<crate::ServeConfig>) -> (StatusCode, Vec<String>) {
    let response = crate::router(config)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/sparql")
                .header(header::CONTENT_TYPE, "application/sparql-query")
                .header(header::ACCEPT, "application/sparql-results+json")
                .body(Body::from(SELECT))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await;
    let Ok(body) = body else {
        return (status, Vec::new());
    };
    let json: serde_json::Value = serde_json::from_slice(&body.to_bytes()).unwrap_or_default();
    let mut ages: Vec<String> = json["results"]["bindings"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["age"]["value"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    ages.sort();
    (status, ages)
}

/// Replace the table under the same name and column, with a column that now
/// means something else. Compiled SQL still runs, so without generation
/// coherence the request would return 200 with values the mapping never meant.
fn replace_with_semantically_different_table(writer: &rusqlite::Connection) {
    writer
        .execute_batch(
            "DROP TABLE people; \
             CREATE TABLE people(id INTEGER PRIMARY KEY, age INTEGER NOT NULL, unit TEXT); \
             INSERT INTO people VALUES(1,360,'months'),(2,300,'months');",
        )
        .unwrap();
}

async fn schema_change_after_startup_is_refused_not_answered(journal: &str) {
    let fixture = Fixture::new(journal);
    let config = fixture.config().await;
    let (status, ages) = select(config.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        ages,
        ["25", "30"],
        "baseline answers from the observed table"
    );

    replace_with_semantically_different_table(&fixture.writer);

    let (status, ages) = select(config).await;
    assert_ne!(
        (status, ages.clone()),
        (StatusCode::OK, vec!["300".to_owned(), "360".to_owned()]),
        "ordinary mode must not answer against a replaced table as if it were the observed one"
    );
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a schema change after startup must refuse with the typed source-generation problem, got {status} {ages:?}"
    );
}

#[tokio::test]
async fn wal_schema_change_after_startup_is_refused_not_answered() {
    schema_change_after_startup_is_refused_not_answered("WAL").await;
}

#[tokio::test]
async fn delete_journal_schema_change_after_startup_is_refused_not_answered() {
    schema_change_after_startup_is_refused_not_answered("DELETE").await;
}

/// Coherence must not cost ordinary availability when nothing changed.
#[tokio::test]
async fn unchanged_schema_keeps_answering_across_requests_and_data_writes() {
    let fixture = Fixture::new("WAL");
    let config = fixture.config().await;
    for _ in 0..3 {
        assert_eq!(
            select(config.clone()).await,
            (StatusCode::OK, vec!["25".into(), "30".into()])
        );
    }
    // A data-only write changes values but not the schema: still answered, freshly.
    fixture
        .writer
        .execute_batch("UPDATE people SET age = age + 1;")
        .unwrap();
    assert_eq!(
        select(config).await,
        (StatusCode::OK, vec!["26".into(), "31".into()])
    );
}

/// Build the real ordinary startup and report whether it came up, plus what
/// the first query answers. These cases must keep STARTING exactly as before
/// this change: the sealed lease declines them, and ordinary mode falls back.
async fn starts_and_answers(fixture: &Fixture) -> (StatusCode, Vec<String>) {
    let source = fixture.opts.source.resolve().unwrap().prepare().unwrap();
    let (mut config, _) = crate::startup::build_config(&fixture.opts, source, None)
        .await
        .expect("an ordinary source that started before G3 must still start");
    config.use_in_process_test_parser();
    select(Arc::new(config)).await
}

/// An unrelated full-text index anywhere in `main` makes the sealed profile
/// decline (virtual tables may run external code outside the snapshot). That
/// must not stop an ordinary deployment from starting.
#[tokio::test]
async fn unrelated_virtual_table_falls_back_and_still_serves() {
    let fixture = Fixture::new("WAL");
    fixture
        .writer
        .execute_batch("CREATE VIRTUAL TABLE notes USING fts5(body);")
        .unwrap();
    assert_eq!(
        starts_and_answers(&fixture).await,
        (StatusCode::OK, vec!["25".into(), "30".into()])
    );
}

/// A `rr:tableName` naming a view was already refused by ordinary startup
/// before G3 (verified against HEAD 522bf9c0, which fails the same way), so it
/// is not a regression surface for this change. Pin that it still refuses with
/// the same typed configuration error rather than some new failure.
#[tokio::test]
async fn view_named_by_table_name_is_still_refused_as_before() {
    let fixture = Fixture::new("WAL");
    fixture
        .writer
        .execute_batch(
            "ALTER TABLE people RENAME TO people_base; \
             CREATE VIEW people AS SELECT id, age FROM people_base;",
        )
        .unwrap();
    let source = fixture.opts.source.resolve().unwrap().prepare().unwrap();
    let error = crate::startup::build_config(&fixture.opts, source, None)
        .await
        .err()
        .expect("a view-named rr:tableName was refused before G3 and still is");
    assert_eq!(error.code(), "startup-configuration");
}

/// A journal mode the sealed profile does not qualify never attempts it.
#[tokio::test]
async fn unqualified_journal_mode_keeps_the_unverified_path() {
    let fixture = Fixture::new("TRUNCATE");
    assert_eq!(
        starts_and_answers(&fixture).await,
        (StatusCode::OK, vec!["25".into(), "30".into()])
    );
}

#[path = "ordinary_sqlite_generation_profile_tests.rs"]
mod ordinary_profile;
