//! Closed, non-authorizing source-schema observation carriage.

use std::fmt;

use sf_core::SourceId;
use sf_sql::introspect::{
    Postgres16PublicObservedSchemaV1, Postgres16PublicObservedSnapshotV1,
    PostgresSchemaIdentityAvailabilityV1, PostgresSchemaIdentityUnavailableV1,
};
use sf_sql::TableSchema;

use crate::BackendKind;

/// Schema identity state paired with an [`crate::IntrospectedSource`].
///
/// The PostgreSQL state can enter only by consuming the opaque committed
/// snapshot. Generic and compatibility constructors create `Unavailable`.
pub(crate) enum SourceSchemaObservationV1 {
    Unavailable(Option<PostgresSchemaIdentityUnavailableV1>),
    Postgres16Public(Postgres16PublicObservedSchemaV1),
}

impl SourceSchemaObservationV1 {
    pub(crate) const fn unavailable() -> Self {
        Self::Unavailable(None)
    }

    pub(crate) fn from_postgres_snapshot(
        snapshot: Postgres16PublicObservedSnapshotV1,
    ) -> (Vec<TableSchema>, Self) {
        let (tables, availability) = snapshot.into_parts();
        let observation = match availability {
            PostgresSchemaIdentityAvailabilityV1::Available(observation) => {
                Self::Postgres16Public(observation)
            }
            PostgresSchemaIdentityAvailabilityV1::Unavailable(reason) => {
                Self::Unavailable(Some(reason))
            }
        };
        (tables, observation)
    }

    #[cfg(test)]
    pub(crate) const fn postgres_unavailable(reason: PostgresSchemaIdentityUnavailableV1) -> Self {
        Self::Unavailable(Some(reason))
    }

    pub(crate) fn bind(
        self,
        backend_kind: BackendKind,
        source_id: SourceId,
    ) -> BoundSourceSchemaObservationV1 {
        debug_assert!(
            backend_kind == BackendKind::Postgres || matches!(&self, Self::Unavailable(None)),
            "PostgreSQL schema observation must bind only to PostgreSQL"
        );
        let state = match (backend_kind, self) {
            (BackendKind::Postgres, state) => state,
            (_, Self::Unavailable(None)) => Self::Unavailable(None),
            // A PostgreSQL-specific state paired with another backend is an
            // internal mismatch. Discard it instead of carrying false facts.
            (_, Self::Unavailable(Some(_)) | Self::Postgres16Public(_)) => Self::Unavailable(None),
        };
        BoundSourceSchemaObservationV1 {
            backend_kind,
            source_id,
            state,
        }
    }
}

pub(crate) struct PostgresStartupObservationDiagnostic<'a>(
    &'a PostgresSchemaIdentityAvailabilityV1,
);

pub(crate) const fn postgres_startup_observation_diagnostic(
    availability: &PostgresSchemaIdentityAvailabilityV1,
) -> PostgresStartupObservationDiagnostic<'_> {
    PostgresStartupObservationDiagnostic(availability)
}

impl fmt::Display for PostgresStartupObservationDiagnostic<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("semantic-fabric: ")?;
        match self.0 {
            PostgresSchemaIdentityAvailabilityV1::Available(_) => {
                formatter.write_str("PostgreSQL schema identity available")
            }
            PostgresSchemaIdentityAvailabilityV1::Unavailable(reason) => {
                fmt::Display::fmt(reason, formatter)
            }
        }
    }
}

/// Observation state bound to the exact runtime backend and mapping source.
/// It deliberately supplies no compiler, cache, readiness, or execution API.
pub(crate) struct BoundSourceSchemaObservationV1 {
    backend_kind: BackendKind,
    source_id: SourceId,
    state: SourceSchemaObservationV1,
}

impl BoundSourceSchemaObservationV1 {
    #[cfg(test)]
    pub(crate) const fn backend_kind(&self) -> BackendKind {
        self.backend_kind
    }

    #[cfg(test)]
    pub(crate) const fn source_id(&self) -> SourceId {
        self.source_id
    }

    #[cfg(test)]
    pub(crate) const fn is_available(&self) -> bool {
        matches!(&self.state, SourceSchemaObservationV1::Postgres16Public(_))
    }

    #[cfg(test)]
    pub(crate) const fn postgres_unavailable_reason(
        &self,
    ) -> Option<PostgresSchemaIdentityUnavailableV1> {
        match &self.state {
            SourceSchemaObservationV1::Unavailable(reason) => *reason,
            SourceSchemaObservationV1::Postgres16Public(_) => None,
        }
    }
}

impl fmt::Debug for BoundSourceSchemaObservationV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (status, reason, table_count) = match &self.state {
            SourceSchemaObservationV1::Unavailable(reason) => ("Unavailable", *reason, None),
            SourceSchemaObservationV1::Postgres16Public(observation) => (
                "Available",
                None,
                Some(observation.direct_mapping_tables().len()),
            ),
        };
        formatter
            .debug_struct("BoundSourceSchemaObservationV1")
            .field("backend_kind", &self.backend_kind)
            .field("source_id", &self.source_id)
            .field("status", &status)
            .field("reason", &reason)
            .field("table_count", &table_count)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_unavailable_state_binds_every_backend_and_source_exactly() {
        for (index, backend_kind) in [
            BackendKind::Sqlite,
            BackendKind::Postgres,
            BackendKind::MySql,
        ]
        .into_iter()
        .enumerate()
        {
            let source_id = SourceId::new(index).unwrap();
            let bound = SourceSchemaObservationV1::unavailable().bind(backend_kind, source_id);
            assert_eq!(bound.backend_kind(), backend_kind);
            assert_eq!(bound.source_id(), source_id);
            assert!(!bound.is_available());
            assert_eq!(bound.postgres_unavailable_reason(), None);
        }
    }

    #[test]
    fn postgres_unavailable_reason_survives_a_postgres_binding() {
        let reason = PostgresSchemaIdentityUnavailableV1::UnqualifiedEnginePatch;
        let source_id = SourceId::new(3).unwrap();
        let postgres = SourceSchemaObservationV1::postgres_unavailable(reason)
            .bind(BackendKind::Postgres, source_id);
        assert_eq!(postgres.postgres_unavailable_reason(), Some(reason));
    }

    #[test]
    #[should_panic(expected = "PostgreSQL schema observation must bind only to PostgreSQL")]
    fn postgres_observation_backend_mismatch_is_an_internal_error() {
        SourceSchemaObservationV1::postgres_unavailable(
            PostgresSchemaIdentityUnavailableV1::UnqualifiedEnginePatch,
        )
        .bind(BackendKind::Sqlite, SourceId::new(3).unwrap());
    }

    #[test]
    fn startup_diagnostic_retains_only_the_closed_unavailable_reason() {
        let availability = PostgresSchemaIdentityAvailabilityV1::Unavailable(
            PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
                sf_sql::introspect::PostgresSchemaIdentityGuardCodeV1::ClientEncoding,
            ),
        );
        let diagnostic = postgres_startup_observation_diagnostic(&availability).to_string();

        assert_eq!(
            diagnostic,
            "semantic-fabric: PostgreSQL schema identity unavailable: client encoding unsupported"
        );
        assert!(diagnostic.len() <= 256);
    }
}
