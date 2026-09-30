//! Startup-boundary tests for the aggregate serving-request ceiling.

use std::time::Duration;

use sf_serve::{serve_blocking, AdditionalSourceOptions, MappingRef, ServeOptions, SourceRef};
use tokio::sync::Semaphore;

fn options(max_concurrent_requests: usize) -> ServeOptions {
    ServeOptions {
        query_shape_profile: sf_serve::QueryShapeProfile::Ordinary,
        query_admission: sf_serve::QueryAdmission::Deny,
        source: SourceRef::environment("SF_REQUEST_ADMISSION_MUST_NOT_BE_READ"),
        mapping: MappingRef::r2rml_file("/mapping/that/must/not/be/read.ttl"),
        additional_source: None,
        ontology_path: "/ontology/that/must/not/be/read.ttl".to_owned(),
        bind: "203.0.113.1:1".to_owned(),
        timeout: Duration::from_secs(1),
        max_query_len: 1024,
        max_concurrent_requests,
        max_compiler_work: 1,
        max_source_work: 1,
        max_result_items: 1,
        max_order_rows: 1,
        max_order_bytes: 1,
        max_serialized_bytes: 1,
        pg_pool_size: 1,
        pg_pool_wait: Duration::from_secs(1),
        sqlite_pool_size: 1,
        shutdown_timeout: std::time::Duration::from_secs(30),
        reload_interval: Duration::ZERO,
        require_verified_generation: false,
        metrics: None,
    }
}

#[test]
fn invalid_request_ceiling_fails_before_source_file_runtime_or_network_io() {
    for invalid in [0, Semaphore::MAX_PERMITS + 1] {
        let error = serve_blocking(options(invalid))
            .expect_err("request-admission configuration must fail first");
        assert_eq!(error.code(), "startup-configuration");
    }
}

#[test]
fn invalid_reload_interval_fails_before_source_file_runtime_or_network_io() {
    for interval in [Duration::from_nanos(1), Duration::from_secs(86_401)] {
        let mut opts = options(1);
        opts.reload_interval = interval;
        assert_eq!(
            serve_blocking(opts).unwrap_err().code(),
            "startup-configuration"
        );
    }
}

#[test]
fn required_generation_rejects_unsupported_modes_before_source_or_file_io() {
    for mode in ["no-reload", "direct", "second-direct", "row-policy"] {
        let mut opts = options(1);
        opts.require_verified_generation = true;
        if mode != "no-reload" {
            opts.reload_interval = Duration::from_secs(1);
        }
        match mode {
            "direct" => opts.mapping = MappingRef::direct("http://example.test/"),
            "second-direct" => {
                opts.additional_source = Some(AdditionalSourceOptions {
                    source: SourceRef::environment("SF_GENERATION_MUST_NOT_BE_READ"),
                    mapping: MappingRef::direct("http://example.test/second/"),
                })
            }
            "row-policy" => {
                opts.query_admission = sf_serve::QueryAdmission::Bearer(
                    sf_serve::BearerQueryAdmission::for_service_principal(
                        "fixture-only-generation-000000000000",
                    )
                    .unwrap()
                    .with_postgres_rls(
                        sf_serve::PostgresRlsClaims::new(std::collections::BTreeMap::from([(
                            "app.subject".into(),
                            "A".into(),
                        )]))
                        .unwrap(),
                    )
                    .unwrap(),
                );
            }
            _ => {}
        }
        assert_eq!(
            serve_blocking(opts).unwrap_err().code(),
            "startup-configuration",
            "{mode}"
        );
    }
    for source in [
        "sqlite::memory:",
        "sqlite:file:/must/not/be/opened.db?immutable=1",
        "sqlite:file:/must/not/be/opened.db?nolock=1",
    ] {
        let mut opts = options(1);
        opts.require_verified_generation = true;
        opts.reload_interval = Duration::from_secs(1);
        opts.source = SourceRef::inline(source);
        assert_eq!(
            serve_blocking(opts).unwrap_err().code(),
            "startup-configuration"
        );
    }
}

#[test]
fn exact_duplicate_source_references_fail_before_files_or_connectors() {
    let mut opts = options(1);
    opts.source = SourceRef::inline("sqlite:/must/not/be/opened.db");
    opts.additional_source = Some(AdditionalSourceOptions {
        source: SourceRef::inline("sqlite:/must/not/be/opened.db"),
        mapping: MappingRef::r2rml_file("/second/mapping/must/not/be/read.ttl"),
    });

    let error = serve_blocking(opts).expect_err("duplicate source references must fail closed");
    assert_eq!(error.code(), "startup-configuration");
}
