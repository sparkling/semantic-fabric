//! The blocking entry point the CLI calls (`semantic-fabric serve`): resolve the
//! source spec, open the backend, parse the mapping `M` and optional ontology `T`,
//! introspect the source schema, bind, and serve until shutdown.

use std::sync::Arc;
use std::time::Duration;

use tokio_postgres::NoTls;

use crate::problem::StartupCause;
use crate::source::PreparedSource;
use crate::{
    introspect_pg_all, router, Backend, IntrospectedSource, ServeConfig, ServeError, SourceRef,
};

/// Options resolved from the `serve` CLI flags. The runner reads the mapping /
/// ontology files itself so the CLI stays a thin argument parser.
pub struct ServeOptions {
    /// Credential-free inline source or environment-injected source reference.
    pub source: SourceRef,
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
pub fn serve_blocking(opts: ServeOptions) -> Result<(), ServeError> {
    // Resolve, bound, parse, and reject inline credentials before runtime, file,
    // DNS, socket, or connector construction.
    let source = opts.source.resolve()?.prepare()?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            ServeError::new(StartupCause::Runtime {
                error: error.to_string(),
            })
        })?;
    rt.block_on(async move { serve_async(opts, source).await })
}

async fn serve_async(opts: ServeOptions, source: PreparedSource) -> Result<(), ServeError> {
    let mapping_ttl = std::fs::read_to_string(&opts.mapping_path).map_err(|error| {
        ServeError::new(StartupCause::MappingRead {
            path: opts.mapping_path.clone(),
            error: error.to_string(),
        })
    })?;
    let source_id = sf_core::SourceId::new(0).expect("single-source slot zero is representable");
    let mapping = sf_mapping::parse_r2rml_for_source(&mapping_ttl, source_id).map_err(|error| {
        ServeError::new(StartupCause::MappingParse {
            error: error.to_string(),
        })
    })?;

    let tbox = match &opts.ontology_path {
        Some(path) => {
            let ttl = std::fs::read_to_string(path).map_err(|error| {
                ServeError::new(StartupCause::OntologyRead {
                    path: path.clone(),
                    error: error.to_string(),
                })
            })?;
            crate::tbox_from_turtle(&ttl)
                .map_err(|error| ServeError::new(StartupCause::OntologyParse { error }))?
        }
        None => sf_sparql::Tbox::default(),
    };

    let source = open_backend(
        source,
        opts.pg_pool_size,
        opts.pg_pool_wait,
        opts.sqlite_pool_size,
    )
    .await?;

    let mut cfg = ServeConfig::new(source, mapping, tbox);
    cfg.timeout = opts.timeout;
    cfg.max_query_len = opts.max_query_len;

    let app = router(Arc::new(cfg));
    let listener = tokio::net::TcpListener::bind(&opts.bind)
        .await
        .map_err(|error| {
            ServeError::new(StartupCause::Bind {
                bind: opts.bind.clone(),
                error: error.to_string(),
            })
        })?;
    let addr = listener.local_addr().map_err(|error| {
        ServeError::new(StartupCause::Server {
            error: error.to_string(),
        })
    })?;
    println!("semantic-fabric: SPARQL 1.2 endpoint listening on http://{addr}/sparql");
    axum::serve(listener, app).await.map_err(|error| {
        ServeError::new(StartupCause::Server {
            error: error.to_string(),
        })
    })
}

/// Open the prepared backend and pair it with the base-table schema observed
/// through that handle. This pairing is not yet a coherent/live snapshot claim.
/// `pg_pool_size`/`pg_pool_wait` size the PostgreSQL pool (ADR-0010 §C
/// stream-lane pool, ADR-0027); `sqlite_pool_size` sizes the read-only pool for
/// a file-backed SQLite source ([`Backend::sqlite_pool_from_path`]).
async fn open_backend(
    source: PreparedSource,
    pg_pool_size: usize,
    pg_pool_wait: Duration,
    sqlite_pool_size: usize,
) -> Result<IntrospectedSource, ServeError> {
    match source {
        PreparedSource::Sqlite { path, label } => {
            Backend::sqlite_pool_from_path(&path, sqlite_pool_size)
                .map(|(backend, schema)| IntrospectedSource::observed(backend, schema))
                .map_err(|error| {
                    ServeError::new(StartupCause::SourceConnect {
                        spec: label.to_owned(),
                        error,
                    })
                })
        }
        PreparedSource::Postgres { config, label } => {
            // A bounded pool (ADR-0010 §C stream-lane pool, ADR-0027; M4 wave-2 finding
            // 2), not a single shared client — mirrors MySQL's `mysql_async::Pool`.
            let manager = deadpool_postgres::Manager::new(*config, NoTls);
            let pool = deadpool_postgres::Pool::builder(manager)
                .max_size(pg_pool_size)
                .wait_timeout(Some(pg_pool_wait))
                // The wait timeout needs an async runtime to enforce it (deadpool is
                // runtime-agnostic by default) — without this, `pool.get()` errors
                // `NoRuntimeSpecified` instead of ever honouring the timeout.
                .runtime(deadpool_postgres::Runtime::Tokio1)
                .build()
                .map_err(|error| {
                    ServeError::new(StartupCause::SourceConnect {
                        spec: label.to_owned(),
                        error: error.to_string(),
                    })
                })?;
            let conn = pool.get().await.map_err(|error| {
                ServeError::new(StartupCause::SourceConnect {
                    spec: label.to_owned(),
                    error: error.to_string(),
                })
            })?;
            let schema = introspect_pg_all(&conn).await.map_err(|error| {
                ServeError::new(StartupCause::Schema {
                    spec: label.to_owned(),
                    error,
                })
            })?;
            drop(conn);
            Ok(IntrospectedSource::observed(Backend::Pg(pool), schema))
        }
        PreparedSource::Mysql { options, label } => {
            let pool = mysql_async::Pool::new(options);
            let mut conn = pool.get_conn().await.map_err(|error| {
                ServeError::new(StartupCause::SourceConnect {
                    spec: label.to_owned(),
                    error: error.to_string(),
                })
            })?;
            let schema = introspect_mysql_all(&mut conn).await.map_err(|error| {
                ServeError::new(StartupCause::Schema {
                    spec: label.to_owned(),
                    error,
                })
            })?;
            drop(conn);
            Ok(IntrospectedSource::observed(Backend::Mysql(pool), schema))
        }
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

    fn prepare_inline(spec: impl Into<String>) -> Result<PreparedSource, ServeError> {
        SourceRef::inline(spec).resolve()?.prepare()
    }

    fn prepare_injected(spec: String) -> Result<PreparedSource, ServeError> {
        crate::SourceInput::injected(spec)?.prepare()
    }

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

        let source = prepare_inline(spec).expect("valid SQLite source");
        let result = open_backend(source, 16, Duration::from_secs(5), 4).await;

        let (backend, schema) = result.expect("valid sqlite spec should open").into_parts();
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

        let source = prepare_inline(spec).expect("valid SQLite source");
        let (_backend, schema) = open_backend(source, 16, Duration::from_secs(5), 4)
            .await
            .expect("valid sqlite spec should open")
            .into_parts();

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

    #[test]
    fn should_reject_unadmitted_source_families_before_connector_construction() {
        let rejected = [
            ("Trino", "trino://query.invalid"),
            ("Presto", "presto://query.invalid"),
            ("PrestoDB", "prestodb://query.invalid"),
            ("AWS Athena", "athena://query.invalid"),
            ("Snowflake", "snowflake://account.invalid"),
            ("BigQuery", "bigquery://project.invalid"),
            ("Databricks", "databricks://workspace.invalid"),
            ("DuckDB", "duckdb:/tmp/source.duckdb"),
            ("SAP HANA", "hana://database.invalid"),
            ("MonetDB", "monetdb://database.invalid"),
            ("ODBC", "odbc:Driver=Unadmitted"),
            ("Oracle", "oracle://database.invalid"),
            ("Redshift", "redshift://cluster.invalid"),
            ("SQL Server", "sqlserver://database.invalid"),
            ("SQL Server alias", "mssql://database.invalid"),
            ("Redis", "redis://cache.invalid"),
            ("generic REST", "rest:https://api.invalid/query"),
            ("generic HTTP", "http://api.invalid/query"),
            ("generic HTTPS", "https://api.invalid/query"),
        ];

        for (family, spec) in rejected {
            let err = match prepare_inline(spec) {
                Err(err) => err,
                Ok(_) => panic!("{family} source {spec:?} must not be admitted"),
            };
            assert_eq!(err.code(), "startup-source");
            let StartupCause::SourceSpec { spec: retained, .. } = err.internal_cause() else {
                panic!("{family} should retain a typed source-spec cause")
            };
            assert_eq!(retained, "<inline-source>");
            assert!(!err.to_string().contains(spec));
            assert!(!format!("{err:?}").contains(spec));
        }
    }

    #[test]
    fn inline_pg_and_mysql_credentials_are_not_retained_by_errors() {
        const SENTINEL: &str = "sf_secret_NEVER_EXPOSE_internal_34aa";
        let specs = [
            format!("pg:host=database.invalid password={SENTINEL}"),
            format!("mysql://user:{SENTINEL}@database.invalid/db"),
        ];

        for spec in specs {
            let error = match prepare_inline(&spec) {
                Err(error) => error,
                Ok(_) => panic!("inline credentials must fail"),
            };
            assert_eq!(error.code(), "startup-source");
            let StartupCause::SourceSpec { spec: retained, .. } = error.internal_cause() else {
                panic!("credential rejection should retain a typed source-spec cause")
            };
            assert_eq!(retained, "<inline-source>");
            assert!(!error.internal_cause().to_string().contains(SENTINEL));
            assert!(!error.to_string().contains(SENTINEL));
            assert!(!format!("{error:?}").contains(SENTINEL));
        }
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

        let source = prepare_inline(spec).expect("valid SQLite source");
        let result = open_backend(source, 16, Duration::from_secs(5), 4).await;

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

        let source = prepare_injected(format!("pg:{conn_str}"))
            .expect("environment-injected pg source should prepare");
        let (backend, _schema) = open_backend(source, 3, Duration::from_secs(2), 4)
            .await
            .expect("reachable pg spec should open")
            .into_parts();

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
