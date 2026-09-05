//! Request-side verified-generation admission before authoritative compilation.

use std::sync::Arc;

use axum::response::Response;

use crate::activation::{ReadinessCause, RuntimeSnapshotLease};
use crate::budget::RequestBudget;
use crate::config::ServeConfig;
use crate::pg_generation::{PgGenerationError, VerifiedGenerationLeases};
use crate::problem::{self, ProblemCode};

/// Discover dependencies without cache authority, then acquire and revalidate
/// every required backend-generation lease before authoritative compilation.
pub(crate) async fn acquire(
    cfg: Arc<ServeConfig>,
    snapshot: &RuntimeSnapshotLease,
    query: &str,
    budget: &RequestBudget,
) -> Result<VerifiedGenerationLeases, Response> {
    let source_ids = cfg.query_mode().source_ids().into_iter().flatten();
    let requirements = snapshot
        .generation_requirements(source_ids)
        .map_err(|error| response_for_error(&cfg, snapshot, error))?;
    if requirements.is_empty() {
        return Ok(VerifiedGenerationLeases::default());
    }

    crate::request_compile::preflight(
        cfg.clone(),
        snapshot.clone(),
        query.to_owned(),
        budget.clone(),
    )
    .await?;
    VerifiedGenerationLeases::acquire(requirements, budget)
        .await
        .map_err(|error| response_for_error(&cfg, snapshot, error))
}

pub(crate) fn response_for_error(
    cfg: &ServeConfig,
    snapshot: &RuntimeSnapshotLease,
    error: PgGenerationError,
) -> Response {
    match error {
        PgGenerationError::Control(error) => problem::response_for_control(error),
        PgGenerationError::SourceUnavailable => {
            problem::response_with_retry_after(ProblemCode::SourceUnavailable)
        }
        PgGenerationError::SchemaDrift => {
            let _ =
                cfg.mark_runtime_not_ready(snapshot.activation_id(), ReadinessCause::SchemaDrift);
            problem::response_with_retry_after(ProblemCode::SourceUnavailable)
        }
        PgGenerationError::CapabilityDrift => {
            let _ = cfg
                .mark_runtime_not_ready(snapshot.activation_id(), ReadinessCause::CapabilityDrift);
            problem::response_with_retry_after(ProblemCode::SourceUnavailable)
        }
        PgGenerationError::Mapping(_) | PgGenerationError::Internal => {
            problem::response(ProblemCode::Internal)
        }
    }
}
