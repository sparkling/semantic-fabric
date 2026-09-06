//! Closed PostgreSQL generation-admission error classification.

use sf_core::query_control::QueryControlError;

#[derive(Debug)]
pub(crate) enum PgGenerationError {
    Control(QueryControlError),
    SourceUnavailable,
    SchemaDrift,
    CapabilityDrift,
    Mapping(
        #[allow(
            dead_code,
            reason = "payload is retained for test diagnostics while live admission is withheld"
        )]
        sf_core::Error,
    ),
    Internal,
}

impl From<QueryControlError> for PgGenerationError {
    fn from(error: QueryControlError) -> Self {
        Self::Control(error)
    }
}
