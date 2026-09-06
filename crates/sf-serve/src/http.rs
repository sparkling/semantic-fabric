//! Private SPARQL Protocol request pipeline and response negotiation.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Extension, RawQuery, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use sf_core::query_control::{QueryCharge, QueryControl};
use sf_sparql::{exec, exec_mysql, Plan, PlanForm};
use sparesults::QueryResultsFormat;

use crate::activation::RuntimeSnapshotLease;
use crate::admission;
use crate::backend::Backend;
use crate::budget::RequestBudget;
use crate::config::ServeConfig;
use crate::deadline::{self, JoinedTaskError};
use crate::pg_generation::VerifiedPostgresGenerationLease;
use crate::problem::{self, ProblemCode};
use crate::request_compile::BoundQuery;
use crate::request_deadline::RequestDeadlineService;
use crate::sqlite_admission;
use crate::stream::{self, RdfFormat};

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;

/// Build the governed query service plus fixed discovery and health controls.
pub fn router(cfg: Arc<ServeConfig>) -> RequestDeadlineService {
    let inner = Router::new()
        .route("/sparql", get(handle_get).post(handle_post))
        .route("/livez", get(crate::health::live))
        .route("/readyz", get(crate::health::ready))
        .fallback(problem::not_found)
        .method_not_allowed_fallback(problem::method_not_allowed)
        .with_state(cfg.clone());
    RequestDeadlineService::new(inner, cfg)
}

/// `GET /sparql?query=...` for the strict, single-query Protocol subset.
async fn handle_get(
    State(cfg): State<Arc<ServeConfig>>,
    Extension(budget): Extension<RequestBudget>,
    Extension(snapshot): Extension<RuntimeSnapshotLease>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    let query = match raw.as_deref() {
        Some(encoded) => {
            match crate::post_body::unique_query_param(encoded.as_bytes(), cfg.max_query_len()) {
                Ok(query) => query,
                Err(crate::post_body::QueryParamError::Invalid) => {
                    return problem::response(ProblemCode::InvalidRequest)
                }
                Err(crate::post_body::QueryParamError::TooLong) => {
                    return problem::response(ProblemCode::PayloadTooLarge)
                }
            }
        }
        // The outer service consumes the exact query-less discovery request.
        None => return problem::response(ProblemCode::InvalidRequest),
    };
    process(cfg, snapshot, query, accept(&headers), budget).await
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
    let accepted = accept(&parts.headers);
    let query = match crate::post_body::query(
        &parts.headers,
        body,
        cfg.max_query_len(),
        cfg.max_form_body_len(),
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

    let mut generations =
        match crate::request_generation::acquire(cfg.clone(), &snapshot, &query, &budget).await {
            Ok(generations) => generations,
            Err(response) => return response,
        };
    let bound =
        match crate::request_compile::compile(cfg.clone(), snapshot.clone(), query, budget.clone())
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
            if let Err(error) = admission::admit(bound.plan(), cfg.max_order_rows()) {
                let _internal_reason = error.reason();
                let _ = generations.finish().await;
                return problem::response(ProblemCode::UnsupportedQuery);
            }
            let execution = match snapshot.prepare_execution(*bound) {
                Ok(execution) => execution,
                Err(_) => {
                    let _ = generations.finish().await;
                    return problem::response(ProblemCode::Internal);
                }
            };
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
                    respond_select(backend, plan, generation, accept, budget).await
                }
                PlanForm::Ask => respond_ask(backend, plan, generation, accept, budget).await,
                PlanForm::Construct { .. } => {
                    respond_construct(backend, plan, generation, accept, budget).await
                }
            }
        }
        BoundQuery::Federated(bound) => {
            for fragment in bound.plan().fragments() {
                if let Err(error) = admission::admit(fragment.plan(), cfg.max_order_rows()) {
                    let _internal_reason = error.reason();
                    let _ = generations.finish().await;
                    return problem::response(ProblemCode::UnsupportedQuery);
                }
            }
            let execution = match snapshot.prepare_federated_execution(*bound) {
                Ok(execution) => execution,
                Err(_) => {
                    let _ = generations.finish().await;
                    return problem::response(ProblemCode::Internal);
                }
            };
            let format = negotiate_results(accept);
            match crate::federation::select_union_body(execution, generations, format, budget).await
            {
                Ok(body) => ok_stream(format.media_type(), body),
                Err(response) => response,
            }
        }
    }
}

/// Stream a SELECT (ADR-0010 §C). The status line is committed once streaming
/// begins, so the recoverable errors (parse → 400, unsupported → 501) are already
/// resolved by [`compile`]; an execution failure or a passed deadline errors the
/// body mid-stream (same posture as the SQLite CONSTRUCT path). HTTP 200 is already
/// committed then, so this slice does not claim an atomic/no-prefix result body.
async fn respond_select(
    backend: Backend,
    plan: Arc<Plan>,
    generation: Option<VerifiedPostgresGenerationLease>,
    accept: Option<&str>,
    budget: RequestBudget,
) -> Response {
    let fmt = negotiate_results(accept);
    let PlanForm::Select { vars } = &plan.form else {
        return problem::response(ProblemCode::Internal);
    };
    let vars = vars.clone();
    let body = match backend {
        Backend::Sqlite(pool) => {
            if generation.is_some() {
                return problem::response(ProblemCode::Internal);
            }
            let lease = match sqlite_admission::acquire(&pool, &budget).await {
                Ok(lease) => lease,
                Err(response) => return response,
            };
            let drive_control: Arc<dyn QueryControl> = Arc::new(budget.clone());
            stream::select_body_streaming_controlled(
                move |sink| {
                    Box::pin(async move {
                        exec::select_each_sqlite_owned_interruptible_leased(
                            &plan,
                            lease,
                            drive_control,
                            sink,
                        )
                        .await
                    })
                },
                fmt,
                vars,
                budget,
            )
        }
        Backend::Pg(pool) => {
            match crate::pg_response::select(pool, plan, generation, fmt, vars, budget).await {
                Ok(body) => body,
                Err(response) => return response,
            }
        }
        Backend::Mysql(pool) => {
            if generation.is_some() {
                return problem::response(ProblemCode::Internal);
            }
            let conn = match crate::source_acquisition::acquire_mysql(&pool, &budget).await {
                Ok(conn) => conn,
                Err(response) => return response,
            };
            let drive_budget = budget.clone();
            stream::select_body_streaming_controlled(
                move |sink| {
                    Box::pin(async move {
                        exec_mysql::select_each_mysql_controlled(&plan, conn, &drive_budget, sink)
                            .await
                    })
                },
                fmt,
                vars,
                budget,
            )
        }
    };
    ok_stream(fmt.media_type(), body)
}

async fn respond_ask(
    backend: Backend,
    plan: Arc<Plan>,
    generation: Option<VerifiedPostgresGenerationLease>,
    accept: Option<&str>,
    budget: RequestBudget,
) -> Response {
    if let Err(error) = budget.preflight_ask_result() {
        return problem::response_for_control(error);
    }
    let fmt = negotiate_results(accept);
    let value = match backend {
        Backend::Sqlite(pool) => {
            if generation.is_some() {
                return problem::response(ProblemCode::Internal);
            }
            // The concrete adapter future proves the `Send` obligation.
            let lease = match sqlite_admission::acquire(&pool, &budget).await {
                Ok(lease) => lease,
                Err(response) => return response,
            };
            let task_control: Arc<dyn QueryControl> = Arc::new(budget.clone());
            let run = tokio::spawn(async move {
                exec::ask_sqlite_owned_interruptible_leased(&plan, lease, task_control).await
            });
            match deadline::join_task(budget.clone(), run).await {
                Err(JoinedTaskError::Control(error)) => {
                    return problem::response_for_control(error)
                }
                Err(JoinedTaskError::Join(_)) => return problem::response(ProblemCode::Internal),
                Ok(r) => r,
            }
        }
        Backend::Pg(pool) => {
            match crate::pg_response::ask(pool, plan, generation, budget.clone()).await {
                Ok(result) => result,
                Err(response) => return response,
            }
        }
        Backend::Mysql(pool) => {
            if generation.is_some() {
                return problem::response(ProblemCode::Internal);
            }
            // ASK collects (a single boolean). Unlike PG (whose `PgRowStream` is
            // `'static`), MySQL's branch cursor BORROWS the connection, so awaiting
            // `ask_mysql` inline in this handler future leaves the borrowing stream
            // held across an await — an HRTB `Send` obligation axum's handler future
            // cannot discharge. `tokio::spawn` checks `Send` on the concrete
            // owned-`Conn` task future directly (provable), and gives the dedicated
            // conn a task to live in, dropped/disposed after the run (§4.2). Mirrors
            // the SQLite ASK arm's `tokio::spawn` + `Ok(Err)/Ok(Ok)` join handling.
            let conn = match crate::source_acquisition::acquire_mysql(&pool, &budget).await {
                Ok(conn) => conn,
                Err(response) => return response,
            };
            let task_budget = budget.clone();
            let run = tokio::spawn(async move {
                exec_mysql::ask_each_mysql_controlled(&plan, conn, &task_budget).await
            });
            match deadline::join_task(budget.clone(), run).await {
                Err(JoinedTaskError::Control(error)) => {
                    return problem::response_for_control(error)
                }
                Err(JoinedTaskError::Join(_)) => return problem::response(ProblemCode::Internal),
                Ok(r) => r,
            }
        }
    };
    match value {
        Ok(b) => {
            if let Err(error) = budget.checkpoint() {
                return problem::response_for_control(error);
            }
            match stream::serialize_boolean(b, fmt) {
                Ok(bytes) => {
                    let Ok(amount) = u64::try_from(bytes.len()) else {
                        return problem::response(ProblemCode::Internal);
                    };
                    if let Err(error) = budget.consume(QueryCharge::SerializedBytes, amount) {
                        return problem::response_for_control(error);
                    }
                    if let Err(error) = budget.checkpoint() {
                        return problem::response_for_control(error);
                    }
                    ok_stream(fmt.media_type(), stream::collected_body(bytes))
                }
                Err(_) => problem::response(ProblemCode::Internal),
            }
        }
        Err(e) => problem::response_for_sparql(&e),
    }
}

/// Stream a CONSTRUCT (ADR-0010 §C) — triples flow from the executor sink through
/// the RDF serialiser into the body, never collected, on **both** backends.
async fn respond_construct(
    backend: Backend,
    plan: Arc<Plan>,
    generation: Option<VerifiedPostgresGenerationLease>,
    accept: Option<&str>,
    budget: RequestBudget,
) -> Response {
    let fmt = negotiate_rdf(accept);
    let body = match backend {
        Backend::Sqlite(pool) => {
            if generation.is_some() {
                return problem::response(ProblemCode::Internal);
            }
            let lease = match sqlite_admission::acquire(&pool, &budget).await {
                Ok(lease) => lease,
                Err(response) => return response,
            };
            let drive_control: Arc<dyn QueryControl> = Arc::new(budget.clone());
            stream::construct_body_streaming_controlled(
                move |sink| {
                    Box::pin(async move {
                        exec::construct_each_sqlite_owned_interruptible_leased(
                            &plan,
                            lease,
                            drive_control,
                            sink,
                        )
                        .await
                    })
                },
                fmt,
                budget,
            )
        }
        Backend::Pg(pool) => {
            match crate::pg_response::construct(pool, plan, generation, fmt, budget).await {
                Ok(body) => body,
                Err(response) => return response,
            }
        }
        Backend::Mysql(pool) => {
            if generation.is_some() {
                return problem::response(ProblemCode::Internal);
            }
            let conn = match crate::source_acquisition::acquire_mysql(&pool, &budget).await {
                Ok(conn) => conn,
                Err(response) => return response,
            };
            let drive_budget = budget.clone();
            stream::construct_body_streaming_controlled(
                move |sink| {
                    Box::pin(async move {
                        exec_mysql::construct_each_mysql_controlled(
                            &plan,
                            conn,
                            &drive_budget,
                            sink,
                        )
                        .await
                    })
                },
                fmt,
                budget,
            )
        }
    };
    ok_stream(fmt.media_type(), body)
}

/// The sole decoded `query` field, rejecting duplicates and every other key.
#[cfg(test)]
fn form_param(encoded: &str, key: &str) -> Option<String> {
    (key == "query")
        .then(|| crate::post_body::unique_query_param(encoded.as_bytes(), usize::MAX).ok())
        .flatten()
}

fn accept(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned())
}

/// Negotiate the SELECT/ASK results format from `Accept` (default: Results JSON).
fn negotiate_results(accept: Option<&str>) -> QueryResultsFormat {
    let a = accept.unwrap_or("").to_ascii_lowercase();
    if a.contains("sparql-results+xml") || a.contains("application/xml") || a.contains("text/xml") {
        QueryResultsFormat::Xml
    } else if a.contains("text/tab-separated-values") {
        QueryResultsFormat::Tsv
    } else if a.contains("text/csv") {
        QueryResultsFormat::Csv
    } else {
        QueryResultsFormat::Json
    }
}

/// Negotiate the CONSTRUCT/DESCRIBE RDF format from `Accept` (default: Turtle).
fn negotiate_rdf(accept: Option<&str>) -> RdfFormat {
    let a = accept.unwrap_or("").to_ascii_lowercase();
    if a.contains("application/ld+json") {
        RdfFormat::JsonLd
    } else if a.contains("application/n-triples") {
        RdfFormat::NTriples
    } else {
        RdfFormat::Turtle
    }
}

fn ok_stream(content_type: &str, body: Body) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(body)
        .expect("static response builder")
}
