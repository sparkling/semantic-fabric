//! Blocking compiler handoff for single-source and bounded federation modes.

use std::sync::Arc;

use axum::response::Response;

use crate::activation::RuntimeSnapshotLease;
use crate::binding::{BoundFederatedPlan, BoundPlan};
use crate::budget::RequestBudget;
use crate::config::{QueryMode, ServeConfig};
use crate::deadline::{self, CompilerRunError};
use crate::problem::{self, ProblemCode};

pub(crate) enum BoundQuery {
    Single(Box<BoundPlan>),
    Federated(Box<BoundFederatedPlan>),
}

/// Parse/rewrite off the async runtime. Federation parses exactly once inside
/// `sf-sparql` and compiles both source-local arms under one request budget.
pub(crate) async fn compile(
    cfg: Arc<ServeConfig>,
    snapshot: RuntimeSnapshotLease,
    query: String,
    budget: RequestBudget,
) -> Result<BoundQuery, Response> {
    let mode = cfg.query_mode();
    let permits = cfg.compiler_permits();
    let compiled = deadline::run_compiler(budget, permits, move |worker_budget| match mode {
        QueryMode::Single(source_id) => snapshot
            .compile(source_id, &query, &worker_budget)
            .map(Box::new)
            .map(BoundQuery::Single),
        QueryMode::SourceAffineUnion(source_ids) => snapshot
            .compile_federated_union(source_ids, &query, &worker_budget)
            .map(Box::new)
            .map(BoundQuery::Federated),
    })
    .await;
    match compiled {
        Err(CompilerRunError::Control(error)) => Err(problem::response_for_control(error)),
        Err(CompilerRunError::AdmissionClosed | CompilerRunError::Join(_)) => {
            Err(problem::response(ProblemCode::Internal))
        }
        Ok(Err(error)) => Err(problem::response_for_sparql(&error)),
        Ok(Ok(plan)) => Ok(plan),
    }
}
