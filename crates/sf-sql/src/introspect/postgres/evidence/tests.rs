use std::mem::size_of;

use super::*;

#[test]
fn ordinary_observer_is_zero_sized() {
    assert_eq!(size_of::<NoopPostgresObservationObserverV1>(), 0);
}

#[cfg(feature = "postgres-observation-evidence")]
mod recording {
    use futures_util::stream;

    use super::*;
    use crate::introspect::postgres::legacy_query::collect_bounded_mapped_rows_observed;
    use crate::introspect::postgres::observation::{
        PostgresSchemaIdentityGuardCodeV1, PostgresSchemaIdentityLimitCodeV1,
    };

    fn stream_error(_: tokio_postgres::Error) -> &'static str {
        "query"
    }

    #[tokio::test]
    async fn exact_cap_records_poll_decode_retention_and_completion() {
        let mut evidence = PostgresObservationEvidenceV1::default();
        let stream_id = PostgresObservationStreamV1::RichRelations;
        evidence.stream_started(stream_id, 2);
        let rows = stream::iter([Ok::<_, tokio_postgres::Error>(1), Ok(2)]);
        let values = collect_bounded_mapped_rows_observed(
            rows,
            2,
            stream_error,
            || "limit",
            |value| Ok(value * 2),
            stream_id,
            &mut evidence,
        )
        .await
        .expect("exact cap is admitted");
        assert_eq!(values, [2, 4]);
        assert_eq!(
            evidence.stream(stream_id),
            PostgresObservationStreamEvidenceV1 {
                started: true,
                cap: 2,
                polled: 2,
                decoded: 2,
                retained_peak: 2,
                terminal: PostgresObservationStreamTerminalV1::Complete,
            }
        );
    }

    #[tokio::test]
    async fn overflow_sentinel_is_polled_but_neither_decoded_nor_retained() {
        let mut evidence = PostgresObservationEvidenceV1::default();
        let stream_id = PostgresObservationStreamV1::RichAttributes;
        evidence.stream_started(stream_id, 2);
        let rows = stream::iter([Ok::<_, tokio_postgres::Error>(1), Ok(2), Ok(3), Ok(4)]);
        let result = collect_bounded_mapped_rows_observed(
            rows,
            2,
            stream_error,
            || "limit",
            Ok,
            stream_id,
            &mut evidence,
        )
        .await;
        assert_eq!(result, Err("limit"));
        let stream = evidence.stream(stream_id);
        assert_eq!(stream.polled(), 3);
        assert_eq!(stream.decoded(), 2);
        assert_eq!(stream.retained_peak(), 2);
        assert!(stream.overflow());
        assert_eq!(
            stream.terminal(),
            PostgresObservationStreamTerminalV1::Overflow
        );
    }

    #[tokio::test]
    async fn row_failure_records_the_polled_but_undecoded_row() {
        let mut evidence = PostgresObservationEvidenceV1::default();
        let stream_id = PostgresObservationStreamV1::RichCatalogConstraints;
        evidence.stream_started(stream_id, 4);
        let rows = stream::iter([Ok::<_, tokio_postgres::Error>(1), Ok(2), Ok(3)]);
        let result = collect_bounded_mapped_rows_observed(
            rows,
            4,
            stream_error,
            || "limit",
            |value| if value == 2 { Err("row") } else { Ok(value) },
            stream_id,
            &mut evidence,
        )
        .await;
        assert_eq!(result, Err("row"));
        let stream = evidence.stream(stream_id);
        assert_eq!(
            (stream.polled(), stream.decoded(), stream.retained_peak()),
            (2, 1, 1)
        );
        assert_eq!(
            stream.terminal(),
            PostgresObservationStreamTerminalV1::RowFailure
        );
    }

    #[test]
    fn phases_lifecycle_and_counts_are_closed_and_first_failure_wins() {
        let mut evidence = PostgresObservationEvidenceV1::default();
        evidence.phase_started(PostgresObservationPhaseV1::RelationNormalization);
        evidence.phase_completed(PostgresObservationPhaseV1::RelationNormalization);
        evidence.phase_failed(
            PostgresObservationPhaseV1::ConstraintNormalization,
            PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint,
        );
        evidence.phase_failed(
            PostgresObservationPhaseV1::IdentityBuild,
            PostgresSchemaIdentityUnavailableV1::IdentityRejected,
        );
        evidence.guard_query_started();
        evidence.guard_query_started();
        PostgresObservationObserverV1::guard_passed(&mut evidence, 160_015);
        evidence.constraint_counts(3, 2, 5);
        evidence.savepoint_active();
        evidence.savepoint_recovered();
        evidence.commit_started();
        evidence.commit_completed();

        assert!(evidence.phase_was_started(PostgresObservationPhaseV1::RelationNormalization));
        assert!(evidence.phase_was_completed(PostgresObservationPhaseV1::RelationNormalization));
        assert_eq!(evidence.guard_queries_started(), 2);
        assert_eq!(evidence.server_version_num(), Some(160_015));
        assert_eq!(evidence.recorded_constraint_counts(), (3, 2, 5));
        assert_eq!(
            evidence.failure(),
            Some((
                PostgresObservationPhaseV1::ConstraintNormalization,
                PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint,
            ))
        );
        assert_eq!(
            evidence.savepoint(),
            PostgresObservationSavepointV1::Recovered
        );
        assert_eq!(evidence.commit(), PostgresObservationCommitV1::Complete);
    }

    #[test]
    fn every_closed_error_has_an_exact_protocol_code_including_client_encoding() {
        use PostgresSchemaIdentityGuardCodeV1 as Guard;
        use PostgresSchemaIdentityLimitCodeV1 as Limit;
        use PostgresSchemaIdentityUnavailableV1 as Error;

        let cases = [
            (Error::ProfileNotImplemented, "ProfileNotImplemented"),
            (Error::UnqualifiedEnginePatch, "UnqualifiedEnginePatch"),
            (
                Error::GuardUnsupported(Guard::ServerEncoding),
                "GuardUnsupported:ServerEncoding",
            ),
            (
                Error::GuardUnsupported(Guard::ClientEncoding),
                "GuardUnsupported:ClientEncoding",
            ),
            (
                Error::GuardUnsupported(Guard::IdentifierLength),
                "GuardUnsupported:IdentifierLength",
            ),
            (
                Error::GuardUnsupported(Guard::IndexKeyLimit),
                "GuardUnsupported:IndexKeyLimit",
            ),
            (
                Error::GuardUnsupported(Guard::IntegerDatetimes),
                "GuardUnsupported:IntegerDatetimes",
            ),
            (
                Error::GuardUnsupported(Guard::ReplicationRole),
                "GuardUnsupported:ReplicationRole",
            ),
            (
                Error::GuardUnsupported(Guard::SearchPath),
                "GuardUnsupported:SearchPath",
            ),
            (
                Error::GuardUnsupported(Guard::PublicNamespace),
                "GuardUnsupported:PublicNamespace",
            ),
            (
                Error::GuardUnsupported(Guard::CurrentDatabase),
                "GuardUnsupported:CurrentDatabase",
            ),
            (Error::LegacyCoordinateMismatch, "LegacyCoordinateMismatch"),
            (Error::CatalogQuery, "CatalogQuery"),
            (Error::CatalogDecode, "CatalogDecode"),
            (
                Error::LimitExceeded(Limit::RichRelations),
                "LimitExceeded:RichRelations",
            ),
            (
                Error::LimitExceeded(Limit::PhysicalAttributes),
                "LimitExceeded:PhysicalAttributes",
            ),
            (
                Error::LimitExceeded(Limit::LiveColumns),
                "LimitExceeded:LiveColumns",
            ),
            (
                Error::LimitExceeded(Limit::RawConstraints),
                "LimitExceeded:RawConstraints",
            ),
            (
                Error::LimitExceeded(Limit::KeyMembers),
                "LimitExceeded:KeyMembers",
            ),
            (Error::LimitExceeded(Limit::Facets), "LimitExceeded:Facets"),
            (
                Error::LimitExceeded(Limit::TextBytes),
                "LimitExceeded:TextBytes",
            ),
            (
                Error::LimitExceeded(Limit::CanonicalBody),
                "LimitExceeded:CanonicalBody",
            ),
            (Error::UnsupportedRelation, "UnsupportedRelation"),
            (Error::UnsupportedType, "UnsupportedType"),
            (Error::UnsupportedCollation, "UnsupportedCollation"),
            (Error::UnsupportedConstraint, "UnsupportedConstraint"),
            (Error::IdentityRejected, "IdentityRejected"),
        ];
        assert_eq!(cases.len(), 27);
        for (error, expected) in cases {
            assert_eq!(error.evidence_code(), expected);
        }
    }
}
