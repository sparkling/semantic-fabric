//! Private SPARQL Protocol request pipeline and response negotiation.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Extension, RawQuery, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use sf_core::query_control::{QueryCharge, QueryControl};
use sf_sparql::{exec, Plan, PlanForm};

use crate::activation::RuntimeSnapshotLease;
use crate::admission;
use crate::backend::Backend;
use crate::budget::RequestBudget;
use crate::config::ServeConfig;
use crate::deadline::{self, JoinedTaskError};
use crate::metrics::MetricsEndpoint;
use crate::pg_generation::VerifiedPostgresGenerationLease;
use crate::problem::{self, ProblemCode};
use crate::request_compile::BoundQuery;
use crate::request_deadline::RequestDeadlineService;
use crate::sqlite_admission;
use crate::stream;
use crate::telemetry::{
    execute as traced_execute, in_stage as traced, in_stage_sync as traced_sync, Stage,
};

#[path = "http_negotiation.rs"]
mod negotiation;
#[path = "http_response.rs"]
mod response;
use response::{respond_ask, respond_construct, respond_select};
#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
use negotiation::{negotiate_rdf, negotiate_results};

/// Build the governed query service plus fixed discovery and health controls.
pub fn router(cfg: Arc<ServeConfig>) -> RequestDeadlineService {
    build_router(cfg, None)
}

/// Build the query service with an explicitly enabled Prometheus control route.
pub fn router_with_metrics(
    cfg: Arc<ServeConfig>,
    endpoint: MetricsEndpoint,
) -> RequestDeadlineService {
    build_router(cfg, Some(endpoint))
}

fn build_router(cfg: Arc<ServeConfig>, metrics: Option<MetricsEndpoint>) -> RequestDeadlineService {
    let metrics_enabled = metrics.is_some();
    let mut inner = Router::new()
        .route("/sparql", get(handle_get).post(handle_post))
        .route("/livez", get(crate::health::live))
        .route("/readyz", get(crate::health::ready))
        .fallback(problem::not_found)
        .method_not_allowed_fallback(problem::method_not_allowed);
    if let Some(endpoint) = metrics {
        inner = inner.route(
            "/metrics",
            get(move || {
                let endpoint = endpoint.clone();
                async move { endpoint.response() }
            }),
        );
    }
    RequestDeadlineService::new(inner.with_state(cfg.clone()), cfg, metrics_enabled)
}

/// `GET /sparql?query=...` for the strict, single-query Protocol subset.
async fn handle_get(
    State(cfg): State<Arc<ServeConfig>>,
    Extension(budget): Extension<RequestBudget>,
    Extension(snapshot): Extension<RuntimeSnapshotLease>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    let decoded = traced_sync(Stage::Decode, || match raw.as_deref() {
        Some(encoded) => {
            crate::post_body::unique_query_param(encoded.as_bytes(), cfg.max_query_len()).map_err(
                |error| match error {
                    crate::post_body::QueryParamError::Invalid => ProblemCode::InvalidRequest,
                    crate::post_body::QueryParamError::TooLong => ProblemCode::PayloadTooLarge,
                },
            )
        }
        None => Err(ProblemCode::InvalidRequest),
    });
    let query = match decoded {
        Ok(query) => query,
        Err(code) => return problem::response(code),
    };
    let accept = match accept(&headers) {
        Ok(accept) => accept,
        Err(code) => return problem::response(code),
    };
    process(cfg, snapshot, query, accept, budget).await
}

/// `POST /sparql` — either a strict single-query urlencoded form or a bounded
/// raw `application/sparql-query` body (SPARQL 1.2 Protocol §2.2.2–2.2.3).
async fn handle_post(
    State(cfg): State<Arc<ServeConfig>>,
    Extension(budget): Extension<RequestBudget>,
    Extension(snapshot): Extension<RuntimeSnapshotLease>,
    request: Request<Body>,
) -> Response {
    if request.uri().query().is_some() {
        return problem::response(ProblemCode::InvalidRequest);
    }
    let (parts, body) = request.into_parts();
    let accepted = match accept(&parts.headers) {
        Ok(accept) => accept,
        Err(code) => return problem::response(code),
    };
    let query = match traced(
        Stage::Decode,
        crate::post_body::query(
            &parts.headers,
            body,
            cfg.max_query_len(),
            cfg.max_form_body_len(),
        ),
    )
    .await
    {
        Ok(query) => query,
        Err(code) => return problem::response(code),
    };
    process(cfg, snapshot, query, accepted, budget).await
}

/// The shared request pipeline: cap → compile → dispatch by query form → stream.
async fn process(
    cfg: Arc<ServeConfig>,
    snapshot: RuntimeSnapshotLease,
    query: String,
    accept: Option<String>,
    budget: RequestBudget,
) -> Response {
    if query.len() > cfg.max_query_len() {
        return problem::response(ProblemCode::PayloadTooLarge);
    }

    // Prove the requested provenance profile before generation admission can
    // perform source I/O. The same pinned snapshot and unchanged query are used
    // below; neither public caller plans nor another generation can be substituted.
    let lineage = match crate::lineage::prepare(
        cfg.clone(),
        snapshot.clone(),
        &query,
        accept.as_deref(),
        &budget,
    )
    .await
    {
        Ok(proof) => proof,
        Err(response) => return response,
    };

    let generation_admission = match traced(
        Stage::GenerationLease,
        crate::request_generation::acquire(cfg.clone(), &snapshot, &query, &budget),
    )
    .await
    {
        Ok(admission) => admission,
        Err(response) => return response,
    };
    let (mut generations, compiler) = generation_admission.into_parts();
    let bound = match traced(
        Stage::Compile,
        crate::request_compile::compile(
            cfg.clone(),
            snapshot.clone(),
            query,
            budget.clone(),
            compiler,
        ),
    )
    .await
    {
        Ok(p) => p,
        Err(response) => {
            let _ = generations.finish().await;
            return response;
        }
    };
    let accept = accept.as_deref();

    match bound {
        BoundQuery::Single(bound) => {
            let admitted = traced_sync(Stage::ShapeAdmission, || {
                admission::admit(bound.plan(), cfg.max_order_rows())
            });
            if let Err(error) = admitted {
                let _internal_reason = error.reason();
                let _ = generations.finish().await;
                return problem::response(ProblemCode::UnsupportedQuery);
            }
            let execution = match traced_sync(Stage::BindExecution, || {
                snapshot.prepare_request_execution(*bound, &budget)
            }) {
                Ok(execution) => execution,
                Err(_) => {
                    let _ = generations.finish().await;
                    return problem::response(ProblemCode::Internal);
                }
            };
            let rls_tables = execution.rls_tables();
            let (source_id, binding_identity, backend, verified_generation, plan) =
                execution.into_parts();
            if !generations.matches(source_id, &binding_identity, verified_generation) {
                let _ = generations.finish().await;
                return problem::response(ProblemCode::Internal);
            }
            let generation = generations.take(source_id, &binding_identity);
            if !generations.is_empty() {
                let _ = generations.finish().await;
                return problem::response(ProblemCode::Internal);
            }
            match &plan.form {
                PlanForm::Select { .. } => {
                    let format = match lineage {
                        Some(proof) => stream::SelectFormat::Lineage(proof),
                        None => negotiate_results(accept).into(),
                    };
                    traced_execute(respond_select(
                        backend, plan, generation, rls_tables, format, budget,
                    ))
                    .await
                }
                PlanForm::Ask => {
                    traced_execute(respond_ask(
                        backend, plan, generation, rls_tables, accept, budget,
                    ))
                    .await
                }
                PlanForm::Construct { .. } => {
                    let format = match lineage {
                        Some(proof) => stream::GraphFormat::Lineage(proof),
                        None => negotiate_rdf(accept).into(),
                    };
                    traced_execute(respond_construct(
                        backend, plan, generation, rls_tables, format, budget,
                    ))
                    .await
                }
            }
        }
        BoundQuery::Federated(bound) => {
            for fragment in bound.plan().fragments() {
                let admitted = traced_sync(Stage::ShapeAdmission, || {
                    admission::admit(fragment.plan(), cfg.max_order_rows())
                });
                if let Err(error) = admitted {
                    let _internal_reason = error.reason();
                    let _ = generations.finish().await;
                    return problem::response(ProblemCode::UnsupportedQuery);
                }
            }
            let execution = match traced_sync(Stage::BindExecution, || {
                snapshot.prepare_federated_request_execution(*bound, &budget)
            }) {
                Ok(execution) => execution,
                Err(_) => {
                    let _ = generations.finish().await;
                    return problem::response(ProblemCode::Internal);
                }
            };
            let format = negotiate_results(accept);
            match traced_execute(crate::federation::select_union_body(
                execution,
                generations,
                format,
                budget,
            ))
            .await
            {
                Ok(body) => ok_stream(format.media_type(), body),
                Err(response) => response,
            }
        }
    }
}

fn accept(headers: &HeaderMap) -> Result<Option<String>, ProblemCode> {
    let mut joined = String::new();
    for (index, value) in headers.get_all(header::ACCEPT).iter().enumerate() {
        let value = value.to_str().map_err(|_| ProblemCode::InvalidRequest)?;
        if index >= 16 || joined.len().saturating_add(value.len()).saturating_add(1) > 8192 {
            return Err(ProblemCode::InvalidRequest);
        }
        if index != 0 {
            joined.push(',');
        }
        joined.push_str(value);
    }
    Ok((!joined.is_empty()).then_some(joined))
}

fn ok_stream(content_type: &str, body: Body) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(body)
        .expect("static response builder")
}
