//! The blocking entry point the CLI calls (`semantic-fabric serve`): resolve the
//! source spec, open the backend, parse the mapping `M` and optional ontology `T`,
//! introspect the source schema, bind, and serve until shutdown.

use std::sync::Arc;
use std::time::Duration;

use tokio_postgres::NoTls;

use crate::{introspect_pg_all, introspect_sqlite_all, router, Backend, ServeConfig};

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

    let (backend, schema) = open_backend(&opts.source).await?;

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
async fn open_backend(spec: &str) -> Result<(Backend, Vec<sf_sql::TableSchema>), String> {
    if let Some(path) = spec.strip_prefix("sqlite:") {
        let conn =
            rusqlite::Connection::open(path).map_err(|e| format!("open SQLite {path}: {e}"))?;
        let schema = introspect_sqlite_all(&conn)?;
        Ok((Backend::sqlite(conn), schema))
    } else if let Some(conninfo) = spec.strip_prefix("pg:") {
        let (client, connection) = tokio_postgres::connect(conninfo, NoTls)
            .await
            .map_err(|e| format!("connect PostgreSQL: {e}"))?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let schema = introspect_pg_all(&client).await?;
        Ok((Backend::Pg(Arc::new(client)), schema))
    } else if spec.starts_with("mysql://") {
        // `Pool::from_url` needs the whole `mysql://…` URL — never strip the scheme.
        let pool =
            mysql_async::Pool::from_url(spec).map_err(|e| format!("connect MySQL: {e}"))?;
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
