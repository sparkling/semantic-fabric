//! Blocking compiler handoff for single-source and bounded federation modes.

use std::sync::Arc;

use axum::response::Response;
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

use crate::activation::RuntimeSnapshotLease;
use crate::binding::{BoundFederatedPlan, BoundPlan};
use crate::budget::RequestBudget;
use crate::config::{QueryMode, ServeConfig};
use crate::deadline::{self, CompilerReservation, CompilerRunError};
use crate::generated_request::GeneratedResponseIdentity;
use crate::problem::{self, ProblemCode};
use crate::telemetry::{in_stage_sync as traced_sync, Stage};

pub(crate) enum BoundQuery {
    Single(Box<BoundPlan>),
    Federated(Box<BoundFederatedPlan>),
}

/// Charge decoded UTF-8 input before each public compiler pass, including cache
/// hits and structural preflight. This is an input-admission floor, not total
/// parser/optimizer CPU accounting; existing profile-specific charges remain.
pub(crate) fn charge_input(
    query: &str,
    control: &dyn QueryControl,
) -> Result<(), QueryControlError> {
    let bytes = u64::try_from(query.len())
        .map_err(|_| control.terminate(QueryControlError::AccountingOverflow))?;
    control.consume(QueryCharge::CompilerWork, bytes)
}

/// Perform semantic and resource-shape admission without reading or populating
/// a plan cache. Protected generations use this before any source I/O, then
/// discards the result and compiles authoritatively only under its lease.
/// Under the opt-in generated-query profile this delegates to the generated
/// runtime-binding preflight, which never falls back to the raw compiler.
pub(crate) async fn preflight(
    cfg: Arc<ServeConfig>,
    snapshot: RuntimeSnapshotLease,
    query: String,
    budget: RequestBudget,
) -> Result<CompilerReservation, Response> {
    if cfg.generated_active() {
        return crate::generated_request::preflight(cfg, snapshot, query, budget).await;
    }
    cfg.query_admission
        .validate(&budget)
        .map_err(problem::response)?;
    charge_input(&query, &budget).map_err(problem::response_for_control)?;
    let mode = cfg.query_mode();
    let max_order_rows = cfg.max_order_rows();
    let permits = cfg.compiler_permits();
    let portable_rows = budget.portable_rows().cloned();
    let (compiled, reservation) =
        deadline::run_compiler_retaining(budget, permits, move |worker_budget| {
            cfg.with_parser(&worker_budget, || match mode {
                QueryMode::Single(source_id) => {
                    let mut plan = snapshot.preflight_compile(source_id, &query, &worker_budget)?;
                    if let Some(policy) = portable_rows.as_deref() {
                        if policy
                            .authorize(source_id, Arc::make_mut(&mut plan))
                            .is_err()
                        {
                            return Ok(false);
                        }
                    }
                    traced_sync(Stage::ShapeAdmission, || {
                        crate::admission::admit(&plan, max_order_rows, &worker_budget)
                    })?;
                    if matches!(plan.form, sf_sparql::PlanForm::Ask) {
                        worker_budget.preflight_ask_result()?;
                    }
                    Ok(true)
                }
                QueryMode::SourceAffineUnion(source_ids) => {
                    let mut plan =
                        snapshot.preflight_federated_union(source_ids, &query, &worker_budget)?;
                    if let Some(policy) = portable_rows.as_deref() {
                        if plan
                            .try_for_each_plan_mut(|source, plan| policy.authorize(source, plan))
                            .is_err()
                        {
                            return Ok(false);
                        }
                    }
                    for fragment in plan.fragments() {
                        traced_sync(Stage::ShapeAdmission, || {
                            crate::admission::admit(fragment.plan(), max_order_rows, &worker_budget)
                        })?;
                    }
                    Ok(true)
                }
            })
        })
        .await
        .map_err(map_compiler_run_error)?;
    if !map_compiler_result(Ok(compiled))? {
        crate::access_telemetry::record(crate::access_telemetry::AccessDecision::Deny);
        return Err(problem::response(ProblemCode::AccessDenied));
    }
    Ok(reservation)
}

/// Authoritative compile under the opt-in generated-query profile: the exact
/// compiled binding's identity is issued together with its bound plan, on the
/// same pinned snapshot and compiler reservation as the ordinary path.
pub(crate) async fn compile_generated(
    cfg: Arc<ServeConfig>,
    snapshot: RuntimeSnapshotLease,
    query: String,
    budget: RequestBudget,
    reservation: Option<CompilerReservation>,
) -> Result<(BoundQuery, GeneratedResponseIdentity), Response> {
    let compiled = crate::generated_request::compile(cfg, snapshot, query, budget, reservation);
    let (plan, identity) = compiled.await?;
    Ok((BoundQuery::Single(Box::new(plan)), identity))
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
    // The generated profile must use `compile_generated`; never fall back to raw.
    if cfg.generated_active() {
        return Err(problem::response(ProblemCode::Internal));
    }
    cfg.query_admission
        .validate(&budget)
        .map_err(problem::response)?;
    charge_input(&query, &budget).map_err(problem::response_for_control)?;
    let mode = cfg.query_mode();
    let permits = cfg.compiler_permits();
    let policy = cfg.query_admission.policy();
    let portable_rows = budget.portable_rows().cloned();
    let work = move |worker_budget: RequestBudget| {
        let mut bound = cfg.with_parser(&worker_budget, || match mode {
            QueryMode::SourceAffineUnion(source_ids) if multi_origin => snapshot
                .snapshot()
                .compile_federated_lineage(source_ids, &query, &worker_budget, policy)
                .map(Box::new)
                .map(BoundQuery::Federated),
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
        })?;
        // Preserve authorization-before-shape-admission: classify the actual
        // policy-adjusted plan and never reveal shape errors before denial.
        if let Some(policy) = portable_rows.as_deref() {
            let authorized = match &mut bound {
                BoundQuery::Single(plan) => plan.authorize_portable_rows(policy),
                BoundQuery::Federated(plan) => plan.authorize_portable_rows(policy),
            };
            if authorized.is_err() {
                return Ok(None);
            }
        }
        // Shape scanning is compiler-owned CPU work, including cache hits.
        // Keep the permit and shared terminal state until every fragment passes.
        match &bound {
            BoundQuery::Single(plan) => traced_sync(Stage::ShapeAdmission, || {
                crate::admission::admit(plan.plan(), cfg.max_order_rows(), &worker_budget)
            })?,
            BoundQuery::Federated(plan) => {
                for fragment in plan.plan().fragments() {
                    traced_sync(Stage::ShapeAdmission, || {
                        crate::admission::admit(
                            fragment.plan(),
                            cfg.max_order_rows(),
                            &worker_budget,
                        )
                    })?;
                }
            }
        }
        Ok(Some(bound))
    };
    let compiled = match reservation {
        Some(reservation) => deadline::run_reserved_compiler(budget, reservation, work).await,
        None => deadline::run_compiler(budget, permits, work).await,
    };
    match map_compiler_result(compiled)? {
        Some(bound) => Ok(bound),
        None => {
            crate::access_telemetry::record(crate::access_telemetry::AccessDecision::Deny);
            Err(problem::response(ProblemCode::AccessDenied))
        }
    }
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

#[cfg(test)]
#[path = "request_compile_tests.rs"]
mod tests;
