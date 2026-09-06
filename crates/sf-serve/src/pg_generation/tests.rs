use super::*;
use crate::source::POSTGRES_RELATION_SCOPE_SETTING;

fn valid_context() -> PgSessionContext {
    PgSessionContext {
        database_oid: 1,
        database_name: "semantic_fabric_test".to_owned(),
        current_role_oid: 2,
        current_role_name: "semantic_fabric_reader".to_owned(),
        session_role_oid: 2,
        session_role_name: "semantic_fabric_reader".to_owned(),
        server_version_num: 160_009,
        search_path: POSTGRES_RELATION_SCOPE_SETTING.to_owned(),
        row_security: "on".to_owned(),
        session_replication_role: "origin".to_owned(),
    }
}

#[test]
fn session_context_rejects_role_or_policy_drift() {
    let valid = valid_context();
    assert!(validate_session_context(&valid, false, false).is_ok());

    let mut changed = valid.clone();
    changed.session_role_oid += 1;
    assert!(validate_session_context(&changed, false, false).is_err());
    assert!(validate_session_context(&valid, true, false).is_err());
    assert!(validate_session_context(&valid, false, true).is_err());
}

#[test]
fn transaction_timeouts_are_finite_and_numeric() {
    let budget = RequestBudget::after(
        Duration::from_secs(2),
        sf_core::query_control::QueryLimits::new(1, 100, 1, 1),
    );
    let sql = transaction_setup_sql(&budget).unwrap();
    assert!(sql.starts_with(BEGIN_GENERATION_SQL));
    assert!(sql.contains("SET LOCAL statement_timeout = "));
    assert!(sql.contains("SET LOCAL lock_timeout = 1000;"));
    assert!(!sql.contains('\''));
}

#[test]
fn live_postgres_profile_rejects_the_untyped_rowid_sentinel() {
    for name in ["rowid", "ROWID", "RowId"] {
        let mut table = TableSchema::new("items");
        table.columns = vec![sf_core::Column::new(name, "integer", true)];
        table.primary_key = vec![name.to_owned()];
        assert!(!postgres_direct_table_profile_is_unambiguous(&[table]));
    }

    let mut ordinary = TableSchema::new("items");
    ordinary.columns = vec![sf_core::Column::new("id", "integer", true)];
    ordinary.primary_key = vec!["id".to_owned()];
    assert!(postgres_direct_table_profile_is_unambiguous(&[ordinary]));
}

#[test]
fn generation_lease_is_pinned_to_the_exact_binding_identity() {
    let source_id = SourceId::new(0).unwrap();
    let binding_identity = RuntimeBindingIdentity::fresh();
    let same_binding = binding_identity.clone();
    let other_binding = RuntimeBindingIdentity::fresh();
    let lease = VerifiedPostgresGenerationLease {
        source_id,
        conn: None,
    };
    let mut generations = VerifiedGenerationLeases {
        leases: BTreeMap::from([(source_id, (binding_identity, lease))]),
    };

    assert!(!generations.contains(source_id, &other_binding));
    assert!(generations.take(source_id, &other_binding).is_none());
    assert!(
        !generations.is_empty(),
        "mismatch must retain cleanup ownership"
    );
    assert!(generations.contains(source_id, &same_binding));
    assert_eq!(
        generations
            .take(source_id, &same_binding)
            .unwrap()
            .source_id(),
        source_id
    );
    assert!(generations.is_empty());
}

#[test]
fn generation_requirement_preserves_the_binding_identity_without_io() {
    use sf_core::schema_identity::{ProfileIdV1, SchemaObservationInputV1, SchemaProfilesV1};

    let profile = |value| ProfileIdV1::new(value).unwrap();
    let observed_identity = ObservedSchemaIdentityV1::build(SchemaObservationInputV1 {
        profiles: SchemaProfilesV1 {
            structural: profile("test-structural-v1"),
            types: profile("test-types-v1"),
            constraints: profile("test-constraints-v1"),
        },
        relations: Vec::new(),
        constraints: Vec::new(),
    })
    .unwrap();
    let source_id = SourceId::new(0).unwrap();
    let generation = SourceGeneration::direct_postgres(PostgresDirectGeneration {
        source_id,
        base_iri: Arc::from("http://example.test/"),
        row_identity: sf_mapping::DirectMappingRowIdentity::RequirePrimaryKey,
        identity: observed_identity,
        session: valid_context(),
        table_names: Arc::from(Vec::<String>::new()),
    });
    let pg_config: tokio_postgres::Config = "host=127.0.0.1 port=1".parse().unwrap();
    let pool = deadpool_postgres::Pool::builder(deadpool_postgres::Manager::new(
        pg_config,
        tokio_postgres::NoTls,
    ))
    .max_size(1)
    .build()
    .unwrap();
    let binding_identity = RuntimeBindingIdentity::fresh();
    let requirement = generation
        .requirement(&crate::Backend::Pg(pool), &binding_identity)
        .unwrap()
        .unwrap();

    assert!(requirement.binding_identity.ptr_eq(&binding_identity));
    assert!(!requirement
        .binding_identity
        .ptr_eq(&RuntimeBindingIdentity::fresh()));
}
