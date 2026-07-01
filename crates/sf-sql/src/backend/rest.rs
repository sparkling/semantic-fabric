//! REST/HTTP `SqlBackend` adapter scaffolds (ADR-0024 M8).
//!
//! Covers databases that expose a SQL-over-REST or SQL-over-JDBC/ODBC interface:
//! Snowflake, Google BigQuery, AWS Athena, Databricks, Trino, PrestoDB, SAP HANA,
//! and MonetDB. Each returns [`Error::Unsupported`] for all calls today. A
//! production implementation would use the respective HTTP/JDBC SDK.
//!
//! Named type aliases give each database a distinct public type.
//!
//! Verification tier: compiles+unit (scaffold — no REST SDK deps).

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

/// Generic REST/JDBC scaffolded backend.
/// Use the named type aliases below for per-database clarity.
pub struct RestBackend;

/// Scaffolded stream for REST backends — never yields rows.
pub struct RestStream;

impl BranchStream for RestStream {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        Err(Error::Unsupported(
            "RestBackend: REST/JDBC driver not wired yet".to_owned(),
        ))
    }
}

impl SqlBackend for RestBackend {
    type Stream<'s>
        = RestStream
    where
        Self: 's;

    async fn column_names(&mut self, _probe_sql: &str) -> Result<Vec<String>> {
        Err(Error::Unsupported(
            "RestBackend: REST/JDBC driver not wired yet".to_owned(),
        ))
    }

    async fn open_branch(&mut self, _sql: &str, _params: &[String]) -> Result<RestStream> {
        Err(Error::Unsupported(
            "RestBackend: REST/JDBC driver not wired yet".to_owned(),
        ))
    }
}

// --- per-database named type aliases -----------------------------------------

/// Snowflake backend (REST scaffold). Alias of [`RestBackend`].
pub type SnowflakeBackend = RestBackend;
/// Google BigQuery backend (REST scaffold). Alias of [`RestBackend`].
pub type BigQueryBackend = RestBackend;
/// AWS Athena backend (REST scaffold). Alias of [`RestBackend`].
pub type AthenaBackend = RestBackend;
/// Databricks backend (REST scaffold). Alias of [`RestBackend`].
pub type DatabricksBackend = RestBackend;
/// Trino backend (REST scaffold). Alias of [`RestBackend`].
pub type TrinoBackend = RestBackend;
/// PrestoDB backend (REST scaffold). Alias of [`RestBackend`].
pub type PrestoDbBackend = RestBackend;
/// SAP HANA backend (JDBC/REST scaffold). Alias of [`RestBackend`].
pub type SapHanaBackend = RestBackend;
/// MonetDB backend (MAPI/REST scaffold). Alias of [`RestBackend`].
pub type MonetDbBackend = RestBackend;
