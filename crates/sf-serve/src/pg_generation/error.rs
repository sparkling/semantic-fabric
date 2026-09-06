use sf_core::query_control::QueryControlError;

#[derive(Debug)]
pub(crate) enum PgGenerationError {
    Control(QueryControlError),
    SourceUnavailable,
    SchemaDrift,
    CapabilityDrift,
    Mapping(sf_core::Error),
    Internal,
}

impl From<QueryControlError> for PgGenerationError {
    fn from(error: QueryControlError) -> Self {
        Self::Control(error)
    }
}
