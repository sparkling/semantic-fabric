//! Private SPARQL Protocol request pipeline and response negotiation.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Extension, RawQuery, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use deadpool_postgres::PoolError;
use sf_sparql::{exec, exec_mysql, exec_pg, Error as SparqlError, Plan, PlanForm};
use sparesults::QueryResultsFormat;

use crate::admission;
use crate::backend::{Backend, PgConn};
use crate::binding::BoundPlan;
use crate::config::ServeConfig;
use crate::deadline::{self, CompilerRunError, JoinedTaskError, RequestDeadline};
use crate::problem::{self, ProblemCode};
use crate::stream::{self, RdfFormat};

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;

/// Build the axum router exposing `GET`/`POST /sparql` over `cfg`.
pub fn router(cfg: Arc<ServeConfig>) -> Router {
    let deadline_state = cfg.clone();
    Router::new()
        .route("/sparql", get(handle_get).post(handle_post))
        .layer(middleware::from_fn_with_state(
            deadline_state,
            begin_request_deadline,
        ))
        .with_state(cfg)
}

/// Mint and enforce the request's one absolute deadline before route extractors
/// consume the body. Streaming continues after response headers and therefore
/// also receives this same instant explicitly.
async fn begin_request_deadline(
    State(cfg): State<Arc<ServeConfig>>,
    mut request: Request,
    next: Next,
) -> Response {
    let deadline = RequestDeadline::after(cfg.timeout);
    request.extensions_mut().insert(deadline);
    match deadline.run(next.run(request)).await {
        Ok(response) => response,
        Err(_) => timeout_response(),
    }
}

/// `GET /sparql?query=...` (SPARQL 1.2 Protocol query via URL parameters).
async fn handle_get(
    State(cfg): State<Arc<ServeConfig>>,
    Extension(deadline): Extension<RequestDeadline>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    let Some(query) = raw.as_deref().and_then(|q| form_param(q, "query")) else {
        return problem::response(ProblemCode::InvalidRequest);
    };
    process(cfg, query, accept(&headers), deadline).await
}

/// `POST /sparql` — either `application/x-www-form-urlencoded` (`query=...`) or a
/// raw `application/sparql-query` body (SPARQL 1.2 Protocol §2.1.2).
async fn handle_post(
    State(cfg): State<Arc<ServeConfig>>,
    Extension(deadline): Extension<RequestDeadline>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ctype = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("").trim().to_owned())
        .unwrap_or_default();
    let query = match ctype.as_str() {
        "application/sparql-query" => match String::from_utf8(body.to_vec()) {
            Ok(q) => q,
            Err(_) => return problem::response(ProblemCode::InvalidRequest),
        },
        "application/x-www-form-urlencoded" => {
            match std::str::from_utf8(&body)
                .ok()
                .and_then(|s| form_param(s, "query"))
            {
                Some(q) => q,
                None => return problem::response(ProblemCode::InvalidRequest),
            }
        }
        _ => return problem::response(ProblemCode::UnsupportedMediaType),
    };
    process(cfg, query, accept(&headers), deadline).await
}

/// The shared request pipeline: cap → compile → dispatch by query form → stream.
async fn process(
    cfg: Arc<ServeConfig>,
    query: String,
    accept: Option<String>,
    deadline: RequestDeadline,
) -> Response {
    if query.len() > cfg.max_query_len {
        return problem::response(ProblemCode::PayloadTooLarge);
    }

    let bound = match compile(cfg.clone(), query, deadline).await {
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
        PlanForm::Select { .. } => respond_select(backend, plan, accept, deadline).await,
        PlanForm::Ask => respond_ask(backend, plan, accept, deadline).await,
        PlanForm::Construct { .. } => respond_construct(backend, plan, accept, deadline).await,
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
    deadline: RequestDeadline,
) -> Result<BoundPlan, Response> {
    let permits = cfg.compiler_permits();
    let compiled = deadline::run_compiler(deadline, permits, move || cfg.compile(&query)).await;
    match compiled {
        Err(CompilerRunError::Deadline(_)) => Err(timeout_response()),
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
    deadline: RequestDeadline,
) -> Response {
    let fmt = negotiate_results(accept);
    let PlanForm::Select { vars } = &plan.form else {
        return problem::response(ProblemCode::Internal);
    };
    let vars = vars.clone();
    // Uniform lane (ADR-0024 M5): every backend drives the generic streamer via a
    // thin concrete `exec_*::select_each_*` closure. SQLite's blocking now lives in
    // the adapter's cap-1 bridge (no sf-serve spawn_blocking special-case).
    let body = match backend {
        Backend::Sqlite(pool) => {
            let conn = pool.pick();
            stream::select_body_streaming(
                move |sink| {
                    Box::pin(async move { exec::select_each_sqlite_owned(&plan, conn, sink).await })
                },
                fmt,
                vars,
                Some(deadline.into_std()),
            )
        }
        Backend::Pg(pool) => {
            let conn = match acquire_pg(&pool, deadline).await {
                Ok(c) => c,
                Err(resp) => return resp,
            };
            stream::select_body_streaming(
                move |sink| {
                    Box::pin(async move { exec_pg::select_each_pg(&plan, conn, sink).await })
                },
                fmt,
                vars,
                Some(deadline.into_std()),
            )
        }
        Backend::Mysql(pool) => stream::select_body_streaming(
            move |sink| {
                Box::pin(async move {
                    let conn = pool
                        .get_conn()
                        .await
                        .map_err(|e| SparqlError::Sql(e.to_string()))?;
                    exec_mysql::select_each_mysql(&plan, conn, sink).await
                })
            },
            fmt,
            vars,
            Some(deadline.into_std()),
        ),
    };
    ok_stream(fmt.media_type(), body)
}

async fn respond_ask(
    backend: Backend,
    plan: Plan,
    accept: Option<&str>,
    deadline: RequestDeadline,
) -> Response {
    let fmt = negotiate_results(accept);
    let value = match backend {
        Backend::Sqlite(pool) => {
            // Uniform lane (ADR-0024 M5): SQLite ASK spawns the owned cap-1-bridge
            // backend onto the runtime like the MySQL arm — no `spawn_blocking`
            // special-case; the adapter owns SQLite's blocking. `tokio::spawn` checks
            // `Send` on the concrete owned-backend future directly (provable).
            let conn = pool.pick();
            let run = tokio::spawn(async move { exec::ask_sqlite_owned(&plan, conn).await });
            match deadline::join_task(deadline, run).await {
                Err(JoinedTaskError::Deadline(_)) => return timeout_response(),
                Err(JoinedTaskError::Join(_)) => return problem::response(ProblemCode::Internal),
                Ok(r) => r,
            }
        }
        Backend::Pg(pool) => {
            let conn = match acquire_pg(&pool, deadline).await {
                Ok(c) => c,
                Err(resp) => return resp,
            };
            match deadline.run(exec_pg::ask_pg(&plan, conn)).await {
                Err(_) => return timeout_response(),
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
            let run = tokio::spawn(async move {
                let conn = pool
                    .get_conn()
                    .await
                    .map_err(|e| SparqlError::Sql(e.to_string()))?;
                exec_mysql::ask_each_mysql(&plan, conn).await
            });
            match deadline::join_task(deadline, run).await {
                Err(JoinedTaskError::Deadline(_)) => return timeout_response(),
                Err(JoinedTaskError::Join(_)) => return problem::response(ProblemCode::Internal),
                Ok(r) => r,
            }
        }
    };
    match value {
        Ok(b) => {
            if deadline.check().is_err() {
                return timeout_response();
            }
            match stream::serialize_boolean(b, fmt) {
                Ok(bytes) if deadline.check().is_ok() => {
                    ok_stream(fmt.media_type(), stream::collected_body(bytes))
                }
                Ok(_) => timeout_response(),
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
    deadline: RequestDeadline,
) -> Response {
    let fmt = negotiate_rdf(accept);
    let body = match backend {
        Backend::Sqlite(pool) => {
            let conn = pool.pick();
            stream::construct_body_streaming(
                move |sink| {
                    Box::pin(
                        async move { exec::construct_each_sqlite_owned(&plan, conn, sink).await },
                    )
                },
                fmt,
                Some(deadline.into_std()),
            )
        }
        Backend::Pg(pool) => {
            let conn = match acquire_pg(&pool, deadline).await {
                Ok(c) => c,
                Err(resp) => return resp,
            };
            stream::construct_body_streaming(
                move |sink| {
                    Box::pin(async move { exec_pg::construct_each_pg(&plan, conn, sink).await })
                },
                fmt,
                Some(deadline.into_std()),
            )
        }
        Backend::Mysql(pool) => stream::construct_body_streaming(
            move |sink| {
                Box::pin(async move {
                    let conn = pool
                        .get_conn()
                        .await
                        .map_err(|e| SparqlError::Sql(e.to_string()))?;
                    exec_mysql::construct_each_mysql(&plan, conn, sink).await
                })
            },
            fmt,
            Some(deadline.into_std()),
        ),
    };
    ok_stream(fmt.media_type(), body)
}

/// Acquire a pooled PostgreSQL connection (ADR-0010 §C stream-lane pool, ADR-0027;
/// M4 wave-2 finding 2). Pool exhaustion (no free connection within the
/// configured `--pg-pool-wait-secs`) is shed as a fast, honest `503` +
/// `Retry-After` rather than queued indefinitely or reported as a generic `500`
/// — the ADR-0010 "shed overflow" clause this pass implements.
async fn acquire_pg(
    pool: &deadpool_postgres::Pool,
    deadline: RequestDeadline,
) -> Result<PgConn, Response> {
    let acquired = match deadline.run(pool.get()).await {
        Ok(result) => result,
        Err(_) => return Err(timeout_response()),
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
    match deadline.run(PgConn::checked(conn)).await {
        Err(_) => Err(timeout_response()),
        Ok(Err(_)) => Err(problem::response(ProblemCode::Internal)),
        Ok(Ok(conn)) => Ok(conn),
    }
}

/// The first value of form key `key` in a urlencoded string (`+`/`%XX` decoded).
fn form_param(encoded: &str, key: &str) -> Option<String> {
    form_urlencoded::parse(encoded.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

fn timeout_response() -> Response {
    problem::response(ProblemCode::RequestTimeout)
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
