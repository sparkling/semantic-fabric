//! The blocking entry point the CLI calls (`semantic-fabric serve`): resolve the
//! source spec, open the backend, parse the mapping `M` and optional ontology `T`,
//! introspect the source schema, bind, and serve until shutdown.

use std::sync::Arc;
use std::time::Duration;

use tokio_postgres::NoTls;

use crate::{introspect_pg_all, router, Backend, ServeConfig};

/// Options resolved from the `serve` CLI flags. The runner reads the mapping /
/// ontology files itself so the CLI stays a thin argument parser.
pub struct ServeOptions {
    /// `sqlite:<path>` (path may be `:memory:`) or `pg:<conninfo>`.
    pub source: String,
    /// Path to the R2RML mapping document (Turtle).
    pub mapping_path: String,
    /// Optional ontology (Turtle) → tier-1 T-Box.
    pub ontology_path: Option<String>,
    /// `host:port` to bind (e.g. `127.0.0.1:7878`).
    pub bind: String,
    /// Request timeout (ADR-0010).
    pub timeout: Duration,
    /// Max query length in bytes (ADR-0010).
    pub max_query_len: usize,
    /// Max PostgreSQL pool connections (ADR-0010 §C stream-lane pool, ADR-0027).
    pub pg_pool_size: usize,
    /// Max wait for a pooled PostgreSQL connection before shedding `503` (ADR-0010 §C).
    pub pg_pool_wait: Duration,
    /// Read-only connection pool size for a file-backed SQLite source (ADR-0010
    /// status-correction part 2).
    pub sqlite_pool_size: usize,
}

/// Build the config + router and serve until the process is stopped. Returns a
/// clear error (never panics) when a required input is missing or invalid.
pub fn serve_blocking(opts: ServeOptions) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime: {e}"))?;
    rt.block_on(async move { serve_async(opts).await })
}

async fn serve_async(opts: ServeOptions) -> Result<(), String> {
    let mapping_ttl = std::fs::read_to_string(&opts.mapping_path)
        .map_err(|e| format!("read mapping {}: {e}", opts.mapping_path))?;
    let mapping = sf_mapping::parse_r2rml(&mapping_ttl).map_err(|e| format!("parse R2RML: {e}"))?;

    let tbox = match &opts.ontology_path {
        Some(path) => {
            let ttl =
                std::fs::read_to_string(path).map_err(|e| format!("read ontology {path}: {e}"))?;
            crate::tbox_from_turtle(&ttl)?
        }
        None => sf_sparql::Tbox::default(),
    };

    let (backend, schema) = open_backend(
        &opts.source,
        opts.pg_pool_size,
        opts.pg_pool_wait,
        opts.sqlite_pool_size,
    )
    .await?;

    let mut cfg = ServeConfig::new(backend, mapping, tbox, schema);
    cfg.timeout = opts.timeout;
    cfg.max_query_len = opts.max_query_len;

    let app = router(Arc::new(cfg));
    let listener = tokio::net::TcpListener::bind(&opts.bind)
        .await
        .map_err(|e| format!("bind {}: {e}", opts.bind))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    println!("semantic-fabric: SPARQL 1.2 endpoint listening on http://{addr}/sparql");
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("server error: {e}"))
}

/// Open the backend named by `spec` and introspect its base-table schema.
/// `pg_pool_size`/`pg_pool_wait` size the PostgreSQL pool (ADR-0010 §C
/// stream-lane pool, ADR-0027); `sqlite_pool_size` sizes the read-only pool for
/// a file-backed SQLite source ([`Backend::sqlite_pool_from_path`]).
async fn open_backend(
    spec: &str,
    pg_pool_size: usize,
    pg_pool_wait: Duration,
    sqlite_pool_size: usize,
) -> Result<(Backend, Vec<sf_sql::TableSchema>), String> {
    if let Some(path) = spec.strip_prefix("sqlite:") {
        Backend::sqlite_pool_from_path(path, sqlite_pool_size)
    } else if let Some(conninfo) = spec.strip_prefix("pg:") {
        // A bounded pool (ADR-0010 §C stream-lane pool, ADR-0027; M4 wave-2 finding
        // 2), not a single shared client — mirrors MySQL's `mysql_async::Pool`.
        let pg_config: tokio_postgres::Config = conninfo
            .parse()
            .map_err(|e| format!("parse PG conninfo {conninfo:?}: {e}"))?;
        let manager = deadpool_postgres::Manager::new(pg_config, NoTls);
        let pool = deadpool_postgres::Pool::builder(manager)
            .max_size(pg_pool_size)
            .wait_timeout(Some(pg_pool_wait))
            // The wait timeout needs an async runtime to enforce it (deadpool is
            // runtime-agnostic by default) — without this, `pool.get()` errors
            // `NoRuntimeSpecified` instead of ever honouring the timeout.
            .runtime(deadpool_postgres::Runtime::Tokio1)
            .build()
            .map_err(|e| format!("build PostgreSQL pool: {e}"))?;
        let conn = pool
            .get()
            .await
            .map_err(|e| format!("PostgreSQL pool get (introspection): {e}"))?;
        let schema = introspect_pg_all(&conn).await?;
        drop(conn);
        Ok((Backend::Pg(pool), schema))
    } else if spec.starts_with("mysql://") {
        // `Pool::from_url` needs the whole `mysql://…` URL — never strip the scheme.
        let pool = mysql_async::Pool::from_url(spec).map_err(|e| format!("connect MySQL: {e}"))?;
        let mut conn = pool
            .get_conn()
            .await
            .map_err(|e| format!("MySQL get_conn: {e}"))?;
        let schema = introspect_mysql_all(&mut conn).await?;
        drop(conn);
        Ok((Backend::Mysql(pool), schema))
    } else {
        Err(format!(
            "unrecognised --source {spec:?}: expected sqlite:<path>, pg:<conninfo>, or mysql://<url>"
        ))
    }
}

/// Introspect every MySQL base table in the current database (name order) — the
/// MySQL analogue of [`introspect_pg_all`].
async fn introspect_mysql_all(
    conn: &mut mysql_async::Conn,
) -> Result<Vec<sf_sql::TableSchema>, String> {
    use mysql_async::prelude::Queryable;
    let names: Vec<String> = conn
        .query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE' ORDER BY table_name",
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut schemas = Vec::with_capacity(names.len());
    for name in names {
        schemas.push(
            sf_sql::introspect::introspect_mysql(conn, &name)
                .await
                .map_err(|e| e.to_string())?,
        );
    }
    Ok(schemas)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique path under the OS temp dir — avoids clashing with other tests
    /// or a stale file from a previous run.
    fn temp_db_path(tag: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "sf_serve_open_backend_{tag}_{}_{unique}.db",
            std::process::id()
        ))
    }

    /// Create a fresh SQLite file at `path` with one `widgets(id, name)` table
    /// and a single row, then close the connection so `open_backend` can reopen it.
    fn seed_sqlite_db(path: &std::path::Path) {
        let conn = rusqlite::Connection::open(path).expect("create temp sqlite db");
        conn.execute_batch(
            "CREATE TABLE widgets (id INTEGER PRIMARY KEY, name TEXT NOT NULL); \
             INSERT INTO widgets (id, name) VALUES (1, 'sprocket');",
        )
        .expect("seed widgets table");
    }

    #[tokio::test]
    async fn should_open_backend_when_spec_is_a_valid_sqlite_path() {
        let path = temp_db_path("valid");
        seed_sqlite_db(&path);
        let spec = format!("sqlite:{}", path.display());

        let result = open_backend(&spec, 16, Duration::from_secs(5), 4).await;

        let (backend, schema) = result.expect("valid sqlite spec should open");
        assert!(matches!(backend, Backend::Sqlite(_)));
        assert!(
            !schema.is_empty(),
            "expected at least one introspected table"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn should_introspect_known_table_when_sqlite_db_has_a_table() {
        let path = temp_db_path("introspect");
        seed_sqlite_db(&path);
        let spec = format!("sqlite:{}", path.display());

        let (_backend, schema) = open_backend(&spec, 16, Duration::from_secs(5), 4)
            .await
            .expect("valid sqlite spec should open");

        let widgets = schema
            .iter()
            .find(|t| t.name == "widgets")
            .expect("widgets table should be introspected");
        assert!(
            widgets.columns.iter().any(|c| c.name == "id"),
            "widgets schema should include the id column"
        );
        assert!(
            widgets.columns.iter().any(|c| c.name == "name"),
            "widgets schema should include the name column"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn should_error_when_spec_scheme_is_unsupported() {
        let result = open_backend("redis://localhost:6379", 16, Duration::from_secs(5), 4).await;

        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("unrecognised scheme should error, not open"),
        };
        assert!(
            err.contains("unrecognised --source"),
            "error should name the problem, got: {err}"
        );
    }

    #[tokio::test]
    async fn should_error_not_panic_when_sqlite_path_is_malformed() {
        // A path whose parent directory does not exist: rusqlite can neither
        // find nor create the file, so `Connection::open` errors.
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let bogus_dir = std::env::temp_dir().join(format!("sf_serve_no_such_dir_{unique}"));
        let path = bogus_dir.join("db.sqlite");
        let spec = format!("sqlite:{}", path.display());

        let result = open_backend(&spec, 16, Duration::from_secs(5), 4).await;

        assert!(
            result.is_err(),
            "opening a sqlite path under a nonexistent directory should error"
        );
    }

    /// F2b flag-plumbing receipt: `--pg-pool-size` must actually reach the pool
    /// `open_backend` builds, not just get parsed and dropped. Deterministic
    /// (no concurrency/timing) — asserts the built pool's own reported
    /// `max_size` rather than exercising pool exhaustion, which
    /// `pg_pool_exhaustion_sheds_503_with_retry_after` and
    /// `pg_pool_concurrency_receipt` (`crates/sf-serve/tests/endpoint.rs`)
    /// already cover. Gate-skips when no PostgreSQL is reachable on
    /// localhost:5432, mirroring that file's `pg` module convention.
    #[tokio::test]
    async fn should_configure_pg_pool_size_when_opening_a_pg_backend() {
        let conn_str = std::env::var("SF_PG_URL").unwrap_or_else(|_| {
            let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned());
            format!("host=localhost port=5432 user={user}")
        });
        let Ok((_client, connection)) = tokio_postgres::connect(&conn_str, NoTls).await else {
            eprintln!(
                "SKIP should_configure_pg_pool_size_when_opening_a_pg_backend: \
                 no PostgreSQL on localhost:5432"
            );
            return;
        };
        tokio::spawn(async move {
            let _ = connection.await;
        });

        let spec = format!("pg:{conn_str}");
        let (backend, _schema) = open_backend(&spec, 3, Duration::from_secs(2), 4)
            .await
            .expect("reachable pg spec should open");

        let Backend::Pg(pool) = backend else {
            panic!("pg: spec should open a Backend::Pg");
        };
        assert_eq!(
            pool.status().max_size,
            3,
            "pg_pool_size passed to open_backend should flow through to the pool's max_size"
        );
    }
}
