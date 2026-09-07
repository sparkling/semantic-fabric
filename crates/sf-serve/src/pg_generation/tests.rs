use super::context::{context_read, validate_runtime_role, PgRuntimeRoleFacts};
use super::*;
use crate::source::POSTGRES_GENERATION_SCOPE_SETTING;

fn valid_context() -> PgSessionContext {
    PgSessionContext {
        database_oid: 1,
        database_name: "semantic_fabric_test".to_owned(),
        current_role_oid: 2,
        current_role_name: "semantic_fabric_reader".to_owned(),
        session_role_oid: 2,
        session_role_name: "semantic_fabric_reader".to_owned(),
        server_version_num: 160_009,
        search_path: POSTGRES_GENERATION_SCOPE_SETTING.to_owned(),
        row_security: "on".to_owned(),
        session_replication_role: "origin".to_owned(),
    }
}

fn valid_runtime_role() -> PgRuntimeRoleFacts {
    PgRuntimeRoleFacts {
        superuser: false,
        inherits_privileges: false,
        creates_roles: false,
        creates_databases: false,
        replicates: false,
        bypasses_row_security: false,
        has_role_memberships: false,
        owns_database: false,
        owns_public_schema: false,
        owns_mapped_table: false,
        database_connect: true,
        database_create: false,
        database_temporary: false,
        schema_usage: true,
        schema_create: false,
        every_mapped_table_select: true,
        any_mapped_table_mutation: false,
        collation_probe_execute: true,
    }
}

fn assert_runtime_role_drift(change: impl FnOnce(&mut PgRuntimeRoleFacts)) {
    let mut facts = valid_runtime_role();
    change(&mut facts);
    assert!(matches!(
        validate_runtime_role(&facts),
        Err(PgGenerationError::CapabilityDrift)
    ));
}

fn observed_identity() -> ObservedSchemaIdentityV1 {
    use sf_core::schema_identity::{ProfileIdV1, SchemaObservationInputV1, SchemaProfilesV1};

    let profile = |value| ProfileIdV1::new(value).unwrap();
    ObservedSchemaIdentityV1::build(SchemaObservationInputV1 {
        profiles: SchemaProfilesV1 {
            structural: profile("test-structural-v1"),
            types: profile("test-types-v1"),
            constraints: profile("test-constraints-v1"),
        },
        relations: Vec::new(),
        constraints: Vec::new(),
    })
    .unwrap()
}

fn direct_generation(source_id: SourceId) -> Arc<PostgresDirectGeneration> {
    Arc::new(PostgresDirectGeneration {
        source_id,
        base_iri: Arc::from("http://example.test/"),
        row_identity: sf_mapping::DirectMappingRowIdentity::RequirePrimaryKey,
        mapping_digest: MappingDigest::from_mapping(&SourceMapping::new(source_id, Vec::new())),
        identity: observed_identity(),
        session: valid_context(),
        tables: Arc::from(Vec::<TableSchema>::new()),
    })
}

#[test]
fn session_context_rejects_role_or_policy_drift() {
    let valid = valid_context();
    assert!(validate_session_context(&valid, 42).is_ok());

    let mut changed = valid.clone();
    changed.session_role_oid += 1;
    assert!(matches!(
        validate_session_context(&changed, 42),
        Err(PgGenerationError::CapabilityDrift)
    ));
    assert!(matches!(
        validate_session_context(&valid, 0),
        Err(PgGenerationError::CapabilityDrift)
    ));
}

#[test]
fn unreadable_session_context_is_source_unavailable() {
    assert!(matches!(
        context_read::<(), _>(Err("transport or row-decode failure")),
        Err(PgGenerationError::SourceUnavailable)
    ));
}

#[test]
fn runtime_role_requires_the_exact_restricted_privilege_boundary() {
    assert!(validate_runtime_role(&valid_runtime_role()).is_ok());

    assert_runtime_role_drift(|facts| facts.superuser = true);
    assert_runtime_role_drift(|facts| facts.inherits_privileges = true);
    assert_runtime_role_drift(|facts| facts.creates_roles = true);
    assert_runtime_role_drift(|facts| facts.creates_databases = true);
    assert_runtime_role_drift(|facts| facts.replicates = true);
    assert_runtime_role_drift(|facts| facts.bypasses_row_security = true);
    assert_runtime_role_drift(|facts| facts.has_role_memberships = true);
    assert_runtime_role_drift(|facts| facts.owns_database = true);
    assert_runtime_role_drift(|facts| facts.owns_public_schema = true);
    assert_runtime_role_drift(|facts| facts.owns_mapped_table = true);
    assert_runtime_role_drift(|facts| facts.database_connect = false);
    assert_runtime_role_drift(|facts| facts.database_create = true);
    assert_runtime_role_drift(|facts| facts.database_temporary = true);
    assert_runtime_role_drift(|facts| facts.schema_usage = false);
    assert_runtime_role_drift(|facts| facts.schema_create = true);
    assert_runtime_role_drift(|facts| facts.every_mapped_table_select = false);
    assert_runtime_role_drift(|facts| facts.any_mapped_table_mutation = true);
    assert_runtime_role_drift(|facts| facts.collation_probe_execute = false);
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
    assert!(sql.contains("SET LOCAL search_path = pg_catalog, public, pg_temp;"));
    assert!(!sql.contains("SELECT"));
    assert!(sql.contains("SET LOCAL row_security = on;"));
    assert!(!sql.contains("statement_timeout = '"));
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
    let lease = VerifiedPostgresGenerationLease::without_connection(
        source_id,
        direct_generation(source_id),
    );
    let mut generations = VerifiedGenerationLeases {
        leases: BTreeMap::from([(source_id, (binding_identity, lease))]),
    };

    assert!(!generations.matches(source_id, &other_binding, true));
    assert!(!generations.matches(source_id, &other_binding, false));
    assert!(generations.take(source_id, &other_binding).is_none());
    assert!(
        !generations.is_empty(),
        "mismatch must retain cleanup ownership"
    );
    assert!(generations.matches(source_id, &same_binding, true));
    assert_eq!(
        generations
            .take(source_id, &same_binding)
            .unwrap()
            .source_id(),
        source_id
    );
    assert!(generations.is_empty());
    assert!(generations.matches(source_id, &same_binding, false));
}

#[test]
fn generation_requirement_preserves_the_binding_identity_without_io() {
    let source_id = SourceId::new(0).unwrap();
    let generation = SourceGeneration::direct_postgres(direct_generation(source_id));
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
        .requirement(&crate::Backend::Pg(pool.into()), &binding_identity)
        .unwrap()
        .unwrap();

    assert!(requirement.binding_identity.ptr_eq(&binding_identity));
    assert!(!requirement
        .binding_identity
        .ptr_eq(&RuntimeBindingIdentity::fresh()));
}

#[tokio::test]
async fn generation_metadata_reservation_rejects_one_short_before_pool_io() {
    use sf_core::query_control::{QueryCharge, QueryControlError, QueryLimits};

    let pg_config: tokio_postgres::Config = "host=127.0.0.1 port=1".parse().unwrap();
    let pool = deadpool_postgres::Pool::builder(deadpool_postgres::Manager::new(
        pg_config,
        tokio_postgres::NoTls,
    ))
    .max_size(1)
    .build()
    .unwrap();
    pool.close();
    let pool: crate::PostgresPool = pool.into();
    let budget = RequestBudget::after(
        Duration::from_secs(1),
        QueryLimits::new(1, GENERATION_METADATA_PROBE_RESERVATION - 1, 1, 1),
    );

    assert!(pool.is_closed());
    assert_eq!(pool.status().size, 0);
    assert!(matches!(
        open_observed_generation(&pool, SourceId::new(0).unwrap(), &[], &budget).await,
        Err(PgGenerationError::Control(
            QueryControlError::SourceWorkExceeded
        ))
    ));
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
    assert_eq!(pool.status().size, 0);
}

#[test]
fn verification_compares_the_exact_rich_projection_and_session() {
    let source_id = SourceId::new(0).unwrap();
    let mut table = TableSchema::new("items");
    table.columns = vec![
        sf_core::Column::new("id", "integer", true),
        sf_core::Column::new("code", "text", true),
    ];
    table.primary_key = vec!["id".to_owned()];
    table.unique = vec![vec!["id".to_owned()], vec!["code".to_owned()]];
    let mapping = sf_mapping::direct_mapping_for_source_with_row_identity(
        &[table.clone()],
        "http://example.test/",
        source_id,
        sf_mapping::DirectMappingRowIdentity::RequirePrimaryKey,
    )
    .unwrap();
    let expected = PostgresDirectGeneration {
        source_id,
        base_iri: Arc::from("http://example.test/"),
        row_identity: sf_mapping::DirectMappingRowIdentity::RequirePrimaryKey,
        mapping_digest: MappingDigest::from_mapping(&mapping),
        identity: observed_identity(),
        session: valid_context(),
        tables: Arc::from(vec![table.clone()]),
    };

    assert!(lease::generation_facts_mismatch(
        Some(&expected.identity),
        Some(&[table.clone()]),
        &expected.session,
        &expected,
    )
    .is_none());

    table.unique.reverse();
    assert!(matches!(
        lease::generation_facts_mismatch(
            Some(&expected.identity),
            Some(&[table]),
            &expected.session,
            &expected,
        ),
        Some(PgGenerationError::SchemaDrift)
    ));

    table = expected.tables[0].clone();
    table.columns[0].sql_type = "bigint".to_owned();
    assert!(matches!(
        lease::generation_facts_mismatch(
            Some(&expected.identity),
            Some(&[table]),
            &expected.session,
            &expected,
        ),
        Some(PgGenerationError::SchemaDrift)
    ));

    let mut changed_session = expected.session.clone();
    changed_session.current_role_oid += 1;
    assert!(matches!(
        lease::generation_facts_mismatch(
            Some(&expected.identity),
            Some(&expected.tables),
            &changed_session,
            &expected,
        ),
        Some(PgGenerationError::CapabilityDrift)
    ));
}

fn one_table() -> TableSchema {
    let mut table = TableSchema::new("items");
    table.columns = vec![sf_core::Column::new("id", "integer", true)];
    table.primary_key = vec!["id".to_owned()];
    table
}

fn direct_mapping(base_iri: &str, source_id: SourceId) -> SourceMapping {
    sf_mapping::direct_mapping_for_source_with_row_identity(
        &[one_table()],
        base_iri,
        source_id,
        sf_mapping::DirectMappingRowIdentity::RequirePrimaryKey,
    )
    .unwrap()
}

fn verified_source(expected: Arc<PostgresDirectGeneration>) -> crate::IntrospectedSource {
    let pg_config: tokio_postgres::Config = "host=127.0.0.1 port=1".parse().unwrap();
    let pool = deadpool_postgres::Pool::builder(deadpool_postgres::Manager::new(
        pg_config,
        tokio_postgres::NoTls,
    ))
    .max_size(1)
    .build()
    .unwrap();
    crate::IntrospectedSource::observed(crate::Backend::Pg(pool.into()), expected.tables.to_vec())
        .bind_postgres_direct(PostgresDirectSourceCandidate {
            tables: expected.tables.to_vec(),
            observation: SourceSchemaObservationV1::unavailable(),
            generation: SourceGeneration::direct_postgres(expected),
        })
        .unwrap()
}

fn direct_ontology(base_iri: &str) -> crate::SemanticOntology {
    crate::test_support::ontology(
        &[&format!("{base_iri}items")],
        &[&format!("{base_iri}items#id")],
    )
}

#[test]
fn verified_generation_rejects_a_different_mapping_or_origin() {
    const EXPECTED_BASE: &str = "http://example.test/expected/";
    const OTHER_BASE: &str = "http://example.test/other/";
    let source_id = SourceId::new(0).unwrap();
    let expected = Arc::new(PostgresDirectGeneration {
        source_id,
        base_iri: Arc::from(EXPECTED_BASE),
        row_identity: sf_mapping::DirectMappingRowIdentity::RequirePrimaryKey,
        mapping_digest: MappingDigest::from_mapping(&direct_mapping(EXPECTED_BASE, source_id)),
        identity: observed_identity(),
        session: valid_context(),
        tables: Arc::from(vec![one_table()]),
    });

    let exact_source = verified_source(Arc::clone(&expected));
    let exact_ontology = direct_ontology(EXPECTED_BASE);
    let exact = crate::semantic_admission::ValidatedMapping::validate(
        direct_mapping(EXPECTED_BASE, source_id),
        crate::semantic_admission::MappingOrigin::Direct,
        &exact_ontology,
        &exact_source,
    )
    .unwrap();
    crate::RuntimeSource::admitted(exact_source, exact).expect("exact Direct receipt must bind");

    let other_source = verified_source(Arc::clone(&expected));
    let other_ontology = direct_ontology(OTHER_BASE);
    let other = crate::semantic_admission::ValidatedMapping::validate(
        direct_mapping(OTHER_BASE, source_id),
        crate::semantic_admission::MappingOrigin::Direct,
        &other_ontology,
        &other_source,
    )
    .unwrap();
    assert_eq!(
        crate::RuntimeSource::admitted(other_source, other).unwrap_err(),
        crate::SemanticAdmissionError::ReceiptGenerationMismatch
    );

    let authored_source = verified_source(expected);
    let authored_ontology = direct_ontology(EXPECTED_BASE);
    let authored = crate::semantic_admission::ValidatedMapping::validate(
        direct_mapping(EXPECTED_BASE, source_id),
        crate::semantic_admission::MappingOrigin::Authored,
        &authored_ontology,
        &authored_source,
    )
    .unwrap();
    assert_eq!(
        crate::RuntimeSource::admitted(authored_source, authored).unwrap_err(),
        crate::SemanticAdmissionError::ReceiptGenerationMismatch
    );
}
