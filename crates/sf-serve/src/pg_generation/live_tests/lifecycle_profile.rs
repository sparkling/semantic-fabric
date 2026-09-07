//! Isolated live proof for the dormant PostgreSQL Direct lifecycle builder.

use super::*;
use crate::pg_direct_lifecycle::{
    PgDirectCoordinatorPolicy, PgDirectLifecycleSpec, PgDirectLifecycleSupervisor,
};

const BASE: &str = "http://example.test/base/";

pub(super) async fn exercise(fixture: &Arc<Fixture>) {
    assert_qualified_patch(fixture).await;
    let spec = PgDirectLifecycleSpec::from_resolved_config(
        fixture.reader_config(),
        3,
        Duration::from_secs(2),
        request_route::direct_ontology(),
        BASE,
        Duration::from_secs(15),
    )
    .expect("construct the immutable closed lifecycle profile");

    assert_eq!(spec.pool_capacities(), (3, 1));
    assert_distinct_pool_backends(&spec).await;

    let initial = spec
        .build_initial()
        .await
        .expect("build a complete isolated initial candidate");
    assert_eq!(initial.snapshot().registry().len(), 1);
    assert_eq!(
        initial
            .snapshot()
            .registry()
            .schema(SourceId::new(0).unwrap())
            .expect("single source schema")
            .len(),
        2
    );
    spec.probe(initial.expectation())
        .await
        .expect("reobserve the exact initial generation");

    assert_profile_mutation_rejected(
        fixture,
        &spec,
        "CREATE TABLE public.no_primary_key (value integer NOT NULL)",
        "DROP TABLE public.no_primary_key",
    )
    .await;
    assert_profile_mutation_rejected(
        fixture,
        &spec,
        "ALTER TABLE public.parent ENABLE ROW LEVEL SECURITY",
        "ALTER TABLE public.parent DISABLE ROW LEVEL SECURITY",
    )
    .await;
    assert_profile_mutation_rejected(
        fixture,
        &spec,
        "CREATE UNLOGGED TABLE public.not_permanent (id integer PRIMARY KEY)",
        "DROP TABLE public.not_permanent",
    )
    .await;

    fixture
        .admin
        .batch_execute(
            "ALTER TABLE public.parent ADD CONSTRAINT parent_label_successor UNIQUE (label)",
        )
        .await
        .expect("create isolated successor generation");
    assert_eq!(
        spec.probe(initial.expectation()).await,
        Err(crate::ReadinessCause::SchemaDrift)
    );
    let successor = spec
        .build_initial()
        .await
        .expect("fully validate the isolated successor candidate");
    spec.probe(successor.expectation())
        .await
        .expect("the successor generation reobserves exactly");
    fixture
        .admin
        .batch_execute("ALTER TABLE public.parent DROP CONSTRAINT parent_label_successor")
        .await
        .expect("restore isolated initial generation");
    assert_eq!(
        spec.probe(successor.expectation()).await,
        Err(crate::ReadinessCause::SchemaDrift)
    );
    let rebuilt = spec
        .build_initial()
        .await
        .expect("only a fully rebuilt candidate establishes the successor generation");
    spec.probe(rebuilt.expectation())
        .await
        .expect("the rebuilt successor generation reobserves exactly");

    let (config, expectation) = crate::ServeConfig::from_initial_pg_direct(rebuilt);
    let config = Arc::new(config);
    request_route::exercise_config(fixture, &config).await;
    let policy = PgDirectCoordinatorPolicy::for_spec(Duration::from_secs(60), &spec)
        .expect("bind coordinator timing to the immutable profile");
    let supervisor =
        PgDirectLifecycleSupervisor::start(Arc::clone(&config), spec, expectation, policy)
            .expect("claim the sole lifecycle worker");
    supervisor
        .shutdown()
        .await
        .expect("planned shutdown fences, stops, and joins");
    assert!(matches!(
        config.runtime_readiness().unwrap(),
        crate::RuntimeReadiness::NotReady {
            cause: crate::ReadinessCause::Administrative,
            ..
        }
    ));
}

async fn assert_qualified_patch(fixture: &Fixture) {
    let version: i32 = fixture
        .admin
        .query_one("SELECT current_setting('server_version_num')::integer", &[])
        .await
        .expect("read isolated PostgreSQL patch")
        .get(0);
    assert!(
        matches!(version, 160_009 | 160_015),
        "live lifecycle evidence requires PostgreSQL 16.9 or 16.15, got {version}"
    );
}

async fn assert_distinct_pool_backends(spec: &PgDirectLifecycleSpec) {
    let pools = spec.pools_for_test();
    let request_pool = pools.request();
    let control_pool = pools.control();
    let request = request_pool.get().await.expect("request-pool member");
    let control = control_pool.get().await.expect("control-pool member");
    let request_pid: i32 = request
        .query_one("SELECT pg_catalog.pg_backend_pid()", &[])
        .await
        .expect("request backend identity")
        .get(0);
    let control_pid: i32 = control
        .query_one("SELECT pg_catalog.pg_backend_pid()", &[])
        .await
        .expect("control backend identity")
        .get(0);
    assert_ne!(request_pid, control_pid);
    assert_eq!(control_pool.status().max_size, 1);
}

async fn assert_profile_mutation_rejected(
    fixture: &Fixture,
    spec: &PgDirectLifecycleSpec,
    apply: &str,
    restore: &str,
) {
    fixture
        .admin
        .batch_execute(apply)
        .await
        .expect("apply isolated excluded-profile mutation");
    let result = spec.build_initial().await;
    fixture
        .admin
        .batch_execute(restore)
        .await
        .expect("restore isolated excluded-profile mutation");
    assert!(matches!(
        result,
        Err(crate::ReadinessCause::CapabilityDrift)
    ));
}
