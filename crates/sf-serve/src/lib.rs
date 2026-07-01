//! `sf-serve` — the SPARQL 1.2 **Protocol** HTTP endpoint over the OBDA
//! virtualiser (ADR-0019 G8: own the 1.2 query endpoint — Oxigraph ships only a
//! 1.1 server binary). Read-only (query operation only; no update).
//!
//! Per request: extract the query (GET `?query=`, POST form `query=`, or a raw
//! `application/sparql-query` body) → [`parse_and_translate_with`] against the
//! configured mapping `M` + T-Box `T` + dialect (the rewriter, off the async
//! runtime via `spawn_blocking`, ADR-0006) → execute over the configured backend →
//! serialise the negotiated form, **streaming** the bytes into the response body
//! (ADR-0010 §C; [`stream`]). Values stay bound parameters end to end — the
//! rewriter/executors never interpolate (ADR-0010 R1).
//!
//! Governance (ADR-0010): a configurable request timeout (deadline-checked in the
//! streaming writer + a `tokio::time::timeout` around collecting execution), a
//! max-query-length cap, and cancel-on-client-drop. Error → status mapping:
//! parse → 400, unsupported feature → 501, execution → 500, success → 200.

pub mod ontology;
pub mod run;
pub mod stream;

pub use run::{serve_blocking, ServeOptions};

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;

use sf_core::ir::TriplesMap;
use sf_sparql::{
    exec, exec_mysql, exec_pg, Epoch, Error as SparqlError, Plan, PlanCache, PlanForm, Tbox,
};
use sf_sql::introspect::{introspect_postgres, introspect_sqlite};
use sf_sql::{Dialect, TableSchema};
use sparesults::QueryResultsFormat;

/// Plan-cache capacity (ADR-0007 *Plan cache, hot path*). 64 entries covers a
/// diverse serve-mode workload without over-committing memory; the cache is sized
/// by `⟨T, M⟩` (never by data), so it cannot go stale vs a live source.
const PLAN_CACHE_CAP: usize = 64;

pub use ontology::tbox_from_turtle;
pub use stream::RdfFormat;

/// Default request timeout and max query length when constructed via [`ServeConfig::new`].
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_QUERY_LEN: usize = 1 << 20; // 1 MiB

/// A live relational backend the endpoint queries (ADR-0006). SQLite is sync
/// (`rusqlite::Connection`, not `Send` across awaits — held behind a `Mutex` and
/// driven inside `spawn_blocking`); PostgreSQL is async (a shared `tokio_postgres`
/// client handle).
#[derive(Clone)]
pub enum Backend {
    Sqlite(Arc<Mutex<rusqlite::Connection>>),
    Pg(Arc<tokio_postgres::Client>),
    /// MySQL: a cloneable `mysql_async::Pool`; each streaming request draws a
    /// DEDICATED connection for the stream's lifetime, discarded/reset on early drop
    /// (ADR-0024 §4.2 — mirrors PG cancel-on-drop).
    Mysql(mysql_async::Pool),
}

impl Backend {
    /// Wrap an open SQLite connection as a backend.
    pub fn sqlite(conn: rusqlite::Connection) -> Self {
        Backend::Sqlite(Arc::new(Mutex::new(conn)))
    }

    /// The SQL dialect this backend speaks (drives emission/introspection).
    pub fn dialect(&self) -> Dialect {
        match self {
            Backend::Sqlite(_) => Dialect::Sqlite,
            Backend::Pg(_) => Dialect::Postgres,
            Backend::Mysql(_) => Dialect::MySql,
        }
    }
}

/// The immutable server configuration shared (in an `Arc`) across all requests:
/// the parsed mapping `M`, the tier-1 T-Box `T`, the introspected source schema,
/// the backend, the ADR-0010 governance knobs, and the plan-compile cache.
pub struct ServeConfig {
    pub mapping: Vec<TriplesMap>,
    pub tbox: Tbox,
    pub schema: Vec<TableSchema>,
    pub backend: Backend,
    pub timeout: Duration,
    pub max_query_len: usize,
    /// Compiled-plan cache (ADR-0007): repeated queries at the same `⟨T, M⟩` +
    /// schema epoch reuse their plan without recompilation.
    plan_cache: PlanCache<Plan>,
    /// Monotonic epoch invalidated by ontology/mapping/schema reloads.
    epoch: Epoch,
}

impl ServeConfig {
    /// Build a config with the default governance knobs (30 s timeout, 1 MiB cap).
    pub fn new(
        backend: Backend,
        mapping: Vec<TriplesMap>,
        tbox: Tbox,
        schema: Vec<TableSchema>,
    ) -> Self {
        Self {
            mapping,
            tbox,
            schema,
            backend,
            timeout: DEFAULT_TIMEOUT,
            max_query_len: DEFAULT_MAX_QUERY_LEN,
            plan_cache: PlanCache::new(PLAN_CACHE_CAP),
            epoch: Epoch::default(),
        }
    }
}

/// Build the axum router exposing `GET`/`POST /sparql` over `cfg`.
pub fn router(cfg: Arc<ServeConfig>) -> Router {
    Router::new()
        .route("/sparql", get(handle_get).post(handle_post))
        .with_state(cfg)
}

/// `GET /sparql?query=...` (SPARQL 1.2 Protocol query via URL parameters).
async fn handle_get(
    State(cfg): State<Arc<ServeConfig>>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    let Some(query) = raw.as_deref().and_then(|q| form_param(q, "query")) else {
        return err_text(StatusCode::BAD_REQUEST, "missing 'query' parameter");
    };
    process(cfg, query, accept(&headers)).await
}

/// `POST /sparql` — either `application/x-www-form-urlencoded` (`query=...`) or a
/// raw `application/sparql-query` body (SPARQL 1.2 Protocol §2.1.2).
async fn handle_post(
    State(cfg): State<Arc<ServeConfig>>,
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
            Err(_) => return err_text(StatusCode::BAD_REQUEST, "query body is not valid UTF-8"),
        },
        "application/x-www-form-urlencoded" => {
            match std::str::from_utf8(&body)
                .ok()
                .and_then(|s| form_param(s, "query"))
            {
                Some(q) => q,
                None => return err_text(StatusCode::BAD_REQUEST, "missing 'query' parameter"),
            }
        }
        other => {
            return err_text(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                format!("unsupported Content-Type: {other:?}"),
            )
        }
    };
    process(cfg, query, accept(&headers)).await
}

/// The shared request pipeline: cap → compile → dispatch by query form → stream.
async fn process(cfg: Arc<ServeConfig>, query: String, accept: Option<String>) -> Response {
    if query.len() > cfg.max_query_len {
        return err_text(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "query exceeds the {}-byte cap (ADR-0010)",
                cfg.max_query_len
            ),
        );
    }

    let plan = match compile(cfg.clone(), query).await {
        Ok(p) => p,
        Err(resp) => return resp,
    };
    let accept = accept.as_deref();

    match &plan.form {
        PlanForm::Select { .. } => respond_select(cfg, plan, accept).await,
        PlanForm::Ask => respond_ask(cfg, plan, accept).await,
        PlanForm::Construct { .. } => respond_construct(cfg, plan, accept).await,
    }
}

/// Compile (parse + rewrite) off the async runtime (ADR-0006); map errors to status.
/// Uses the per-config plan cache (ADR-0007): repeated queries at the same epoch
/// skip the full rewrite and return a cached plan clone.
async fn compile(cfg: Arc<ServeConfig>, query: String) -> Result<Plan, Response> {
    let dialect = cfg.backend.dialect();
    let joined = tokio::task::spawn_blocking(move || {
        sf_sparql::parse_and_translate_cached(
            &query,
            &cfg.mapping,
            dialect,
            &cfg.tbox,
            &cfg.schema,
            &cfg.plan_cache,
            cfg.epoch,
        )
    })
    .await;
    match joined {
        Err(e) => Err(err_text(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("compile task join error: {e}"),
        )),
        Ok(Err(e)) => Err(err_text(status_for(&e), e.to_string())),
        Ok(Ok(plan)) => Ok(plan),
    }
}

/// Stream a SELECT (ADR-0010 §C). The status line is committed once streaming
/// begins, so the recoverable errors (parse → 400, unsupported → 501) are already
/// resolved by [`compile`]; an execution failure or a passed deadline aborts the
/// body mid-stream (same posture as the SQLite CONSTRUCT path).
async fn respond_select(cfg: Arc<ServeConfig>, plan: Plan, accept: Option<&str>) -> Response {
    let fmt = negotiate_results(accept);
    let PlanForm::Select { vars } = &plan.form else {
        return err_text(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal: non-SELECT plan reached respond_select",
        );
    };
    let vars = vars.clone();
    let deadline = Some(Instant::now() + cfg.timeout);
    let body = match cfg.backend.clone() {
        Backend::Sqlite(conn) => stream::select_body_sqlite(conn, plan, fmt, vars, deadline),
        Backend::Pg(client) => stream::select_body_pg(client, plan, fmt, vars, deadline),
        Backend::Mysql(pool) => stream::select_body_mysql(pool, plan, fmt, vars, deadline),
    };
    ok_stream(fmt.media_type(), body)
}

async fn respond_ask(cfg: Arc<ServeConfig>, plan: Plan, accept: Option<&str>) -> Response {
    let fmt = negotiate_results(accept);
    let value = match cfg.backend.clone() {
        Backend::Sqlite(conn) => {
            let run = tokio::task::spawn_blocking(move || {
                let conn = conn.lock().unwrap_or_else(|p| p.into_inner());
                exec::ask(&plan, &conn)
            });
            match tokio::time::timeout(cfg.timeout, run).await {
                Err(_) => {
                    return err_text(StatusCode::GATEWAY_TIMEOUT, "request timeout (ADR-0010)")
                }
                Ok(Err(e)) => {
                    return err_text(StatusCode::INTERNAL_SERVER_ERROR, format!("exec task: {e}"))
                }
                Ok(Ok(r)) => r,
            }
        }
        Backend::Pg(client) => {
            match tokio::time::timeout(cfg.timeout, exec_pg::ask_pg(&plan, client)).await {
                Err(_) => {
                    return err_text(StatusCode::GATEWAY_TIMEOUT, "request timeout (ADR-0010)")
                }
                Ok(r) => r,
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
            // the SQLite `spawn_blocking` arm's `Ok(Err)/Ok(Ok)` join handling.
            let run = tokio::spawn(async move {
                let conn = pool
                    .get_conn()
                    .await
                    .map_err(|e| SparqlError::Sql(e.to_string()))?;
                exec_mysql::ask_each_mysql(&plan, conn).await
            });
            match tokio::time::timeout(cfg.timeout, run).await {
                Err(_) => {
                    return err_text(StatusCode::GATEWAY_TIMEOUT, "request timeout (ADR-0010)")
                }
                Ok(Err(e)) => {
                    return err_text(StatusCode::INTERNAL_SERVER_ERROR, format!("exec task: {e}"))
                }
                Ok(Ok(r)) => r,
            }
        }
    };
    match value {
        Ok(b) => match stream::serialize_boolean(b, fmt) {
            Ok(bytes) => ok_stream(fmt.media_type(), stream::collected_body(bytes)),
            Err(e) => err_text(StatusCode::INTERNAL_SERVER_ERROR, e),
        },
        Err(e) => err_text(status_for(&e), e.to_string()),
    }
}

/// Stream a CONSTRUCT (ADR-0010 §C) — triples flow from the executor sink through
/// the RDF serialiser into the body, never collected, on **both** backends.
async fn respond_construct(cfg: Arc<ServeConfig>, plan: Plan, accept: Option<&str>) -> Response {
    let fmt = negotiate_rdf(accept);
    let deadline = Some(Instant::now() + cfg.timeout);
    let body = match cfg.backend.clone() {
        Backend::Sqlite(conn) => stream::construct_body_sqlite(conn, plan, fmt, deadline),
        Backend::Pg(client) => stream::construct_body_pg(client, plan, fmt, deadline),
        Backend::Mysql(pool) => stream::construct_body_mysql(pool, plan, fmt, deadline),
    };
    ok_stream(fmt.media_type(), body)
}

/// Map a rewriter error to an HTTP status (ADR-0010 §error handling).
fn status_for(err: &SparqlError) -> StatusCode {
    match err {
        SparqlError::Parse(_) => StatusCode::BAD_REQUEST,
        SparqlError::Unsupported(_) => StatusCode::NOT_IMPLEMENTED,
        SparqlError::Mapping(_) | SparqlError::Sql(_) | SparqlError::Core(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// The first value of form key `key` in a urlencoded string (`+`/`%XX` decoded).
fn form_param(encoded: &str, key: &str) -> Option<String> {
    form_urlencoded::parse(encoded.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
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

fn err_text(status: StatusCode, msg: impl Into<String>) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(msg.into()))
        .expect("static response builder")
}

/// Introspect every SQLite base table (schema order from `sqlite_master`), filling
/// the source schema that makes the ADR-0007 cascade passes fire.
pub fn introspect_sqlite_all(conn: &rusqlite::Connection) -> Result<Vec<TableSchema>, String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .map_err(|e| e.to_string())?;
    let names: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let mut schemas = Vec::with_capacity(names.len());
    for name in names {
        schemas.push(introspect_sqlite(conn, &name).map_err(|e| e.to_string())?);
    }
    Ok(schemas)
}

/// Introspect every PostgreSQL public base table.
pub async fn introspect_pg_all(
    client: &tokio_postgres::Client,
) -> Result<Vec<TableSchema>, String> {
    let rows = client
        .query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_type = 'BASE TABLE' ORDER BY table_name",
            &[],
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut schemas = Vec::with_capacity(rows.len());
    for r in rows {
        let name: String = r.get(0);
        schemas.push(
            introspect_postgres(client, &name)
                .await
                .map_err(|e| e.to_string())?,
        );
    }
    Ok(schemas)
}
