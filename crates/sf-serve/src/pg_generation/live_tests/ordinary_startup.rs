//! G3 ordinary-mode PostgreSQL startup through the protected profile.
//!
//! Ordinary startup (no `--require-verified-generation`, reload disabled)
//! attempts the verified authored generation when the admission permits it,
//! so a later schema replacement is refused rather than answered. Native RLS
//! bearers keep the unverified path, where the binding still applies RLS.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use super::*;

const MAPPING: &str = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#parent> a rr:TriplesMap; rr:logicalTable [rr:tableName "parent"];
rr:subjectMap [rr:template "http://example.test/parent/{id}"];
rr:predicateObjectMap [rr:predicate <http://example.test/label>; rr:objectMap [rr:column "label"]]."#;
const SELECT: &str = "SELECT ?label WHERE { ?p <http://example.test/label> ?label }";

struct Files(std::path::PathBuf);

impl Drop for Files {
    fn drop(&mut self) {
        // Created exclusively by this module.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn files() -> Files {
    let root = std::env::temp_dir().join(format!("sf-ordinary-pg-g3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("ontology.ttl"),
        "<http://example.test/label> a <http://www.w3.org/1999/02/22-rdf-syntax-ns#Property> .",
    )
    .unwrap();
    std::fs::write(root.join("mapping.ttl"), MAPPING).unwrap();
    Files(root)
}

fn conninfo(config: &Config) -> String {
    let host = match &config.get_hosts()[0] {
        tokio_postgres::config::Host::Tcp(host) => host.clone(),
        #[cfg(unix)]
        tokio_postgres::config::Host::Unix(path) => path.display().to_string(),
    };
    format!(
        "pg:host={host} port={} dbname={} user={} password={}",
        config.get_ports().first().copied().unwrap_or(5432),
        config.get_dbname().unwrap(),
        config.get_user().unwrap(),
        String::from_utf8(config.get_password().unwrap().to_vec()).unwrap(),
    )
}

fn options(files: &Files, admission: crate::QueryAdmission) -> crate::run::ServeOptions {
    crate::run::ServeOptions {
        query_shape_profile: crate::QueryShapeProfile::Ordinary,
        query_admission: admission,
        source: crate::SourceRef::inline("unused: prepared directly"),
        mapping: crate::MappingRef::r2rml_file(files.0.join("mapping.ttl").to_str().unwrap()),
        additional_source: None,
        ontology_path: files.0.join("ontology.ttl").to_string_lossy().into_owned(),
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
        pg_pool_size: 2,
        pg_pool_wait: Duration::from_secs(2),
        sqlite_pool_size: 1,
        shutdown_timeout: Duration::from_secs(1),
        reload_interval: Duration::ZERO,
        require_verified_generation: false,
        metrics: None,
    }
}

async fn config(fixture: &Fixture, opts: &crate::run::ServeOptions) -> Arc<crate::ServeConfig> {
    let source = crate::source::SourceInput::injected(conninfo(&fixture.reader_config()))
        .unwrap()
        .prepare()
        .unwrap();
    let (mut config, _) = crate::startup::build_config(opts, source, None)
        .await
        .expect("ordinary PostgreSQL startup");
    config.use_in_process_test_parser();
    Arc::new(config)
}

fn applies_rls(config: &crate::ServeConfig) -> bool {
    config
        .runtime_lease()
        .expect("lease ordinary snapshot")
        .permits_rls(SourceId::new(0).unwrap())
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
    let Ok(body) = response.into_body().collect().await else {
        return (status, Vec::new());
    };
    let json: serde_json::Value = serde_json::from_slice(&body.to_bytes()).unwrap_or_default();
    let labels = json["results"]["bindings"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["label"]["value"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    (status, labels)
}

pub(super) async fn exercise(fixture: &Arc<Fixture>) {
    let files = files();

    // Native RLS bearer: the unverified path is kept so RLS still applies.
    let rls = crate::QueryAdmission::Bearer(
        crate::BearerQueryAdmission::for_service_principal("fixture-only-ordinary-pg-00000000000")
            .unwrap()
            .with_postgres_rls(
                crate::PostgresRlsClaims::new(std::collections::BTreeMap::from([(
                    "app.subject".into(),
                    "A".into(),
                )]))
                .unwrap(),
            )
            .unwrap(),
    );
    let native = config(fixture, &options(&files, rls)).await;
    assert!(
        applies_rls(&native),
        "a native RLS bearer must keep the RLS-applying unverified generation"
    );
    drop(native);

    // Portable admission: ordinary startup binds the protected generation.
    let opts = options(&files, crate::QueryAdmission::UnrestrictedDevelopment);
    let verified = config(fixture, &opts).await;
    assert!(
        !applies_rls(&verified),
        "ordinary PostgreSQL startup must bind the verified protected generation"
    );
    assert_eq!(
        select(verified.clone()).await,
        (StatusCode::OK, vec!["parent".to_owned()])
    );

    // Replace the mapped table with one whose column now means something else.
    fixture
        .admin
        .batch_execute(&format!(
            "DROP TABLE public.child; DROP TABLE public.parent; \
             CREATE TABLE public.parent (id integer PRIMARY KEY, label text NOT NULL, unit text); \
             INSERT INTO public.parent VALUES (1, 'replaced', 'other'); \
             GRANT SELECT ON public.parent TO {};",
            fixture.role
        ))
        .await
        .expect("replace mapped table");
    let (status, labels) = select(verified).await;
    assert_ne!(
        (status, labels.clone()),
        (StatusCode::OK, vec!["replaced".to_owned()]),
        "ordinary mode must not answer against a replaced table as if it were the observed one"
    );
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "schema replacement after startup must refuse, got {status} {labels:?}"
    );
}
