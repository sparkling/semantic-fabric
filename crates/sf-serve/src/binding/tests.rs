use super::*;

const SECRET: &str = "sf_secret_binding_debug_must_not_expose";
const MAPPING: &str = r#"
    @prefix rr: <http://www.w3.org/ns/r2rml#> .
    <#items> a rr:TriplesMap ;
        rr:logicalTable [ rr:sqlQuery "SELECT sf_secret_binding_debug_must_not_expose FROM private_items" ] ;
        rr:subjectMap [ rr:template "http://example.test/item/{id}" ] .
"#;

fn binding(source_index: usize) -> RuntimeBinding {
    binding_at(source_index, Epoch::default())
}

fn binding_at(source_index: usize, epoch: Epoch) -> RuntimeBinding {
    let source_id = SourceId::new(source_index).unwrap();
    let source = IntrospectedSource::unchecked(
        Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
        vec![TableSchema::new(SECRET)],
    );
    binding_from_source(source, source_id, epoch)
}

fn binding_from_source(
    source: IntrospectedSource,
    source_id: SourceId,
    epoch: Epoch,
) -> RuntimeBinding {
    let mapping = sf_mapping::parse_r2rml_for_source(MAPPING, source_id).unwrap();
    let ontology = crate::test_support::empty_ontology();
    let mapping = crate::semantic_admission::ValidatedMapping::validate(
        mapping,
        crate::semantic_admission::MappingOrigin::Authored,
        &ontology,
        &source,
    )
    .unwrap();
    RuntimeBinding::new(source, mapping, ontology.tbox().clone(), epoch)
}

fn offline_postgres_pool() -> deadpool_postgres::Pool {
    let config: tokio_postgres::Config = "host=127.0.0.1 port=1".parse().unwrap();
    deadpool_postgres::Pool::builder(deadpool_postgres::Manager::new(
        config,
        tokio_postgres::NoTls,
    ))
    .max_size(1)
    .build()
    .unwrap()
}

#[test]
fn backend_profiles_are_derived_and_never_admission_claims() {
    for kind in [
        BackendKind::Sqlite,
        BackendKind::Postgres,
        BackendKind::MySql,
    ] {
        let profile = BackendProfile::from_kind(kind);
        assert_eq!(profile.kind(), kind);
        assert_eq!(profile.dialect(), kind.dialect());
        assert_eq!(
            profile.supports_recursive_paths(),
            kind.dialect().supports_recursive_paths()
        );
        assert_eq!(
            profile.like_is_case_sensitive(),
            kind.dialect().like_is_case_sensitive()
        );
    }
}

#[test]
fn a_plan_from_another_binding_is_rejected_before_execution() {
    let first = binding(0);
    let second = binding(0);
    let control = sf_core::query_control::UncontrolledQueryControl;
    let bound = first
        .compile("SELECT * WHERE { ?s ?p ?o }", &control)
        .unwrap();

    assert_eq!(first.scope(), second.scope(), "regression precondition");
    let Err(error) = second.prepare_execution(bound) else {
        panic!("content-equal bindings must not share plan authority");
    };
    assert_eq!(error, BindingMismatch);
    assert_eq!(
        error.to_string(),
        "compiled plan does not belong to this runtime binding"
    );
}

#[test]
fn a_plan_remains_valid_for_its_original_binding() {
    let binding = binding(0);
    let control = sf_core::query_control::UncontrolledQueryControl;
    let bound = binding
        .compile("SELECT * WHERE { ?s ?p ?o }", &control)
        .unwrap();

    binding.prepare_execution(bound).unwrap();
}

#[test]
fn binding_debug_output_is_structural_and_secret_free() {
    let binding = binding(7);
    let debug = format!("{binding:?}");

    assert!(debug.contains("digests"));
    assert!(debug.contains("triples_map_count"));
    assert!(debug.contains("Unverified"));
    assert!(debug.contains("schema_observation"));
    assert!(debug.contains("Unavailable"));
    assert!(!debug.contains(SECRET));
    assert!(!debug.contains("private_items"));
    assert!(!debug.contains("SELECT"));
}

#[test]
fn unchecked_observation_binds_exact_backend_and_source_without_authority() {
    let source_id = SourceId::new(2).unwrap();
    let binding = binding(source_id.index());
    let observation = binding.schema_observation();

    assert_eq!(observation.backend_kind(), BackendKind::Sqlite);
    assert_eq!(observation.source_id(), source_id);
    assert!(!observation.is_available());
    assert_eq!(observation.postgres_unavailable_reason(), None);
}

#[test]
fn postgres_unavailability_survives_binding_without_authorizing_compilation() {
    let source_id = SourceId::new(3).unwrap();
    let reason = sf_sql::introspect::PostgresSchemaIdentityUnavailableV1::UnqualifiedEnginePatch;
    let source = IntrospectedSource::postgres_unavailable(
        offline_postgres_pool().into(),
        vec![TableSchema::new(SECRET)],
        reason,
    );
    let binding = binding_from_source(source, source_id, Epoch::default());
    let observation = binding.schema_observation();

    assert_eq!(observation.backend_kind(), BackendKind::Postgres);
    assert_eq!(observation.source_id(), source_id);
    assert!(!observation.is_available());
    assert_eq!(observation.postgres_unavailable_reason(), Some(reason));
    assert_eq!(
        binding.compiler.constraint_authority(),
        sf_sparql::ConstraintAuthority::Unverified
    );
    assert_eq!(
        binding.scope().constraint_authority(),
        sf_sparql::ConstraintAuthority::Unverified
    );
}

#[test]
fn runtime_binding_quarantines_catalogue_constraint_authority() {
    let binding = binding(0);
    assert_eq!(
        binding.compiler.constraint_authority(),
        sf_sparql::ConstraintAuthority::Unverified
    );
    assert_eq!(
        binding.scope().constraint_authority(),
        sf_sparql::ConstraintAuthority::Unverified
    );
}

#[test]
fn runtime_binding_reuses_the_cached_plan_allocation() {
    let binding = binding(0);
    let control = sf_core::query_control::UncontrolledQueryControl;
    let first = binding
        .compile("SELECT * WHERE { ?s ?p ?o }", &control)
        .unwrap();
    let second = binding
        .compile("SELECT * WHERE { ?s ?p ?o }", &control)
        .unwrap();

    assert!(Arc::ptr_eq(&first.plan, &second.plan));
}
