use super::*;
use sf_core::query_control::{QueryControl, UncontrolledQueryControl};

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

#[test]
fn request_budgets_retain_the_runtime_identity_of_their_config_snapshot() {
    let executable = std::env::current_exe().expect("current test executable");
    let runtime_a = sf_sparql::ParserRuntime::prepare_identity_for_evidence(&executable)
        .expect("first runtime identity");
    let runtime_b = sf_sparql::ParserRuntime::prepare_identity_for_evidence(&executable)
        .expect("second runtime identity");
    assert!(!runtime_a.same_instance_for_evidence(&runtime_b));

    let mut active = config();
    active.set_parser_runtime(runtime_a.clone());
    let budget_a = active.request_budget();
    active.set_parser_runtime(runtime_b.clone());
    let budget_b = active.request_budget();

    fn held(budget: &RequestBudget) -> &sf_sparql::ParserRuntime {
        budget
            .capability(std::any::TypeId::of::<sf_sparql::ParserRuntime>())
            .and_then(|value| value.downcast_ref::<sf_sparql::ParserRuntime>())
            .expect("request runtime capability")
    }
    assert!(held(&budget_a).same_instance_for_evidence(&runtime_a));
    assert!(held(&budget_b).same_instance_for_evidence(&runtime_b));
    assert!(!held(&budget_a).same_instance_for_evidence(held(&budget_b)));

    std::thread::scope(|scope| {
        let first = scope.spawn(|| held(&budget_a).same_instance_for_evidence(&runtime_a));
        let second = scope.spawn(|| held(&budget_b).same_instance_for_evidence(&runtime_b));
        assert!(first.join().expect("first request identity thread"));
        assert!(second.join().expect("second request identity thread"));
    });

    let raw = UncontrolledQueryControl;
    assert!(raw
        .capability(std::any::TypeId::of::<sf_sparql::ParserRuntime>())
        .is_none());
    assert_eq!(
        sf_sparql::exercise_raw_sql_fallback_for_evidence(&raw)
            .expect("raw emission uses the in-process fallback"),
        "SELECT 1 AS c0"
    );
}
