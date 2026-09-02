use sf_core::schema_identity::{
    ObservedSchemaIdentityV1, ProfileIdV1, SchemaObservationInputV1, SchemaProfilesV1,
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
