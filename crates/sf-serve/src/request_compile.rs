//! Blocking compiler handoff for single-source and bounded federation modes.

use std::sync::Arc;

use axum::response::Response;

use crate::activation::RuntimeSnapshotLease;
use crate::binding::{BoundFederatedPlan, BoundPlan};
use crate::budget::RequestBudget;
use crate::config::{QueryMode, ServeConfig};
use crate::deadline::{self, CompilerReservation, CompilerRunError};
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
) -> Result<CompilerReservation, Response> {
    cfg.query_admission
        .validate(&budget)
        .map_err(problem::response)?;
    let mode = cfg.query_mode();
    let max_order_rows = cfg.max_order_rows();
    let permits = cfg.compiler_permits();
    let (compiled, reservation) =
        deadline::run_compiler_retaining(budget, permits, move |worker_budget| match mode {
            QueryMode::Single(source_id) => {
                let plan = snapshot.preflight_compile(source_id, &query, &worker_budget)?;
                crate::admission::admit(&plan, max_order_rows).map_err(|_| {
                    sf_sparql::Error::Unsupported("query shape is not admitted".into())
                })?;
                if matches!(plan.form, sf_sparql::PlanForm::Ask) {
                    worker_budget.preflight_ask_result()?;
                }
                Ok(())
            }
            QueryMode::SourceAffineUnion(source_ids) => {
                let plan =
                    snapshot.preflight_federated_union(source_ids, &query, &worker_budget)?;
                for fragment in plan.fragments() {
                    crate::admission::admit(fragment.plan(), max_order_rows).map_err(|_| {
                        sf_sparql::Error::Unsupported("query shape is not admitted".into())
                    })?;
                }
                Ok(())
            }
        })
        .await
        .map_err(map_compiler_run_error)?;
    map_compiler_result(Ok(compiled))?;
    Ok(reservation)
}

/// Parse/rewrite off the async runtime. Federation parses exactly once inside
/// `sf-sparql` and compiles both source-local arms under one request budget.
pub(crate) async fn compile(
    cfg: Arc<ServeConfig>,
    snapshot: RuntimeSnapshotLease,
    query: String,
    budget: RequestBudget,
    reservation: Option<CompilerReservation>,
    multi_origin: bool,
) -> Result<BoundQuery, Response> {
    cfg.query_admission
        .validate(&budget)
        .map_err(problem::response)?;
    let mode = cfg.query_mode();
    let permits = cfg.compiler_permits();
    let policy = cfg.query_admission.policy();
    let portable_rows = budget.portable_rows().cloned();
    let work = move |worker_budget: RequestBudget| match mode {
        QueryMode::Single(source_id) if multi_origin => snapshot
            .snapshot()
            .registry()
            .binding(source_id)
            .ok_or_else(|| sf_sparql::Error::Mapping("lineage source is missing".into()))?
            .compile_lineage(&query, &worker_budget, policy)
            .map(Box::new)
            .map(BoundQuery::Single),
        QueryMode::Single(source_id) if policy.is_some() => snapshot
            .compile_secured(source_id, &query, &worker_budget, policy.unwrap())
            .map(Box::new)
            .map(BoundQuery::Single),
        QueryMode::SourceAffineUnion(source_ids) if policy.is_some() => snapshot
            .compile_federated_secured(source_ids, &query, &worker_budget, policy.unwrap())
            .map(Box::new)
            .map(BoundQuery::Federated),
        QueryMode::Single(source_id) => snapshot
            .compile(source_id, &query, &worker_budget)
            .map(Box::new)
            .map(BoundQuery::Single),
        QueryMode::SourceAffineUnion(source_ids) => snapshot
            .compile_federated_union(source_ids, &query, &worker_budget)
            .map(Box::new)
            .map(BoundQuery::Federated),
    };
    let compiled = match reservation {
        Some(reservation) => deadline::run_reserved_compiler(budget, reservation, work).await,
        None => deadline::run_compiler(budget, permits, work).await,
    };
    let mut bound = map_compiler_result(compiled)?;
    if let Some(policy) = portable_rows.as_deref() {
        let authorized = match &mut bound {
            BoundQuery::Single(plan) => plan.authorize_portable_rows(policy),
            BoundQuery::Federated(plan) => plan.authorize_portable_rows(policy),
        };
        if authorized.is_err() {
            crate::access_telemetry::record(crate::access_telemetry::AccessDecision::Deny);
            return Err(problem::response(ProblemCode::AccessDenied));
        }
    }
    Ok(bound)
}

fn map_compiler_run_error(error: CompilerRunError) -> Response {
    match error {
        CompilerRunError::Control(error) => problem::response_for_control(error),
        CompilerRunError::AdmissionClosed | CompilerRunError::Join(_) => {
            problem::response(ProblemCode::Internal)
        }
    }
}

// Axum's response is the endpoint's owned error value. Boxing it here would
// add an allocation only to satisfy a size heuristic before the caller returns
// the same response unchanged.
#[allow(clippy::result_large_err)]
fn map_compiler_result<T>(
    compiled: Result<sf_sparql::Result<T>, CompilerRunError>,
) -> Result<T, Response> {
    match compiled {
        Err(error) => Err(map_compiler_run_error(error)),
        Ok(Err(error)) => Err(problem::response_for_sparql(&error)),
        Ok(Ok(plan)) => Ok(plan),
    }
}
