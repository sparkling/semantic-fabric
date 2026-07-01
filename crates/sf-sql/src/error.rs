//! Errors raised by the source/SQL layer (ADR-0006).

/// Failures in dialect emission, schema introspection, streaming, or planning.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// SQL emission/parse failed (`sqlparser`); e.g. a malformed `rr:sqlQuery`.
    #[error("sql emission error: {0}")]
    Emit(String),
    /// Schema introspection failed (catalog query / unexpected catalog shape).
    #[error("introspection error: {0}")]
    Introspection(String),
    /// Cross-source semi-join planning failed (degenerate cardinality inputs).
    #[error("planning error: {0}")]
    Planning(String),
    /// A SQLite source driver error (`rusqlite`).
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A PostgreSQL source driver error (`tokio-postgres`).
    #[error("postgres error: {0}")]
    Postgres(#[from] tokio_postgres::Error),
    /// A MySQL source driver error (`mysql_async`).
    #[error("mysql error: {0}")]
    Mysql(#[from] mysql_async::Error),
    /// A per-cell result-marshalling failure in a backend adapter (ADR-0024): a
    /// non-UTF-8 text column, or a BLOB in a non-`hexBinary` position. Surfaced as a
    /// HARD error via `BranchStream::next_row` (design A2), never a silent short read.
    #[error("value marshalling error: {0}")]
    Marshal(String),
    /// A result type a backend adapter's decoder does not cover (ADR-0024): e.g. a
    /// PostgreSQL result-column type outside `pg_value`'s set. Preserved as a distinct
    /// variant so `exec_core::map_sql_err` can map it back to `sf_sparql::Error::Unsupported`
    /// (501 skip), keeping pre-M3 conformance classification byte-identical.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

/// The crate result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Bridge DuckDB driver errors to [`Error::Marshal`] (ADR-0024 M8).
///
/// `duckdb::Error` does not carry `Send + Sync` bounds required by thiserror's
/// `#[from]` attribute, so we implement `From` manually here, feature-gated so
/// the `duckdb` crate is not compiled unless `duckdb-backend` is enabled.
#[cfg(feature = "duckdb-backend")]
impl From<duckdb::Error> for Error {
    fn from(e: duckdb::Error) -> Self {
        Error::Marshal(format!("duckdb: {e}"))
    }
}
