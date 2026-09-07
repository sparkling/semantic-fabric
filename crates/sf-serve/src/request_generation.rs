//! Request-side verified-generation admission before authoritative compilation.

use std::sync::Arc;

use axum::response::Response;

use crate::activation::RuntimeSnapshotLease;
use crate::budget::RequestBudget;
use crate::config::ServeConfig;
use crate::deadline::CompilerReservation;
use crate::pg_generation::{PgGenerationError, VerifiedGenerationLeases};
use crate::problem::{self, ProblemCode};

pub(crate) struct RequestGenerationAdmission {
    generations: VerifiedGenerationLeases,
    compiler: Option<CompilerReservation>,
}

impl RequestGenerationAdmission {
    pub(crate) fn into_parts(self) -> (VerifiedGenerationLeases, Option<CompilerReservation>) {
        (self.generations, self.compiler)
    }
}

/// Discover dependencies without cache authority, then acquire and revalidate
/// every required backend-generation lease before authoritative compilation.
pub(crate) async fn acquire(
    cfg: Arc<ServeConfig>,
    snapshot: &RuntimeSnapshotLease,
    query: &str,
    budget: &RequestBudget,
) -> Result<RequestGenerationAdmission, Response> {
    cfg.query_admission
        .validate(budget)
        .map_err(problem::response)?;
    let source_ids = cfg.query_mode().source_ids().into_iter().flatten();
    let requirements = snapshot
        .generation_requirements(source_ids)
        .map_err(response_for_error)?;
    if requirements.is_empty() {
        return Ok(RequestGenerationAdmission {
            generations: VerifiedGenerationLeases::default(),
            compiler: None,
        });
    }

    let compiler = crate::request_compile::preflight(
        cfg.clone(),
        snapshot.clone(),
        query.to_owned(),
        budget.clone(),
    )
    .await?;
    let generations = VerifiedGenerationLeases::acquire(requirements, budget)
        .await
        .map_err(response_for_error)?;
    Ok(RequestGenerationAdmission {
        generations,
        compiler: Some(compiler),
    })
}

pub(crate) fn response_for_error(error: PgGenerationError) -> Response {
    match error {
        PgGenerationError::Control(error) => problem::response_for_control(error),
        PgGenerationError::SourceUnavailable => {
            problem::response_with_retry_after(ProblemCode::SourceUnavailable)
        }
        PgGenerationError::SchemaDrift | PgGenerationError::CapabilityDrift => {
            problem::response_with_retry_after(ProblemCode::SourceUnavailable)
        }
        PgGenerationError::Mapping(_) | PgGenerationError::Internal => {
            problem::response(ProblemCode::Internal)
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::{Backend, IntrospectedSource};
    use sf_core::{SourceId, SourceMapping};

    fn config() -> ServeConfig {
        let source = IntrospectedSource::unchecked(
            Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
            Vec::new(),
        );
        ServeConfig::new(
            source,
            SourceMapping::new(SourceId::new(0).unwrap(), Vec::new()),
            crate::test_support::empty_ontology(),
        )
        .unwrap()
    }

    #[test]
    fn request_scoped_drift_does_not_change_runtime_readiness() {
        let config = config();
        let ready = config.runtime_readiness().unwrap();

        for error in [
            PgGenerationError::SchemaDrift,
            PgGenerationError::CapabilityDrift,
        ] {
            let response = response_for_error(error);
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(config.runtime_readiness().unwrap(), ready);
        }
    }
}
