use super::*;
use crate::{QueryAdmission, QueryShapeProfile, SourceRef};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn common_startup_configuration_keeps_shape_and_subject_axes_separate() {
    for direct in [false, true] {
        for profile in [
            QueryShapeProfile::Ordinary,
            QueryShapeProfile::GeneratedSelectAsk,
        ] {
            for denied in [false, true] {
                let (mut config, _) = crate::generated_http_test_support::config(
                    QueryShapeProfile::Ordinary,
                    QueryAdmission::UnrestrictedDevelopment,
                );
                let opts = ServeOptions {
                    query_admission: if denied {
                        QueryAdmission::Deny
                    } else {
                        QueryAdmission::UnrestrictedDevelopment
                    },
                    query_shape_profile: profile,
                    source: SourceRef::environment("SF_PROFILE_SOURCE_MUST_NOT_BE_READ"),
                    mapping: if direct {
                        MappingRef::direct("https://example.test/")
                    } else {
                        MappingRef::r2rml_file("/mapping/must/not/be/read")
                    },
                    additional_source: None,
                    ontology_path: "/ontology/must/not/be/read".into(),
                    bind: "127.0.0.1:0".into(),
                    timeout: Duration::from_secs(5),
                    max_query_len: 4096,
                    max_concurrent_requests: 8,
                    max_compiler_work: crate::DEFAULT_QUERY_LIMITS.max_compiler_work(),
                    max_source_work: crate::DEFAULT_QUERY_LIMITS.max_source_work(),
                    max_result_items: 1000,
                    max_order_rows: 100,
                    max_order_bytes: 4096,
                    max_serialized_bytes: 4096,
                    pg_pool_size: 1,
                    pg_pool_wait: Duration::from_secs(1),
                    sqlite_pool_size: 1,
                    shutdown_timeout: Duration::from_secs(1),
                    reload_interval: Duration::ZERO,
                    require_verified_generation: false,
                    metrics: None,
                };
                let config = Arc::get_mut(&mut config).unwrap();
                configure(&opts, config).unwrap();
                assert_eq!(config.query_shape_profile(), profile);
                assert_eq!(
                    matches!(config.query_admission, QueryAdmission::Deny),
                    denied
                );
            }
        }
    }
}
