//! Startup-boundary tests for the aggregate serving-request ceiling.

use std::time::Duration;

use sf_serve::{serve_blocking, ServeOptions, SourceRef};
use tokio::sync::Semaphore;

fn options(max_concurrent_requests: usize) -> ServeOptions {
    ServeOptions {
        source: SourceRef::environment("SF_REQUEST_ADMISSION_MUST_NOT_BE_READ"),
        mapping_path: "/mapping/that/must/not/be/read.ttl".to_owned(),
        ontology_path: Some("/ontology/that/must/not/be/read.ttl".to_owned()),
        bind: "203.0.113.1:1".to_owned(),
        timeout: Duration::from_secs(1),
        max_query_len: 1024,
        max_concurrent_requests,
        max_source_work: 1,
        max_result_items: 1,
        max_order_rows: 1,
        max_order_bytes: 1,
        max_serialized_bytes: 1,
        pg_pool_size: 1,
        pg_pool_wait: Duration::from_secs(1),
        sqlite_pool_size: 1,
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
