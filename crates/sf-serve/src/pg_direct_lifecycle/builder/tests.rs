use super::*;
use crate::source::POSTGRES_RELATION_SCOPE_OPTIONS;

fn resolved_config() -> tokio_postgres::Config {
    let mut config = tokio_postgres::Config::new();
    config.options(POSTGRES_RELATION_SCOPE_OPTIONS);
    config
}

fn ontology() -> SemanticOntology {
    crate::test_support::empty_ontology()
}

#[test]
fn closed_spec_mints_distinct_bounded_pools_from_one_resolved_config() {
    let spec = PgDirectLifecycleSpec::from_resolved_config(
        resolved_config(),
        6,
        Duration::from_secs(2),
        ontology(),
        "https://example.test/direct/",
        Duration::from_secs(5),
    )
    .unwrap();

    assert_eq!(spec.pool_capacities(), (6, 1));
}

#[test]
fn closed_spec_rejects_invalid_immutable_inputs_without_source_io() {
    let cases = [
        (
            tokio_postgres::Config::new(),
            6,
            Duration::from_secs(2),
            "https://example.test/direct/",
            Duration::from_secs(5),
        ),
        (
            resolved_config(),
            0,
            Duration::from_secs(2),
            "https://example.test/direct/",
            Duration::from_secs(5),
        ),
        (
            resolved_config(),
            6,
            Duration::ZERO,
            "https://example.test/direct/",
            Duration::from_secs(5),
        ),
        (
            resolved_config(),
            6,
            Duration::from_secs(2),
            "relative-base",
            Duration::from_secs(5),
        ),
        (
            resolved_config(),
            6,
            Duration::from_secs(2),
            "https://example.test/direct/",
            Duration::ZERO,
        ),
    ];

    for (config, size, wait, base, operation) in cases {
        assert!(matches!(
            PgDirectLifecycleSpec::from_resolved_config(
                config,
                size,
                wait,
                ontology(),
                base,
                operation,
            ),
            Err(ReadinessCause::CapabilityDrift)
        ));
    }
}

#[test]
fn generation_failures_map_only_to_the_closed_readiness_algebra() {
    assert_eq!(
        generation_cause(PgGenerationError::SourceUnavailable),
        ReadinessCause::SourceUnavailable
    );
    assert_eq!(
        generation_cause(PgGenerationError::SchemaDrift),
        ReadinessCause::SchemaDrift
    );
    assert_eq!(
        generation_cause(PgGenerationError::CapabilityDrift),
        ReadinessCause::CapabilityDrift
    );
    assert_eq!(
        generation_cause(PgGenerationError::Internal),
        ReadinessCause::CapabilityDrift
    );
}

#[tokio::test]
async fn control_cleanup_retains_capacity_and_observes_only_forced_shutdown() {
    use crate::lifecycle::ShutdownPhase;
    use sf_core::query_control::{QueryControl, QueryControlError};
    let mut spec = PgDirectLifecycleSpec::from_resolved_config(
        resolved_config(),
        1,
        Duration::from_secs(2),
        ontology(),
        "https://example.test/direct/",
        Duration::from_secs(5),
    )
    .unwrap();
    let (signal, receiver) = tokio::sync::watch::channel(ShutdownPhase::Running);
    spec.observe_shutdown(receiver);
    let first = spec.control_budget().unwrap();
    let cleanup = first.clone();
    drop(first);
    assert!(matches!(
        spec.control_budget(),
        Err(ReadinessCause::SourceUnavailable)
    ));
    assert_eq!(spec.control_permits().available_permits(), 0);
    signal.send_replace(ShutdownPhase::Draining);
    assert_eq!(cleanup.checkpoint(), Ok(()));
    signal.send_replace(ShutdownPhase::Forced);
    assert_eq!(cleanup.checkpoint(), Err(QueryControlError::Cancelled));
    drop(cleanup);
    assert_eq!(spec.control_permits().available_permits(), 1);
    assert_eq!(
        spec.control_budget().unwrap().checkpoint(),
        Err(QueryControlError::Cancelled)
    );
}
