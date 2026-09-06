use super::*;

fn config() -> ServeConfig {
    ServeConfig::new_with_unverified_source(
        Backend::sqlite(rusqlite::Connection::open_in_memory().expect("open fixture")),
        Vec::new(),
        crate::test_support::empty_ontology(),
        Vec::new(),
    )
    .expect("empty mapping passes semantic admission")
}

#[test]
fn request_admission_defaults_to_one_shared_finite_gate() {
    let config = config();
    assert_eq!(
        config.max_concurrent_requests(),
        DEFAULT_MAX_CONCURRENT_REQUESTS
    );
    assert_eq!(
        config.available_request_permits(),
        DEFAULT_MAX_CONCURRENT_REQUESTS
    );
    assert_eq!(config.max_order_rows(), DEFAULT_MAX_ORDER_ROWS);
    assert_eq!(
        config.query_limits.max_retained_bytes(),
        DEFAULT_MAX_ORDER_BYTES
    );
}

#[test]
fn request_admission_setter_is_checked_and_preserves_state_on_error() {
    let mut config = config();
    config
        .set_max_concurrent_requests(3)
        .expect("finite request ceiling");
    let configured_gate = config.request_admission_permits();
    assert_eq!(config.available_request_permits(), 3);

    for invalid in [0, Semaphore::MAX_PERMITS + 1] {
        let error = config
            .set_max_concurrent_requests(invalid)
            .expect_err("invalid request ceiling");
        assert_eq!(error.code(), "startup-configuration");
        assert!(matches!(
            error.internal_cause(),
            StartupCause::Configuration { .. }
        ));
        assert_eq!(config.max_concurrent_requests(), 3);
        assert!(Arc::ptr_eq(
            &configured_gate,
            &config.request_admission_permits()
        ));
    }
}

#[test]
fn activation_rejects_a_candidate_without_the_selected_source() {
    let config = config();
    let current = config.runtime_readiness().unwrap();
    let selected_source = SourceId::new(0).unwrap();
    let other_source = SourceId::new(1).unwrap();
    let candidate = RuntimeSnapshot::single(
        Epoch(1),
        crate::test_support::empty_ontology(),
        RuntimeSource::new(
            IntrospectedSource::unchecked(
                Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
                Vec::new(),
            ),
            SourceMapping::new(other_source, Vec::new()),
        ),
    )
    .expect("empty replacement mapping passes semantic admission");

    assert!(matches!(
        config.activate_snapshot(current, candidate),
        Err(ActivationError::CandidateMissingSource { source_id })
            if source_id == selected_source
    ));
    assert_eq!(config.runtime_readiness().unwrap(), current);
}

#[test]
fn exactly_one_postgres_direct_lifecycle_can_claim_a_runtime() {
    let config = config();

    assert_eq!(config.claim_pg_direct_lifecycle(), Ok(()));
    assert_eq!(
        config.claim_pg_direct_lifecycle(),
        Err(ReadinessCause::CapabilityDrift)
    );
}
