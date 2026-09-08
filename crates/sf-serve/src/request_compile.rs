//! Blocking compiler handoff for single-source and bounded federation modes.

use std::sync::Arc;

use axum::response::Response;
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

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
    charge_input(&query, &budget).map_err(problem::response_for_control)?;
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
    charge_input(&query, &budget).map_err(problem::response_for_control)?;
    let mode = cfg.query_mode();
    let permits = cfg.compiler_permits();
    let policy = cfg.query_admission.policy();
    let portable_rows = budget.portable_rows().cloned();
    let work = move |worker_budget: RequestBudget| match mode {
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use sf_core::{query_control::QueryLimits, SourceId, SourceMapping};
    use tower::ServiceExt;

    const QUERY: &str = "SELECT ?s WHERE { ?s ?p ?o }";

    fn config(work: u64) -> (Arc<ServeConfig>, crate::SqlitePool) {
        let backend = crate::Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap());
        let crate::Backend::Sqlite(pool) = &backend else {
            unreachable!()
        };
        let pool = pool.clone();
        let source = crate::IntrospectedSource::unchecked(backend, vec![]);
        let mut cfg = ServeConfig::new(
            source,
            SourceMapping::new(SourceId::new(0).unwrap(), vec![]),
            crate::test_support::empty_ontology(),
        )
        .unwrap();
        cfg.query_limits = QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX);
        (Arc::new(cfg), pool)
    }

    #[tokio::test]
    async fn preflight_and_authoritative_compile_share_cumulative_input_charge() {
        for (work, succeeds) in [
            (2 * QUERY.len() as u64 - 1, false),
            (2 * QUERY.len() as u64, true),
        ] {
            let (cfg, _) = config(work);
            let budget = cfg.request_budget();
            let snapshot = cfg.runtime_lease().unwrap();
            let reservation =
                preflight(cfg.clone(), snapshot.clone(), QUERY.into(), budget.clone())
                    .await
                    .unwrap();
            assert_eq!(
                budget.consumed(QueryCharge::CompilerWork),
                QUERY.len() as u64
            );
            let result = compile(
                cfg.clone(),
                snapshot,
                QUERY.into(),
                budget.clone(),
                Some(reservation),
                false,
            )
            .await;
            if succeeds {
                assert!(result.is_ok());
                assert_eq!(budget.consumed(QueryCharge::CompilerWork), work);
            } else {
                assert_eq!(
                    result.err().unwrap().status(),
                    StatusCode::TOO_MANY_REQUESTS
                );
                assert_eq!(
                    budget.consumed(QueryCharge::CompilerWork),
                    QUERY.len() as u64
                );
                assert!(budget.checkpoint().is_err());
            }
            assert_eq!(cfg.compiler_permits().available_permits(), 4);
        }
    }

    #[tokio::test]
    async fn zero_allowance_rejects_preflight_before_compiler_queue() {
        let (cfg, _) = config(0);
        let held = cfg.compiler_permits().acquire_many_owned(4).await.unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            preflight(
                cfg.clone(),
                cfg.runtime_lease().unwrap(),
                QUERY.into(),
                cfg.request_budget(),
            ),
        )
        .await
        .expect("must not queue behind held compiler permits");
        assert_eq!(
            result.err().unwrap().status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        drop(held);
    }

    #[tokio::test]
    async fn public_zero_allowance_never_enters_source_admission() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (cfg, pool) = config(0);
        let held = pool.pick_owned().acquire().await.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        pool.set_admission_pending_observer(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        });
        let request = Request::post("/sparql")
            .header("content-type", "application/sparql-query")
            .body(Body::from(QUERY))
            .unwrap();
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            crate::router(cfg).oneshot(request),
        )
        .await
        .expect("must not wait for source")
        .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        drop(held);
    }
}
