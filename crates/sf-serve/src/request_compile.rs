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

/// Perform semantic and resource-shape admission without reading or populating
/// a plan cache. Verified Direct Mapping uses this before any source I/O, then
/// discards the result and compiles authoritatively only under its lease.
pub(crate) async fn preflight(
    cfg: Arc<ServeConfig>,
    snapshot: RuntimeSnapshotLease,
    query: String,
    budget: RequestBudget,
) -> Result<(), Response> {
    let mode = cfg.query_mode();
    let max_order_rows = cfg.max_order_rows();
    let permits = cfg.compiler_permits();
    let compiled = deadline::run_compiler(budget, permits, move |worker_budget| match mode {
        QueryMode::Single(source_id) => {
            let plan = snapshot.preflight_compile(source_id, &query, &worker_budget)?;
            crate::admission::admit(&plan, max_order_rows)
                .map_err(|_| sf_sparql::Error::Unsupported("query shape is not admitted".into()))?;
            if matches!(plan.form, sf_sparql::PlanForm::Ask) {
                worker_budget.preflight_ask_result()?;
            }
            Ok(())
        }
        QueryMode::SourceAffineUnion(source_ids) => {
            let plan = snapshot.preflight_federated_union(source_ids, &query, &worker_budget)?;
            for fragment in plan.fragments() {
                crate::admission::admit(fragment.plan(), max_order_rows).map_err(|_| {
                    sf_sparql::Error::Unsupported("query shape is not admitted".into())
                })?;
            }
            Ok(())
        }
    })
    .await;
    map_compiler_result(compiled)
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
    map_compiler_result(compiled)
}

// Axum's response is the endpoint's owned error value. Boxing it here would
// add an allocation only to satisfy a size heuristic before the caller returns
// the same response unchanged.
#[allow(clippy::result_large_err)]
fn map_compiler_result<T>(
    compiled: Result<sf_sparql::Result<T>, CompilerRunError>,
) -> Result<T, Response> {
    match compiled {
        Err(CompilerRunError::Control(error)) => Err(problem::response_for_control(error)),
        Err(CompilerRunError::AdmissionClosed | CompilerRunError::Join(_)) => {
            Err(problem::response(ProblemCode::Internal))
        }
        Ok(Err(error)) => Err(problem::response_for_sparql(&error)),
        Ok(Ok(plan)) => Ok(plan),
    }
}
