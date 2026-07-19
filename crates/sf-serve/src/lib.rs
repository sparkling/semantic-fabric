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
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use deadpool_postgres::PoolError;

use sf_core::ir::TriplesMap;
use sf_sparql::{
    exec, exec_mysql, exec_pg, Epoch, Error as SparqlError, Plan, PlanCache, PlanForm, Tbox,
};
use sf_sql::introspect::introspect_sqlite;
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

/// A pooled PostgreSQL connection, re-derefed to `tokio_postgres::Client` in one
/// hop. `deadpool_postgres::Object` derefs to its own `ClientWrapper` (adds
/// statement caching), not directly to `Client` — `sf_sparql::exec_pg`'s generic
/// client-handle bound (`Deref<Target = Client>`, shared with the conformance
/// harness's plain `Arc<Client>`) needs the single hop this newtype provides.
struct PgConn(deadpool_postgres::Object);

impl std::ops::Deref for PgConn {
    type Target = tokio_postgres::Client;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A small fixed pool of SQLite connections, dispatched round-robin. Serve is a
/// READ-ONLY endpoint (query operation only), and SQLite allows many concurrent
/// READERS against a file-backed database regardless of journal mode — so a
/// file-backed source gets `pool_size` independent read-only connections instead
/// of forcing every concurrent request through one shared connection (ADR-0010
/// status-correction part 2: "SQLite remains a single `Mutex<Connection>` by
/// choice ... an open refinement" — this closes that refinement). Each request
/// takes exactly one member's mutex for its query's duration, so up to
/// `pool_size` requests proceed concurrently instead of fully serialising.
///
/// `:memory:` sources stay a pool of one, read-write (see [`Backend::sqlite`] /
/// `run::open_backend`): each `rusqlite::Connection::open(":memory:")` call
/// creates an independent, private, empty database, so pooling `:memory:` the
/// normal way would silently serve queries against the wrong (empty) database.
#[derive(Clone)]
pub struct SqlitePool {
    conns: Arc<Vec<Arc<Mutex<rusqlite::Connection>>>>,
    next: Arc<std::sync::atomic::AtomicUsize>,
}

impl SqlitePool {
    /// A pool of one connection — the original single-`Mutex` shape, used for
    /// `:memory:` sources and any caller that already owns one open connection.
    fn one(conn: rusqlite::Connection) -> Self {
        Self::new(vec![conn])
    }

    fn new(conns: Vec<rusqlite::Connection>) -> Self {
        assert!(
            !conns.is_empty(),
            "SqlitePool needs at least one connection"
        );
        Self {
            conns: Arc::new(conns.into_iter().map(|c| Arc::new(Mutex::new(c))).collect()),
            next: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    /// Round-robin the next pool member (wraps around). A pool of one (every
    /// `:memory:` source, and any single-connection test fixture built via
    /// [`Backend::sqlite`]) always returns that same connection, so callers see
    /// identical behaviour to the pre-pool single-`Mutex` design.
    pub fn pick(&self) -> Arc<Mutex<rusqlite::Connection>> {
        let n = self.conns.len();
        let i = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % n;
        self.conns[i].clone()
    }
}

/// A live relational backend the endpoint queries (ADR-0006). SQLite is sync
/// (`rusqlite::Connection`, not `Send` across awaits — held behind a `Mutex`); its
/// blocking now lives entirely in the adapter's cap-1 `spawn_blocking` bridge
/// (ADR-0024 §4.1 [`SqliteOwnedBackend`]), so the serve lane drives all three
/// backends through the same async streamer. A file-backed SQLite source is a
/// small [`SqlitePool`] of read-only connections (SQLite read-concurrency,
/// ADR-0010 status-correction part 2), round-robin dispatched per request.
/// PostgreSQL is a bounded `deadpool_postgres::Pool` (ADR-0010 §C stream-lane
/// pool, ADR-0027; M4 wave-2 finding 2) — was a single shared `Client`
/// serialising every PG HTTP request; each request now draws a pooled connection
/// for its lifetime, mirroring MySQL's existing `mysql_async::Pool`.
#[derive(Clone)]
pub enum Backend {
    Sqlite(SqlitePool),
    Pg(deadpool_postgres::Pool),
    /// MySQL: a cloneable `mysql_async::Pool`; each streaming request draws a
    /// DEDICATED connection for the stream's lifetime, discarded/reset on early drop
    /// (ADR-0024 §4.2 — mirrors PG cancel-on-drop).
    Mysql(mysql_async::Pool),
}

impl Backend {
    /// Wrap a single open SQLite connection as a backend (a pool of one — the
    /// shape every `:memory:` source and most test fixtures want).
    pub fn sqlite(conn: rusqlite::Connection) -> Self {
        Backend::Sqlite(SqlitePool::one(conn))
    }

    /// Open `pool_size` independent READ-ONLY connections to the SQLite file at
    /// `path`, dispatched round-robin per request ([`SqlitePool`]). Never
    /// touches journal mode or any other persistent setting on the file.
    /// `:memory:` is special-cased to a single read-write connection regardless
    /// of `pool_size` ([`SqlitePool`] doc). Returns the introspected schema
    /// alongside the backend (introspected from one pool member — read-only
    /// queries against `sqlite_master`/`PRAGMA table_info`, safe to share). Public
    /// so callers (including tests) can build a multi-connection SQLite backend
    /// the same way `run::open_backend` does, without going through the blocking
    /// `serve` entry point — mirrors PG pools being fully caller-constructible via
    /// public `deadpool_postgres` APIs.
    pub fn sqlite_pool_from_path(
        path: &str,
        pool_size: usize,
    ) -> Result<(Self, Vec<TableSchema>), String> {
        if path == ":memory:" {
            let conn =
                rusqlite::Connection::open(path).map_err(|e| format!("open SQLite {path}: {e}"))?;
            let schema = introspect_sqlite_all(&conn)?;
            return Ok((Backend::sqlite(conn), schema));
        }
        let n = pool_size.max(1);
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_URI;
        let mut conns = Vec::with_capacity(n);
        for _ in 0..n {
            conns.push(
                rusqlite::Connection::open_with_flags(path, flags)
                    .map_err(|e| format!("open SQLite (read-only) {path}: {e}"))?,
            );
        }
        let schema = introspect_sqlite_all(&conns[0])?;
        Ok((Backend::Sqlite(SqlitePool::new(conns)), schema))
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
    // Uniform lane (ADR-0024 M5): every backend drives the generic streamer via a
    // thin concrete `exec_*::select_each_*` closure. SQLite's blocking now lives in
    // the adapter's cap-1 bridge (no sf-serve spawn_blocking special-case).
    let body = match cfg.backend.clone() {
        Backend::Sqlite(pool) => {
            let conn = pool.pick();
            stream::select_body_streaming(
                move |sink| {
                    Box::pin(async move { exec::select_each_sqlite_owned(&plan, conn, sink).await })
                },
                fmt,
                vars,
                deadline,
            )
        }
        Backend::Pg(pool) => {
            let conn = match acquire_pg(&pool).await {
                Ok(c) => c,
                Err(resp) => return resp,
            };
            stream::select_body_streaming(
                move |sink| {
                    Box::pin(async move { exec_pg::select_each_pg(&plan, conn, sink).await })
                },
                fmt,
                vars,
                deadline,
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
            deadline,
        ),
    };
    ok_stream(fmt.media_type(), body)
}

async fn respond_ask(cfg: Arc<ServeConfig>, plan: Plan, accept: Option<&str>) -> Response {
    let fmt = negotiate_results(accept);
    let value = match cfg.backend.clone() {
        Backend::Sqlite(pool) => {
            // Uniform lane (ADR-0024 M5): SQLite ASK spawns the owned cap-1-bridge
            // backend onto the runtime like the MySQL arm — no `spawn_blocking`
            // special-case; the adapter owns SQLite's blocking. `tokio::spawn` checks
            // `Send` on the concrete owned-backend future directly (provable).
            let conn = pool.pick();
            let run = tokio::spawn(async move { exec::ask_sqlite_owned(&plan, conn).await });
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
        Backend::Pg(pool) => {
            let conn = match acquire_pg(&pool).await {
                Ok(c) => c,
                Err(resp) => return resp,
            };
            match tokio::time::timeout(cfg.timeout, exec_pg::ask_pg(&plan, conn)).await {
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
            // the SQLite ASK arm's `tokio::spawn` + `Ok(Err)/Ok(Ok)` join handling.
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
        Backend::Sqlite(pool) => {
            let conn = pool.pick();
            stream::construct_body_streaming(
                move |sink| {
                    Box::pin(
                        async move { exec::construct_each_sqlite_owned(&plan, conn, sink).await },
                    )
                },
                fmt,
                deadline,
            )
        }
        Backend::Pg(pool) => {
            let conn = match acquire_pg(&pool).await {
                Ok(c) => c,
                Err(resp) => return resp,
            };
            stream::construct_body_streaming(
                move |sink| {
                    Box::pin(async move { exec_pg::construct_each_pg(&plan, conn, sink).await })
                },
                fmt,
                deadline,
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
            deadline,
        ),
    };
    ok_stream(fmt.media_type(), body)
}

/// Acquire a pooled PostgreSQL connection (ADR-0010 §C stream-lane pool, ADR-0027;
/// M4 wave-2 finding 2). Pool exhaustion (no free connection within the
/// configured `--pg-pool-wait-secs`) is shed as a fast, honest `503` +
/// `Retry-After` rather than queued indefinitely or reported as a generic `500`
/// — the ADR-0010 "shed overflow" clause this pass implements.
async fn acquire_pg(pool: &deadpool_postgres::Pool) -> Result<PgConn, Response> {
    pool.get().await.map(PgConn).map_err(|e| match e {
        PoolError::Timeout(_) => {
            let mut resp = err_text(
                StatusCode::SERVICE_UNAVAILABLE,
                "PostgreSQL connection pool exhausted, retry shortly (ADR-0010)",
            );
            resp.headers_mut().insert(
                header::RETRY_AFTER,
                // Fixed at 1s rather than derived from pool pressure/wait-time —
                // a pressure-aware value is future work (ADR-0010 status
                // correction part 2's second open refinement).
                HeaderValue::from_static("1"),
            );
            resp
        }
        other => err_text(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("PostgreSQL pool: {other}"),
        ),
    })
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

/// Introspect every PostgreSQL public base table in 5 set-based round trips
/// total, rather than 5 **per table** (M4 wave-2 finding 4 — the N+1 this
/// function used to drive via a per-table
/// [`introspect_postgres`](sf_sql::introspect::introspect_postgres) loop).
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
    let names: Vec<String> = rows.into_iter().map(|r| r.get(0)).collect();
    sf_sql::introspect::introspect_postgres_all(client, &names)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod pure_helper_tests {
    use super::*;

    // --- status_for ---------------------------------------------------------

    #[test]
    fn status_for_maps_every_error_variant_to_its_documented_status() {
        assert_eq!(
            status_for(&SparqlError::Parse("x".into())),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status_for(&SparqlError::Unsupported("x".into())),
            StatusCode::NOT_IMPLEMENTED
        );
        assert_eq!(
            status_for(&SparqlError::Mapping("x".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            status_for(&SparqlError::Sql("x".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            status_for(&SparqlError::Core("x".into())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    // --- form_param -----------------------------------------------------------

    #[test]
    fn form_param_extracts_the_named_key() {
        assert_eq!(
            form_param("query=SELECT+%2A", "query"),
            Some("SELECT *".to_owned())
        );
    }

    #[test]
    fn form_param_returns_none_when_key_absent() {
        assert_eq!(form_param("other=1", "query"), None);
    }

    #[test]
    fn form_param_returns_the_first_match_when_key_repeated() {
        assert_eq!(
            form_param("query=first&query=second", "query"),
            Some("first".to_owned())
        );
    }

    #[test]
    fn form_param_decodes_plus_and_percent_encoding() {
        assert_eq!(
            form_param("query=a+b%26c", "query"),
            Some("a b&c".to_owned())
        );
    }

    #[test]
    fn form_param_on_empty_string_is_none() {
        assert_eq!(form_param("", "query"), None);
    }

    // --- negotiate_results ------------------------------------------------

    #[test]
    fn negotiate_results_defaults_to_json_when_no_accept_header() {
        assert_eq!(negotiate_results(None), QueryResultsFormat::Json);
    }

    #[test]
    fn negotiate_results_defaults_to_json_for_an_unrecognised_accept() {
        assert_eq!(
            negotiate_results(Some("text/plain")),
            QueryResultsFormat::Json
        );
    }

    #[test]
    fn negotiate_results_picks_xml_for_sparql_results_xml() {
        assert_eq!(
            negotiate_results(Some("application/sparql-results+xml")),
            QueryResultsFormat::Xml
        );
    }

    #[test]
    fn negotiate_results_picks_xml_for_generic_application_xml() {
        assert_eq!(
            negotiate_results(Some("application/xml")),
            QueryResultsFormat::Xml
        );
    }

    #[test]
    fn negotiate_results_picks_tsv() {
        assert_eq!(
            negotiate_results(Some("text/tab-separated-values")),
            QueryResultsFormat::Tsv
        );
    }

    #[test]
    fn negotiate_results_picks_csv() {
        assert_eq!(negotiate_results(Some("text/csv")), QueryResultsFormat::Csv);
    }

    #[test]
    fn negotiate_results_is_case_insensitive() {
        assert_eq!(negotiate_results(Some("TEXT/CSV")), QueryResultsFormat::Csv);
    }

    #[test]
    fn negotiate_results_first_match_wins_on_a_multi_value_accept_header() {
        // XML is checked before TSV/CSV in negotiate_results's own ordering, so
        // a header offering both must resolve to XML, not whichever appears
        // first in the (unordered) Accept string.
        assert_eq!(
            negotiate_results(Some("text/csv, application/sparql-results+xml")),
            QueryResultsFormat::Xml
        );
    }

    // --- negotiate_rdf --------------------------------------------------------

    #[test]
    fn negotiate_rdf_defaults_to_turtle_when_no_accept_header() {
        assert_eq!(negotiate_rdf(None), RdfFormat::Turtle);
    }

    #[test]
    fn negotiate_rdf_defaults_to_turtle_for_an_unrecognised_accept() {
        assert_eq!(negotiate_rdf(Some("text/plain")), RdfFormat::Turtle);
    }

    #[test]
    fn negotiate_rdf_picks_json_ld() {
        assert_eq!(
            negotiate_rdf(Some("application/ld+json")),
            RdfFormat::JsonLd
        );
    }

    #[test]
    fn negotiate_rdf_picks_ntriples() {
        assert_eq!(
            negotiate_rdf(Some("application/n-triples")),
            RdfFormat::NTriples
        );
    }

    #[test]
    fn negotiate_rdf_is_case_insensitive() {
        assert_eq!(
            negotiate_rdf(Some("APPLICATION/N-TRIPLES")),
            RdfFormat::NTriples
        );
    }
}
