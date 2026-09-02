use sf_core::schema_identity::{
    ColumnInputV1, ColumnKeyV1, ColumnRefV1, ConstraintInputV1, ConstraintStateV1, IdentifierV1,
    ObservedSchemaIdentityV1, ProfileIdV1, QualifiedNameV1, RelationInputV1, RelationKindV1,
    RelationRefV1, SchemaIdentityErrorV1, SchemaIdentityInvalidCodeV1, SchemaIdentityLimitV1,
    SchemaIdentityLocationV1, SchemaIdentityOperationV1, SchemaObservationInputV1,
    SchemaProfilesV1, SourceTypeV1, TokenV1, TypeFacetV1, TypeFacetValueV1, TypeFamilyV1,
};

use super::*;

fn empty_registered_identity() -> ObservedSchemaIdentityV1 {
    let profile = |value| ProfileIdV1::new(value).expect("registered profile IDs are valid");
    ObservedSchemaIdentityV1::build(SchemaObservationInputV1 {
        profiles: SchemaProfilesV1 {
            structural: profile(POSTGRES16_PUBLIC_STRUCTURAL_PROFILE_ID_V1),
            types: profile(POSTGRES16_PUBLIC_TYPE_PROFILE_ID_V1),
            constraints: profile(POSTGRES16_PUBLIC_CONSTRAINT_PROFILE_ID_V1),
        },
        relations: Vec::new(),
        constraints: Vec::new(),
    })
    .expect("the registered profile admits an empty public scope")
}

fn available_snapshot() -> Postgres16PublicObservedSnapshotV1 {
    Postgres16PublicObservedSnapshotV1 {
        legacy_tables: vec![TableSchema::new("private_table_name")],
        availability: PostgresSchemaIdentityAvailabilityV1::Available(
            Postgres16PublicObservedSchemaV1 {
                identity: empty_registered_identity(),
            },
        ),
    }
}

#[test]
fn available_snapshot_preserves_identity_and_legacy_order() {
    let snapshot = available_snapshot();
    assert_eq!(snapshot.legacy_tables()[0].name, "private_table_name");
    assert!(snapshot.availability().is_available());
    assert_eq!(
        snapshot.availability().identity(),
        Some(&empty_registered_identity())
    );
    assert!(!format!("{snapshot:?}").contains("private_table_name"));

    let (tables, availability) = snapshot.into_parts();
    assert_eq!(tables[0].name, "private_table_name");
    assert!(matches!(
        availability,
        PostgresSchemaIdentityAvailabilityV1::Available(_)
    ));
}

#[test]
fn lossy_projection_discards_only_availability() {
    let tables = available_snapshot().into_legacy_tables();
    assert_eq!(tables, vec![TableSchema::new("private_table_name")]);
}

#[test]
fn unavailable_snapshot_is_total_and_exclusive() {
    let reason = PostgresSchemaIdentityUnavailableV1::ProfileNotImplemented;
    let snapshot = Postgres16PublicObservedSnapshotV1 {
        legacy_tables: Vec::new(),
        availability: PostgresSchemaIdentityAvailabilityV1::Unavailable(reason),
    };
    assert!(!snapshot.availability().is_available());
    assert!(snapshot.availability().identity().is_none());
    assert_eq!(snapshot.availability().unavailable_reason(), Some(reason));
}

#[test]
fn registered_engine_selection_is_exact() {
    for version in [160_009, 160_015] {
        assert_eq!(
            select_registered_profile_v1(version),
            Ok(RegisteredPostgresObservationProfileV1::Postgres16PublicBaseV1)
        );
    }
    for version in [160_000, 160_008, 160_010, 160_014, 160_016, 169_999] {
        assert_eq!(
            select_registered_profile_v1(version),
            Err(PostgresSchemaIdentityUnavailableV1::UnqualifiedEnginePatch)
        );
    }
    for version in [-1, 0, 150_999, 170_000, i32::MAX] {
        assert_eq!(
            select_registered_profile_v1(version),
            Err(PostgresSchemaIdentityUnavailableV1::ProfileNotImplemented)
        );
    }
}

#[test]
fn qualified_patches_build_the_same_registered_identity() {
    let (relations, constraints) = sample_facts();
    let expected = ObservedSchemaIdentityV1::build(SchemaObservationInputV1 {
        profiles: SchemaProfilesV1 {
            structural: ProfileIdV1::new(
                "io.github.sparkling.semantic-fabric.pg16-pb.structural-v1",
            )
            .unwrap(),
            types: ProfileIdV1::new("io.github.sparkling.semantic-fabric.pg16-pb.type-v1").unwrap(),
            constraints: ProfileIdV1::new(
                "io.github.sparkling.semantic-fabric.pg16-pb.constraint-v1",
            )
            .unwrap(),
        },
        relations: relations.clone(),
        constraints: constraints.clone(),
    })
    .unwrap();

    for version in [160_009, 160_015] {
        let observed =
            build_registered_observation(version, relations.clone(), constraints.clone()).unwrap();
        assert_eq!(observed.identity(), &expected);
    }
}

#[test]
fn version_selection_precedes_identity_validation() {
    let invalid_relations = vec![RelationInputV1 {
        name: qualified_name(Some("public"), "empty"),
        kind: RelationKindV1::BaseTable,
        columns: Vec::new(),
    }];
    assert_eq!(
        build_registered_observation(160_008, invalid_relations.clone(), Vec::new()).unwrap_err(),
        PostgresSchemaIdentityUnavailableV1::UnqualifiedEnginePatch
    );
    assert_eq!(
        build_registered_observation(170_000, invalid_relations.clone(), Vec::new()).unwrap_err(),
        PostgresSchemaIdentityUnavailableV1::ProfileNotImplemented
    );
    assert_eq!(
        build_registered_observation(160_009, invalid_relations, Vec::new()).unwrap_err(),
        PostgresSchemaIdentityUnavailableV1::IdentityRejected
    );
}

#[test]
fn kernel_failures_map_to_the_closed_postgres_algebra() {
    use PostgresSchemaIdentityLimitCodeV1 as PgLimit;
    use PostgresSchemaIdentityUnavailableV1 as Unavailable;
    use SchemaIdentityLimitV1 as KernelLimit;

    let cases = [
        (
            KernelLimit::ProfileOrTokenBytes,
            Unavailable::IdentityRejected,
        ),
        (KernelLimit::IdentifierBytes, Unavailable::IdentityRejected),
        (
            KernelLimit::FacetTextValueBytes,
            Unavailable::IdentityRejected,
        ),
        (
            KernelLimit::Relations,
            Unavailable::LimitExceeded(PgLimit::RichRelations),
        ),
        (
            KernelLimit::ColumnsPerRelation,
            Unavailable::LimitExceeded(PgLimit::LiveColumns),
        ),
        (
            KernelLimit::ColumnsTotal,
            Unavailable::LimitExceeded(PgLimit::LiveColumns),
        ),
        (
            KernelLimit::RawConstraints,
            Unavailable::LimitExceeded(PgLimit::RawConstraints),
        ),
        (
            KernelLimit::CanonicalConstraints,
            Unavailable::LimitExceeded(PgLimit::RawConstraints),
        ),
        (
            KernelLimit::KeyMembers,
            Unavailable::LimitExceeded(PgLimit::KeyMembers),
        ),
        (
            KernelLimit::FacetsPerColumn,
            Unavailable::LimitExceeded(PgLimit::Facets),
        ),
        (
            KernelLimit::FacetsTotal,
            Unavailable::LimitExceeded(PgLimit::Facets),
        ),
        (
            KernelLimit::FacetListItems,
            Unavailable::LimitExceeded(PgLimit::Facets),
        ),
        (
            KernelLimit::FacetListItemsTotal,
            Unavailable::LimitExceeded(PgLimit::Facets),
        ),
        (
            KernelLimit::Utf8PayloadBytes,
            Unavailable::LimitExceeded(PgLimit::TextBytes),
        ),
        (
            KernelLimit::StructuralBodyBytes,
            Unavailable::LimitExceeded(PgLimit::CanonicalBody),
        ),
        (
            KernelLimit::TypeBodyBytes,
            Unavailable::LimitExceeded(PgLimit::CanonicalBody),
        ),
        (
            KernelLimit::ConstraintBodyBytes,
            Unavailable::LimitExceeded(PgLimit::CanonicalBody),
        ),
    ];
    for (limit, expected) in cases {
        assert_eq!(
            map_schema_identity_error_v1(SchemaIdentityErrorV1::LimitExceeded {
                limit,
                observed: 2,
                maximum: 1,
            }),
            expected
        );
    }
    assert_eq!(
        map_schema_identity_error_v1(SchemaIdentityErrorV1::Invalid {
            code: SchemaIdentityInvalidCodeV1::Identifier,
            location: SchemaIdentityLocationV1::root(),
        }),
        Unavailable::IdentityRejected
    );
    assert_eq!(
        map_schema_identity_error_v1(SchemaIdentityErrorV1::ArithmeticOverflow {
            operation: SchemaIdentityOperationV1::IndexConversion,
        }),
        Unavailable::IdentityRejected
    );
}

fn sample_facts() -> (Vec<RelationInputV1>, Vec<ConstraintInputV1>) {
    let relation_name = qualified_name(Some("public"), "items");
    let column_name = IdentifierV1::new("id").unwrap();
    let column_key = ColumnKeyV1 {
        ordinal: 1.try_into().unwrap(),
        name: column_name.clone(),
    };
    let relation_ref = RelationRefV1 {
        name: relation_name.clone(),
    };
    let relations = vec![RelationInputV1 {
        name: relation_name,
        kind: RelationKindV1::BaseTable,
        columns: vec![ColumnInputV1 {
            ordinal: 1.try_into().unwrap(),
            name: column_name,
            source_type: SourceTypeV1 {
                native_name: qualified_name(Some("pg_catalog"), "int8"),
                family: TypeFamilyV1::SignedInteger,
                facets: vec![TypeFacetV1 {
                    key: TokenV1::new("bit-width").unwrap(),
                    value: TypeFacetValueV1::U64(64),
                }],
            },
        }],
    }];
    let constraints = vec![ConstraintInputV1::NotNull {
        column: ColumnRefV1 {
            relation: relation_ref,
            column: column_key,
        },
        state: ConstraintStateV1 {
            validated: true,
            enforced: true,
        },
    }];
    (relations, constraints)
}

fn qualified_name(schema: Option<&str>, local: &str) -> QualifiedNameV1 {
    QualifiedNameV1 {
        catalog: None,
        schema: schema.map(|value| IdentifierV1::new(value).unwrap()),
        local: IdentifierV1::new(local).unwrap(),
    }
}
