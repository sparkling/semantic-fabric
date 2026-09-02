//! Private SPARQL Protocol request pipeline and response negotiation.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Extension, RawQuery, State};
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use deadpool_postgres::PoolError;
use sf_core::query_control::{QueryCharge, QueryControl};
use sf_sparql::{exec, exec_mysql, exec_pg, Plan, PlanForm};
use sparesults::QueryResultsFormat;

use crate::admission;
use crate::backend::{Backend, PgConn};
use crate::binding::BoundPlan;
use crate::budget::RequestBudget;
use crate::config::ServeConfig;
use crate::deadline::{self, CompilerRunError, JoinedTaskError};
use crate::problem::{self, ProblemCode};
use crate::request_deadline::RequestDeadlineService;
use crate::stream::{self, RdfFormat};

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;

/// Build the governed request service exposing `GET`/`POST /sparql` over `cfg`.
pub fn router(cfg: Arc<ServeConfig>) -> RequestDeadlineService {
    let inner = Router::new()
        .route("/sparql", get(handle_get).post(handle_post))
        .fallback(problem::not_found)
        .method_not_allowed_fallback(problem::method_not_allowed)
        .with_state(cfg.clone());
    RequestDeadlineService::new(inner, cfg)
}

/// `GET /sparql?query=...` for the strict, single-query Protocol subset.
async fn handle_get(
    State(cfg): State<Arc<ServeConfig>>,
    Extension(budget): Extension<RequestBudget>,
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
        None => return problem::response(ProblemCode::InvalidRequest),
    };
    process(cfg, query, accept(&headers), budget).await
}

/// `POST /sparql` — either a strict single-query urlencoded form or a bounded
/// raw `application/sparql-query` body (SPARQL 1.2 Protocol §2.2.2–2.2.3).
async fn handle_post(
    State(cfg): State<Arc<ServeConfig>>,
    Extension(budget): Extension<RequestBudget>,
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
    process(cfg, query, accepted, budget).await
}

/// The shared request pipeline: cap → compile → dispatch by query form → stream.
async fn process(
    cfg: Arc<ServeConfig>,
    query: String,
    accept: Option<String>,
    budget: RequestBudget,
) -> Response {
    if query.len() > cfg.max_query_len() {
        return problem::response(ProblemCode::PayloadTooLarge);
    }

    let bound = match compile(cfg.clone(), query, budget.clone()).await {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    if let Err(error) = admission::admit(bound.plan()) {
        let _internal_state = error.state();
        return problem::response(ProblemCode::UnsupportedQuery);
    }
    let execution = match cfg.prepare_execution(bound) {
        Ok(execution) => execution,
        Err(_) => return problem::response(ProblemCode::Internal),
    };
    let (backend, plan) = execution.into_parts();
    let accept = accept.as_deref();

    match &plan.form {
        PlanForm::Select { .. } => respond_select(backend, plan, accept, budget).await,
        PlanForm::Ask => respond_ask(backend, plan, accept, budget).await,
        PlanForm::Construct { .. } => respond_construct(backend, plan, accept, budget).await,
    }
}

/// Compile (parse + rewrite) off the async runtime (ADR-0006); map errors to status.
/// Uses the per-config plan cache (ADR-0007): repeated queries at the same epoch
/// skip the full rewrite and return a cached plan clone. Timeout stops the request
/// waiter, not CPU work already running; the owned admission permit stays charged
/// until that detached blocking closure actually returns.
async fn compile(
    cfg: Arc<ServeConfig>,
    query: String,
    budget: RequestBudget,
) -> Result<BoundPlan, Response> {
    let permits = cfg.compiler_permits();
    let compiled = deadline::run_compiler(budget, permits, move || cfg.compile(&query)).await;
    match compiled {
        Err(CompilerRunError::Control(error)) => Err(problem::response_for_control(error)),
        Err(CompilerRunError::AdmissionClosed | CompilerRunError::Join(_)) => {
            Err(problem::response(ProblemCode::Internal))
        }
        Ok(Err(e)) => Err(problem::response_for_sparql(&e)),
        Ok(Ok(plan)) => Ok(plan),
    }
}

/// Stream a SELECT (ADR-0010 §C). The status line is committed once streaming
/// begins, so the recoverable errors (parse → 400, unsupported → 501) are already
/// resolved by [`compile`]; an execution failure or a passed deadline errors the
/// body mid-stream (same posture as the SQLite CONSTRUCT path). HTTP 200 is already
/// committed then, so this slice does not claim an atomic/no-prefix result body.
async fn respond_select(
    backend: Backend,
    plan: Plan,
    accept: Option<&str>,
    budget: RequestBudget,
) -> Response {
    let fmt = negotiate_results(accept);
    let PlanForm::Select { vars } = &plan.form else {
        return problem::response(ProblemCode::Internal);
    };
    let vars = vars.clone();
    // Every backend drives the generic streamer; SQLite owns its blocking cap-1
    // bridge rather than giving sf-serve a special case (ADR-0024 M5).
    let body = match backend {
        Backend::Sqlite(pool) => {
            let conn = pool.pick();
            let drive_budget = budget.clone();
            stream::select_body_streaming_controlled(
                move |sink| {
                    Box::pin(async move {
                        exec::select_each_sqlite_owned_controlled(&plan, conn, &drive_budget, sink)
                            .await
                    })
                },
                fmt,
                vars,
                budget,
            )
        }
        Backend::Pg(pool) => {
            let conn = match acquire_pg(&pool, budget.clone()).await {
                Ok(c) => c,
                Err(resp) => return resp,
            };
            let drive_budget = budget.clone();
            stream::select_body_streaming_controlled(
                move |sink| {
                    Box::pin(async move {
                        exec_pg::select_each_pg_controlled(&plan, conn, &drive_budget, sink).await
                    })
                },
                fmt,
                vars,
                budget,
            )
        }
        Backend::Mysql(pool) => {
            let conn = match acquire_mysql(&pool, &budget).await {
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
    plan: Plan,
    accept: Option<&str>,
    budget: RequestBudget,
) -> Response {
    if let Err(error) = budget.preflight_ask_result() {
        return problem::response_for_control(error);
    }
    let fmt = negotiate_results(accept);
    let value = match backend {
        Backend::Sqlite(pool) => {
            // The adapter owns SQLite's blocking; spawning its concrete future
            // proves the `Send` obligation directly (ADR-0024 M5).
            let conn = pool.pick();
            let task_budget = budget.clone();
            let run = tokio::spawn(async move {
                exec::ask_sqlite_owned_controlled(&plan, conn, &task_budget).await
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
            let conn = match acquire_pg(&pool, budget.clone()).await {
                Ok(c) => c,
                Err(resp) => return resp,
            };
            match budget
                .run(exec_pg::ask_pg_controlled(&plan, conn, &budget))
                .await
            {
                Err(error) => return problem::response_for_control(error),
                Ok(result) => result,
            }
        }
        Backend::Mysql(pool) => {
            // ASK collects (a single boolean). Unlike PG (whose `PgRowStream` is
            // `'static`), MySQL's branch cursor BORROWS the connection, so awaiting
            // `ask_mysql` inline in this handler future leaves the borrowing stream
            // held across an await — an HRTB `Send` obligation axum's handler future
            // cannot discharge. `tokio::spawn` checks `Send` on the concrete
            // owned-`Conn` task future directly (provable), and gives the dedicated
            // conn a task to live in, dropped/disposed after the run (§4.2). Mirrors
            // the SQLite ASK arm's `tokio::spawn` + `Ok(Err)/Ok(Ok)` join handling.
            let conn = match acquire_mysql(&pool, &budget).await {
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
    plan: Plan,
    accept: Option<&str>,
    budget: RequestBudget,
) -> Response {
    let fmt = negotiate_rdf(accept);
    let body = match backend {
        Backend::Sqlite(pool) => {
            let conn = pool.pick();
            let drive_budget = budget.clone();
            stream::construct_body_streaming_controlled(
                move |sink| {
                    Box::pin(async move {
                        exec::construct_each_sqlite_owned_controlled(
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
        Backend::Pg(pool) => {
            let conn = match acquire_pg(&pool, budget.clone()).await {
                Ok(c) => c,
                Err(resp) => return resp,
            };
            let drive_budget = budget.clone();
            stream::construct_body_streaming_controlled(
                move |sink| {
                    Box::pin(async move {
                        exec_pg::construct_each_pg_controlled(&plan, conn, &drive_budget, sink)
                            .await
                    })
                },
                fmt,
                budget,
            )
        }
        Backend::Mysql(pool) => {
            let conn = match acquire_mysql(&pool, &budget).await {
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

async fn acquire_mysql(
    pool: &mysql_async::Pool,
    budget: &RequestBudget,
) -> Result<mysql_async::Conn, Response> {
    match budget.run(pool.get_conn()).await {
        Err(error) => Err(problem::response_for_control(error)),
        Ok(Err(_)) => Err(problem::response(ProblemCode::SourceUnavailable)),
        Ok(Ok(conn)) => Ok(conn),
    }
}

/// Acquire a pooled PostgreSQL connection (ADR-0010 §C stream-lane pool, ADR-0027;
/// M4 wave-2 finding 2). Pool exhaustion (no free connection within the
/// configured `--pg-pool-wait-secs`) is shed as a fast, honest `503` +
/// `Retry-After` rather than queued indefinitely or reported as a generic `500`
/// — the ADR-0010 "shed overflow" clause this pass implements.
async fn acquire_pg(
    pool: &deadpool_postgres::Pool,
    budget: RequestBudget,
) -> Result<PgConn, Response> {
    let acquired = match budget.run(pool.get()).await {
        Ok(result) => result,
        Err(error) => return Err(problem::response_for_control(error)),
    };
    let conn = acquired.map_err(|e| match e {
        PoolError::Timeout(_) => {
            let mut resp = problem::response(ProblemCode::SourceUnavailable);
            resp.headers_mut().insert(
                header::RETRY_AFTER,
                // Fixed at 1s rather than derived from pool pressure/wait-time —
                // a pressure-aware value is future work (ADR-0010 status
                // correction part 2's second open refinement).
                HeaderValue::from_static("1"),
            );
            resp
        }
        _ => problem::response(ProblemCode::Internal),
    })?;
    match budget.run(PgConn::checked(conn)).await {
        Err(error) => Err(problem::response_for_control(error)),
        Ok(Err(_)) => Err(problem::response(ProblemCode::Internal)),
        Ok(Ok(conn)) => Ok(conn),
    }
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
