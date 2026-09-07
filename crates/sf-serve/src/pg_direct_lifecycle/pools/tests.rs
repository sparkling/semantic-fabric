use super::*;

fn resolved_config() -> tokio_postgres::Config {
    let mut config = tokio_postgres::Config::new();
    config.options(POSTGRES_RELATION_SCOPE_OPTIONS);
    config
}

#[test]
fn constructs_independent_request_and_single_member_control_pools() {
    let pools =
        PgDirectPools::from_resolved_config(resolved_config(), 4, Duration::from_secs(1)).unwrap();
    assert_eq!(pools.capacities(), (4, CONTROL_POOL_SIZE));

    let request = pools.request();
    let control = pools.control();
    request.close();
    assert!(request.is_closed());
    assert!(
        !control.is_closed(),
        "control must not clone request pool state"
    );
}

#[test]
fn rejects_unresolved_scope_and_unbounded_pool_inputs_without_io() {
    for (config, size, wait) in [
        (tokio_postgres::Config::new(), 4, Duration::from_secs(1)),
        (resolved_config(), 0, Duration::from_secs(1)),
        (resolved_config(), 4, Duration::ZERO),
    ] {
        assert!(matches!(
            PgDirectPools::from_resolved_config(config, size, wait),
            Err(PgDirectPoolError::InvalidConfiguration)
        ));
    }
}
