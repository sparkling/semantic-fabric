//! Origin isolation and pure profile rejection require no database connection.
use super::*;

fn source(expected: Arc<PostgresGeneration>, origin: MappingOrigin) -> crate::IntrospectedSource {
    let source = verified_source(Arc::clone(&expected));
    source
        .bind_postgres_generation(PostgresSourceCandidate {
            tables: expected.tables.to_vec(),
            observation: SourceSchemaObservationV1::unavailable(),
            generation: match origin {
                MappingOrigin::Authored => SourceGeneration::AuthoredPostgres(expected),
                MappingOrigin::Direct => SourceGeneration::DirectPostgres(expected),
            },
        })
        .unwrap()
}

#[test]
fn authored_authority_binds_origin_source_and_mapping_digest_independently() {
    const BASE: &str = "http://example.test/authored/";
    let id = SourceId::new(0).unwrap();
    for (envelope, expected_origin, mapped_origin, expected_id, mapped_id, base, accepted) in [
        (
            MappingOrigin::Authored,
            MappingOrigin::Authored,
            MappingOrigin::Authored,
            id,
            id,
            BASE,
            true,
        ),
        (
            MappingOrigin::Authored,
            MappingOrigin::Direct,
            MappingOrigin::Authored,
            id,
            id,
            BASE,
            false,
        ),
        (
            MappingOrigin::Direct,
            MappingOrigin::Authored,
            MappingOrigin::Direct,
            id,
            id,
            BASE,
            false,
        ),
        (
            MappingOrigin::Authored,
            MappingOrigin::Authored,
            MappingOrigin::Direct,
            id,
            id,
            BASE,
            false,
        ),
        (
            MappingOrigin::Authored,
            MappingOrigin::Authored,
            MappingOrigin::Authored,
            id,
            SourceId::new(1).unwrap(),
            BASE,
            false,
        ),
        (
            MappingOrigin::Authored,
            MappingOrigin::Authored,
            MappingOrigin::Authored,
            id,
            id,
            "http://example.test/changed/",
            false,
        ),
    ] {
        let expected = Arc::new(PostgresGeneration {
            source_id: expected_id,
            origin: expected_origin,
            mapping_digest: MappingDigest::from_mapping(&direct_mapping(BASE, mapped_id)),
            identity: observed_identity(),
            session: valid_context(),
            tables: vec![one_table()].into(),
        });
        let source = source(expected, envelope);
        let mapping = ValidatedMapping::validate(
            direct_mapping(base, mapped_id),
            mapped_origin,
            &direct_ontology(base),
            &source,
        )
        .unwrap();
        let result = source.ensure_generation_mapping(&mapping);
        if accepted {
            result.unwrap();
            crate::RuntimeSource::admitted(source, mapping).unwrap();
        } else {
            assert_eq!(
                result,
                Err(SemanticAdmissionError::ReceiptGenerationMismatch)
            );
        }
    }
}

#[test]
fn authored_profile_is_pure_bounded_and_deduplicates_tables() {
    use sf_core::ir::LogicalSource;
    let id = SourceId::new(0).unwrap();
    let (_, maps) = direct_mapping("http://example.test/", id).into_parts();
    assert_eq!(
        crate::pg_generation::authored::mapped_tables(&SourceMapping::new(
            id,
            [maps.clone(), maps.clone()].concat()
        ))
        .unwrap()
        .as_ref(),
        &["items".to_owned()]
    );
    for name in [
        "",
        "public.items",
        "items;SELECT",
        "1items",
        "items\0",
        "é",
        &"a".repeat(64),
    ] {
        let mut changed = maps.clone();
        changed[0].source = LogicalSource::Table(name.to_owned());
        assert!(
            crate::pg_generation::authored::mapped_tables(&SourceMapping::new(id, changed))
                .is_err()
        );
    }
    let mut changed = maps.clone();
    changed[0].source = LogicalSource::Query("SELECT id FROM items".into());
    assert!(
        crate::pg_generation::authored::mapped_tables(&SourceMapping::new(id, changed)).is_err()
    );
    assert!(
        crate::pg_generation::authored::mapped_tables(&SourceMapping::new(id, vec![])).is_err()
    );
    let mut too_many = Vec::new();
    for index in 0..257 {
        let mut map = maps[0].clone();
        map.source = LogicalSource::Table(format!("items_{index}"));
        too_many.push(map);
    }
    assert!(
        crate::pg_generation::authored::mapped_tables(&SourceMapping::new(id, too_many)).is_err()
    );
}

#[test]
fn authored_reload_preserves_generation_failure_classification() {
    use crate::ReadinessCause;
    use sf_core::query_control::QueryControlError;
    for (error, expected) in [
        (PgGenerationError::SchemaDrift, ReadinessCause::SchemaDrift),
        (
            PgGenerationError::CapabilityDrift,
            ReadinessCause::CapabilityDrift,
        ),
        (PgGenerationError::Internal, ReadinessCause::CapabilityDrift),
        (
            PgGenerationError::SourceUnavailable,
            ReadinessCause::SourceUnavailable,
        ),
        (
            PgGenerationError::Control(QueryControlError::Cancelled),
            ReadinessCause::SourceUnavailable,
        ),
        (
            PgGenerationError::Control(QueryControlError::DeadlineExceeded),
            ReadinessCause::SourceUnavailable,
        ),
        (
            PgGenerationError::Control(QueryControlError::SourceWorkExceeded),
            ReadinessCause::SourceUnavailable,
        ),
    ] {
        let error = crate::pg_generation::authored::generation_error(error);
        assert_eq!(crate::reload::rejection_cause(&error), expected);
        assert!(!format!("{error:?}").contains("PostgreSQL"));
    }
}

#[test]
fn authored_requirement_retains_binding_identity_and_unverified_is_not_promoted() {
    let id = SourceId::new(0).unwrap();
    let mut expected = direct_generation(id);
    Arc::get_mut(&mut expected).unwrap().origin = MappingOrigin::Authored;
    let source = source(Arc::clone(&expected), MappingOrigin::Authored);
    let binding = RuntimeBindingIdentity::fresh();
    let generation = SourceGeneration::AuthoredPostgres(expected);
    let requirement = generation
        .requirement(source.backend(), &binding)
        .unwrap()
        .unwrap();
    assert!(requirement.binding_identity.ptr_eq(&binding));
    assert_eq!(requirement.source_id(), id);
    let unverified = crate::IntrospectedSource::observed(
        source.backend().clone(),
        source.observed_schema().to_vec(),
    );
    assert!(unverified.verified_identity().is_none());
    assert!(SourceGeneration::Unverified
        .requirement(unverified.backend(), &binding)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn protected_reload_control_is_owned_until_last_worker_and_observes_shutdown() {
    use sf_core::query_control::{QueryControl, QueryControlError};
    let mut config = crate::ServeConfig::new(
        crate::IntrospectedSource::unchecked(
            crate::Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
            vec![],
        ),
        SourceMapping::new(SourceId::new(0).unwrap(), vec![]),
        crate::test_support::empty_ontology(),
    )
    .unwrap();
    assert!(crate::startup_authored::control_budget(Some(&config)).is_err());
    let gate = Arc::new(tokio::sync::Semaphore::new(1));
    config.control_work = Some(Arc::clone(&gate));
    let budget = crate::startup_authored::control_budget(Some(&config)).unwrap();
    assert_eq!(gate.available_permits(), 0);
    assert!(crate::startup_authored::control_budget(Some(&config)).is_err());
    let worker = budget.clone();
    drop(budget);
    assert_eq!(gate.available_permits(), 0);
    config.begin_shutdown();
    worker.checkpoint().unwrap();
    config.force_shutdown();
    assert_eq!(worker.checkpoint(), Err(QueryControlError::Cancelled));
    assert_eq!(gate.available_permits(), 0);
    drop(worker);
    assert_eq!(gate.available_permits(), 1);
}
