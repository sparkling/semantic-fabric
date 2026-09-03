use std::error::Error as _;

use sf_sql::introspect::{
    Postgres16PublicObservedSnapshotV1, PostgresSchemaIdentityAvailabilityV1,
    PostgresSchemaIdentityGuardCodeV1, PostgresSchemaIdentityLimitCodeV1,
    PostgresSchemaIdentityUnavailableV1, POSTGRES16_PUBLIC_CONSTRAINT_PROFILE_ID_V1,
    POSTGRES16_PUBLIC_STRUCTURAL_PROFILE_ID_V1, POSTGRES16_PUBLIC_TYPE_PROFILE_ID_V1,
};

#[test]
fn postgres16_public_profile_ids_are_exact_and_independently_domain_separated() {
    assert_eq!(
        POSTGRES16_PUBLIC_STRUCTURAL_PROFILE_ID_V1,
        "io.github.sparkling.semantic-fabric.pg16-pb.structural-v1"
    );
    assert_eq!(
        POSTGRES16_PUBLIC_TYPE_PROFILE_ID_V1,
        "io.github.sparkling.semantic-fabric.pg16-pb.type-v1"
    );
    assert_eq!(
        POSTGRES16_PUBLIC_CONSTRAINT_PROFILE_ID_V1,
        "io.github.sparkling.semantic-fabric.pg16-pb.constraint-v1"
    );
    assert_eq!(POSTGRES16_PUBLIC_STRUCTURAL_PROFILE_ID_V1.len(), 57);
    assert_eq!(POSTGRES16_PUBLIC_TYPE_PROFILE_ID_V1.len(), 51);
    assert_eq!(POSTGRES16_PUBLIC_CONSTRAINT_PROFILE_ID_V1.len(), 57);
    let ids = [
        POSTGRES16_PUBLIC_STRUCTURAL_PROFILE_ID_V1,
        POSTGRES16_PUBLIC_TYPE_PROFILE_ID_V1,
        POSTGRES16_PUBLIC_CONSTRAINT_PROFILE_ID_V1,
    ];
    for id in ids {
        assert_eq!(
            sf_core::schema_identity::ProfileIdV1::new(id)
                .expect("registered profile IDs are valid")
                .as_str(),
            id
        );
    }
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[0], ids[2]);
    assert_ne!(ids[1], ids[2]);
}

fn snapshot_contract_is_public(_: &Postgres16PublicObservedSnapshotV1) {}

fn guard_code_is_exhaustive(code: PostgresSchemaIdentityGuardCodeV1) {
    match code {
        PostgresSchemaIdentityGuardCodeV1::ServerEncoding
        | PostgresSchemaIdentityGuardCodeV1::IndexKeyLimit
        | PostgresSchemaIdentityGuardCodeV1::IntegerDatetimes
        | PostgresSchemaIdentityGuardCodeV1::ReplicationRole
        | PostgresSchemaIdentityGuardCodeV1::SearchPath
        | PostgresSchemaIdentityGuardCodeV1::PublicNamespace
        | PostgresSchemaIdentityGuardCodeV1::CurrentDatabase => {}
    }
}

fn limit_code_is_exhaustive(code: PostgresSchemaIdentityLimitCodeV1) {
    match code {
        PostgresSchemaIdentityLimitCodeV1::RichRelations
        | PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes
        | PostgresSchemaIdentityLimitCodeV1::LiveColumns
        | PostgresSchemaIdentityLimitCodeV1::RawConstraints
        | PostgresSchemaIdentityLimitCodeV1::KeyMembers
        | PostgresSchemaIdentityLimitCodeV1::Facets
        | PostgresSchemaIdentityLimitCodeV1::TextBytes
        | PostgresSchemaIdentityLimitCodeV1::CanonicalBody => {}
    }
}

fn unavailable_reason_is_exhaustive(reason: PostgresSchemaIdentityUnavailableV1) {
    match reason {
        PostgresSchemaIdentityUnavailableV1::ProfileNotImplemented
        | PostgresSchemaIdentityUnavailableV1::UnqualifiedEnginePatch
        | PostgresSchemaIdentityUnavailableV1::GuardUnsupported(_)
        | PostgresSchemaIdentityUnavailableV1::LegacyCoordinateMismatch
        | PostgresSchemaIdentityUnavailableV1::CatalogQuery
        | PostgresSchemaIdentityUnavailableV1::CatalogDecode
        | PostgresSchemaIdentityUnavailableV1::LimitExceeded(_)
        | PostgresSchemaIdentityUnavailableV1::UnsupportedRelation
        | PostgresSchemaIdentityUnavailableV1::UnsupportedType
        | PostgresSchemaIdentityUnavailableV1::UnsupportedCollation
        | PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint
        | PostgresSchemaIdentityUnavailableV1::IdentityRejected => {}
    }
}

#[test]
fn unavailable_algebra_is_closed_redacted_and_source_free() {
    let guard_codes = [
        PostgresSchemaIdentityGuardCodeV1::ServerEncoding,
        PostgresSchemaIdentityGuardCodeV1::IndexKeyLimit,
        PostgresSchemaIdentityGuardCodeV1::IntegerDatetimes,
        PostgresSchemaIdentityGuardCodeV1::ReplicationRole,
        PostgresSchemaIdentityGuardCodeV1::PublicNamespace,
        PostgresSchemaIdentityGuardCodeV1::CurrentDatabase,
    ];
    let limit_codes = [
        PostgresSchemaIdentityLimitCodeV1::RichRelations,
        PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes,
        PostgresSchemaIdentityLimitCodeV1::LiveColumns,
        PostgresSchemaIdentityLimitCodeV1::RawConstraints,
        PostgresSchemaIdentityLimitCodeV1::KeyMembers,
        PostgresSchemaIdentityLimitCodeV1::Facets,
        PostgresSchemaIdentityLimitCodeV1::TextBytes,
        PostgresSchemaIdentityLimitCodeV1::CanonicalBody,
    ];
    let mut reasons = vec![
        PostgresSchemaIdentityUnavailableV1::ProfileNotImplemented,
        PostgresSchemaIdentityUnavailableV1::UnqualifiedEnginePatch,
        PostgresSchemaIdentityUnavailableV1::LegacyCoordinateMismatch,
        PostgresSchemaIdentityUnavailableV1::CatalogQuery,
        PostgresSchemaIdentityUnavailableV1::CatalogDecode,
        PostgresSchemaIdentityUnavailableV1::UnsupportedRelation,
        PostgresSchemaIdentityUnavailableV1::UnsupportedType,
        PostgresSchemaIdentityUnavailableV1::UnsupportedCollation,
        PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint,
        PostgresSchemaIdentityUnavailableV1::IdentityRejected,
    ];
    reasons.extend(
        guard_codes
            .into_iter()
            .map(PostgresSchemaIdentityUnavailableV1::GuardUnsupported),
    );
    reasons.extend(
        limit_codes
            .into_iter()
            .map(PostgresSchemaIdentityUnavailableV1::LimitExceeded),
    );

    for reason in reasons {
        unavailable_reason_is_exhaustive(reason);
        if let PostgresSchemaIdentityUnavailableV1::GuardUnsupported(code) = reason {
            guard_code_is_exhaustive(code);
        }
        if let PostgresSchemaIdentityUnavailableV1::LimitExceeded(code) = reason {
            limit_code_is_exhaustive(code);
        }
        let display = reason.to_string();
        let debug = format!("{reason:?}");
        assert!(display.len() <= 256);
        assert!(debug.len() <= 256);
        assert!(display.is_ascii());
        assert!(debug.is_ascii());
        assert!(!display.contains('"'));
        assert_eq!(debug, display);
        assert!(reason.source().is_none());

        let availability = PostgresSchemaIdentityAvailabilityV1::Unavailable(reason);
        assert!(!availability.is_available());
        assert!(availability.identity().is_none());
        assert_eq!(availability.unavailable_reason(), Some(reason));
    }

    let _ = snapshot_contract_is_public;
}
