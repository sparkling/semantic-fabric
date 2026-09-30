//! Opt-in generated-query HTTP profile: guard, preflight, authoritative compile
//! and identity attachment (ADR-0056, SOURCE work only).
//!
//! Composes the accepted `RuntimeBinding` generated methods (sealed mapping
//! coverage, runtime binding, issued identity) with the existing subject
//! admission. `QueryAdmission::validate` always runs first. Coverage is checked
//! on every request, including cache hits. Nothing here selects the profile from
//! a request, provisions an endpoint, or implements nonempty dataset allowlists:
//! every dataset clause is refused by the accepted structural screen.
//!
//! The identity is an attestation of the admitted mapping/ontology/profile. It
//! is not data freshness, stream completeness, a bearer credential or a
//! caller-selected authority, and no request header is ever read for it.

#![allow(clippy::result_large_err)]

use std::sync::Arc;

use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use sf_core::query_control::QueryControl;
use sf_core::SourceId;
use sf_sparql::cache::generated::GeneratedQueryRefusal;
use sf_sparql::PlanForm;
use tokio_stream::StreamExt;

use crate::activation::RuntimeSnapshotLease;
use crate::binding::{BoundPlan, GeneratedDeferredOutcome, GeneratedRuntimeError, RuntimeBinding};
use crate::budget::RequestBudget;
use crate::config::{QueryMode, ServeConfig};
use crate::deadline::{self, CompilerReservation, CompilerRunError};
use crate::generated_profile_identity::GeneratedProfileIdentity;
use crate::problem::{self, ProblemCode};
use crate::telemetry::{in_stage_sync as traced_sync, Stage};

/// Documented ASCII response header carrying the issued profile identity on a
/// successful generated SELECT/ASK response only.
pub(crate) const PROFILE_HEADER: &str = "x-semantic-fabric-profile";

const LINEAGE_MEDIA_PREFIX: &str = "application/vnd.semantic-fabric.lineage";

enum Verdict {
    Denied,
    Internal,
    Failed(GeneratedRuntimeError),
}

type Admitted<T> = sf_sparql::Result<Result<T, Verdict>>;
type Issued = (BoundPlan, GeneratedProfileIdentity);

fn refuse(rule: &'static str) -> Response {
    problem::response_with_rule(ProblemCode::UnsupportedQuery, rule)
}

fn deny() -> Response {
    crate::access_telemetry::record(crate::access_telemetry::AccessDecision::Deny);
    problem::response(ProblemCode::AccessDenied)
}

fn run_error(error: CompilerRunError) -> Response {
    match error {
        CompilerRunError::Control(error) => problem::response_for_control(error),
        CompilerRunError::AdmissionClosed | CompilerRunError::Join(_) => {
            problem::response(ProblemCode::Internal)
        }
    }
}

fn failure_response(error: GeneratedRuntimeError) -> Response {
    match error {
        GeneratedRuntimeError::Refused(GeneratedQueryRefusal::Rule(rule)) => refuse(rule.code()),
        GeneratedRuntimeError::Refused(GeneratedQueryRefusal::CoverageRefused) => {
            refuse("coverage-refused")
        }
        GeneratedRuntimeError::Compiler(error) => problem::response_for_sparql(&error),
        GeneratedRuntimeError::PolicyMismatch | GeneratedRuntimeError::MissingSecurityContext => {
            deny()
        }
        GeneratedRuntimeError::ReceiptMismatch(_) => problem::response(ProblemCode::Internal),
    }
}

fn verdict_response(verdict: Verdict) -> Response {
    match verdict {
        Verdict::Denied => deny(),
        Verdict::Internal => problem::response(ProblemCode::Internal),
        Verdict::Failed(error) => failure_response(error),
    }
}

fn single_source(cfg: &ServeConfig) -> Result<SourceId, Response> {
    match cfg.query_mode() {
        QueryMode::Single(source_id) => Ok(source_id),
        QueryMode::SourceAffineUnion(_) => Err(refuse("federation-unsupported")),
    }
}

/// Any Accept value naming the lineage media type is an explicit provenance
/// request; it is refused rather than silently stripped.
fn requests_lineage(accept: Option<&str>) -> bool {
    match accept {
        Some(value) => value.to_ascii_lowercase().contains(LINEAGE_MEDIA_PREFIX),
        None => false,
    }
}

fn binding_of(
    snapshot: &RuntimeSnapshotLease,
    source_id: SourceId,
) -> sf_sparql::Result<&RuntimeBinding> {
    snapshot
        .snapshot()
        .registry()
        .binding(source_id)
        .ok_or_else(|| sf_sparql::Error::Mapping("runtime source is not registered".into()))
}

/// Use the already parsed authorization-only plan; never reparse a refusal.
fn admitted<T>(
    source_id: SourceId,
    portable: Option<&crate::PortableRowPolicy>,
    outcome: Result<GeneratedDeferredOutcome<T>, GeneratedRuntimeError>,
) -> Result<T, Verdict> {
    match outcome.map_err(Verdict::Failed)? {
        GeneratedDeferredOutcome::Admitted(value) => Ok(value),
        GeneratedDeferredOutcome::Refused {
            refusal,
            authorization_plan,
        } => {
            if let (Some(policy), Some(mut plan)) = (portable, authorization_plan) {
                if policy
                    .authorize(source_id, Arc::make_mut(&mut plan))
                    .is_err()
                {
                    return Err(Verdict::Denied);
                }
            }
            Err(Verdict::Failed(GeneratedRuntimeError::Refused(refusal)))
        }
    }
}

/// Subject admission first, then source-RLS compatibility on the pinned
/// snapshot, then the explicit federation/lineage refusals. Policy denial
/// therefore never reveals structural refusal detail.
///
/// A portable row policy authorizes query tables, not only the subject. A
/// lineage request under one therefore authorizes this exact query through the
/// uncached generated preflight before the lineage refusal. The request ends
/// here, so that is its only parse; no source I/O occurs and the compiler
/// reservation is released. Denial and control errors are returned unchanged.
pub(crate) async fn guard(
    cfg: &Arc<ServeConfig>,
    snapshot: &RuntimeSnapshotLease,
    query: &str,
    accept: Option<&str>,
    budget: &RequestBudget,
) -> Result<(), Response> {
    cfg.query_admission
        .validate(budget)
        .map_err(problem::response)?;
    // Same check as request_generation::acquire; no source I/O is performed.
    let rls_unsupported = cfg
        .query_mode()
        .source_ids()
        .into_iter()
        .flatten()
        .any(|source_id| !snapshot.permits_rls(source_id));
    if budget.postgres_rls().is_some() && rls_unsupported {
        return Err(crate::pg_rls::denied());
    }
    single_source(cfg)?;
    if !requests_lineage(accept) {
        return Ok(());
    }
    if budget.portable_rows().is_some() {
        let query = query.to_owned();
        let authorized = preflight(cfg.clone(), snapshot.clone(), query, budget.clone());
        drop(authorized.await?);
    }
    Err(refuse("lineage-unsupported"))
}

/// Uncached generated admission before any generation lease or source I/O. The
/// plan is discarded and no identity is issued; the returned compiler slot is
/// reused by the authoritative compile.
pub(crate) async fn preflight(
    cfg: Arc<ServeConfig>,
    snapshot: RuntimeSnapshotLease,
    query: String,
    budget: RequestBudget,
) -> Result<CompilerReservation, Response> {
    cfg.query_admission
        .validate(&budget)
        .map_err(problem::response)?;
    crate::request_compile::charge_input(&query, &budget).map_err(problem::response_for_control)?;
    let source_id = single_source(&cfg)?;
    let max_order_rows = cfg.max_order_rows();
    let permits = cfg.compiler_permits();
    let portable = budget.portable_rows().cloned();
    let work = move |worker: RequestBudget| -> Admitted<()> {
        cfg.with_parser(&worker, || {
            let rows = portable.as_deref();
            let binding = binding_of(&snapshot, source_id)?;
            let mut plan = match admitted(
                source_id,
                rows,
                binding.preflight_generated_deferred(&query, &worker),
            ) {
                Ok(plan) => plan,
                Err(verdict) => return Ok(Err(verdict)),
            };
            if let Some(rows) = rows {
                let authorized = rows.authorize(source_id, Arc::make_mut(&mut plan));
                if authorized.is_err() {
                    return Ok(Err(Verdict::Denied));
                }
            }
            traced_sync(Stage::ShapeAdmission, || {
                crate::admission::admit(&plan, max_order_rows, &worker)
            })?;
            if matches!(plan.form, PlanForm::Ask) {
                worker.preflight_ask_result()?;
            }
            Ok(Ok(()))
        })
    };
    let run = deadline::run_compiler_retaining(budget, permits, work);
    let (admitted, reservation) = run.await.map_err(run_error)?;
    match admitted {
        Err(error) => Err(problem::response_for_sparql(&error)),
        Ok(Err(verdict)) => Err(verdict_response(verdict)),
        Ok(Ok(())) => Ok(reservation),
    }
}

/// Authoritative generated compile on the pinned snapshot's exact binding. It
/// reruns admission (including coverage) under the same compiler reservation and
/// returns the binding's issued identity with its inseparable bound plan.
pub(crate) async fn compile(
    cfg: Arc<ServeConfig>,
    snapshot: RuntimeSnapshotLease,
    query: String,
    budget: RequestBudget,
    reservation: Option<CompilerReservation>,
) -> Result<Issued, Response> {
    cfg.query_admission
        .validate(&budget)
        .map_err(problem::response)?;
    crate::request_compile::charge_input(&query, &budget).map_err(problem::response_for_control)?;
    let source_id = single_source(&cfg)?;
    let max_order_rows = cfg.max_order_rows();
    let permits = cfg.compiler_permits();
    let policy = cfg.query_admission.policy();
    let portable = budget.portable_rows().cloned();
    let work = move |worker: RequestBudget| -> Admitted<Issued> {
        cfg.with_parser(&worker, || {
            let rows = portable.as_deref();
            let binding = binding_of(&snapshot, source_id)?;
            let attempt = match policy {
                Some(policy) => binding.compile_generated_secured_deferred(&query, &worker, policy),
                None => binding.compile_generated_deferred(&query, &worker),
            };
            let mut compiled = match admitted(source_id, rows, attempt) {
                Ok(compiled) => compiled,
                Err(verdict) => return Ok(Err(verdict)),
            };
            if let Some(rows) = rows {
                if compiled.plan.authorize_portable_rows(rows).is_err() {
                    return Ok(Err(Verdict::Denied));
                }
            }
            if matches!(compiled.plan.plan().form, PlanForm::Construct { .. }) {
                return Ok(Err(Verdict::Internal));
            }
            traced_sync(Stage::ShapeAdmission, || {
                crate::admission::admit(compiled.plan.plan(), max_order_rows, &worker)
            })?;
            Ok(Ok((compiled.plan, compiled.identity)))
        })
    };
    let compiled = match reservation {
        Some(reservation) => deadline::run_reserved_compiler(budget, reservation, work).await,
        None => deadline::run_compiler(budget, permits, work).await,
    };
    match compiled.map_err(run_error)? {
        Err(error) => Err(problem::response_for_sparql(&error)),
        Ok(Err(verdict)) => Err(verdict_response(verdict)),
        Ok(Ok(issued)) => Ok(issued),
    }
}

/// Attach the issued identity only to an actual successful (200, non-problem)
/// response. No request header is consulted.
pub(crate) async fn attach(
    response: Response,
    identity: &GeneratedProfileIdentity,
    budget: &RequestBudget,
) -> Response {
    let mut response = response;
    if response.status() == StatusCode::OK && !problem::is_pending(&response) {
        let (parts, body) = response.into_parts();
        let mut stream = body.into_data_stream();
        // SELECT emits no chunk until a row is produced or execution finishes.
        // Retain one bounded chunk, not the result set, before issuing identity.
        let first = match budget.run(stream.next()).await {
            Err(error) => return problem::response_for_control(error),
            Ok(Some(Ok(bytes))) => bytes,
            Ok(Some(Err(_))) | Ok(None) => {
                return match budget.checkpoint() {
                    Err(error) => problem::response_for_control(error),
                    Ok(()) => problem::response(ProblemCode::Internal),
                };
            }
        };
        let body = axum::body::Body::from_stream(
            tokio_stream::iter([Ok::<_, axum::Error>(first)]).chain(stream),
        );
        response = Response::from_parts(parts, body);
        if let Ok(value) = HeaderValue::from_str(&identity.wire()) {
            response.headers_mut().insert(PROFILE_HEADER, value);
        }
    }
    response
}

#[cfg(test)]
mod deferred_tests {
    use super::*;
    use crate::generated_http_test_support::{config, open, CONSTRUCT};
    use crate::{PortableRowPolicy, PortableRowRule, QueryShapeProfile};
    use sf_core::query_control::{QueryControlError, UncontrolledQueryControl};
    use sf_sparql::cache::generated::ShapeRule;

    #[test]
    fn generated_deferred_mapping_error_is_not_discarded() {
        let source = SourceId::new(1).unwrap();
        let result = admitted::<()>(
            source,
            None,
            Err(GeneratedRuntimeError::Compiler(sf_sparql::Error::Mapping(
                "diagnostic".into(),
            ))),
        );
        assert!(
            matches!(result, Err(Verdict::Failed(GeneratedRuntimeError::Compiler(
            sf_sparql::Error::Mapping(message),
        ))) if message == "diagnostic")
        );
    }

    #[test]
    fn generated_deferred_refusal_without_plan_stays_refused() {
        let source = SourceId::new(1).unwrap();
        let outcome = GeneratedDeferredOutcome::<()>::Refused {
            refusal: GeneratedQueryRefusal::CoverageRefused,
            authorization_plan: None,
        };
        assert!(matches!(
            admitted(source, None, Ok(outcome)),
            Err(Verdict::Failed(GeneratedRuntimeError::Refused(
                GeneratedQueryRefusal::CoverageRefused
            )))
        ));
    }

    #[test]
    fn generated_deferred_non_control_errors_keep_their_typed_cause() {
        let source = SourceId::new(0).unwrap();
        let errors = [
            sf_sparql::Error::Mapping("diagnostic".into()),
            sf_sparql::Error::Sql("diagnostic".into()),
            sf_sparql::Error::Core("diagnostic".into()),
        ];
        for error in errors {
            let text = error.to_string();
            let result = admitted::<()>(source, None, Err(GeneratedRuntimeError::Compiler(error)));
            let Err(Verdict::Failed(GeneratedRuntimeError::Compiler(kept))) = result else {
                panic!("typed compiler error must propagate: {text}");
            };
            assert_eq!(kept.to_string(), text);
        }
    }

    #[test]
    fn generated_compiler_failures_are_typed_problems_not_refusals() {
        let cases = [
            (
                sf_sparql::Error::Mapping("m".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                sf_sparql::Error::Sql("s".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                sf_sparql::Error::Core("c".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                sf_sparql::Error::QueryControl(QueryControlError::DeadlineExceeded),
                StatusCode::GATEWAY_TIMEOUT,
            ),
            (
                sf_sparql::Error::QueryControl(QueryControlError::CompilerWorkExceeded),
                StatusCode::TOO_MANY_REQUESTS,
            ),
        ];
        for (error, status) in cases {
            let text = error.to_string();
            let response = failure_response(GeneratedRuntimeError::Compiler(error));
            assert_eq!(response.status(), status, "{text}");
            assert!(problem::is_pending(&response), "{text}");
        }
    }

    #[test]
    fn a_refused_plan_is_denied_by_row_policy_before_refusal_detail() {
        let (cfg, _) = config(QueryShapeProfile::GeneratedSelectAsk, open());
        let lease = cfg.runtime_lease().unwrap();
        let source = SourceId::new(0).unwrap();
        let binding = binding_of(&lease, source).unwrap();
        let control = &UncontrolledQueryControl;
        let policy = |table: &str| {
            let rule = PortableRowRule::new(0, table, "tenant", "a").unwrap();
            PortableRowPolicy::new(vec![rule]).unwrap()
        };

        let outcome = binding.preflight_generated_deferred(CONSTRUCT, control);
        let denying = policy("other");
        assert!(matches!(
            admitted(source, Some(&denying), outcome),
            Err(Verdict::Denied)
        ));

        let construct = GeneratedQueryRefusal::Rule(ShapeRule::ConstructForm);
        let permitting = policy("people");
        for rows in [Some(&permitting), None] {
            let outcome = binding.preflight_generated_deferred(CONSTRUCT, control);
            let result = admitted(source, rows, outcome);
            let Err(Verdict::Failed(GeneratedRuntimeError::Refused(refusal))) = result else {
                panic!("permitted refusal must keep its named rule");
            };
            assert_eq!(refusal, construct);
        }
    }
}
