//! Generic ODBC `SqlBackend` adapter scaffold (ADR-0024 M8).
//!
//! This scaffold covers databases that Ontop reaches via ODBC: IBM DB2, H2,
//! Spark (Hive ODBC), Dremio, Denodo, and JBoss Teiid. A production
//! implementation would use the `odbc-api` crate (v28+). This scaffold returns
//! [`Error::Unsupported`] for all calls, proving the trait boundary compiles
//! without a live ODBC driver.
//!
//! Named type aliases at the bottom give each database a distinct public type.
//!
//! Verification tier: compiles+unit (scaffold — no `odbc-api` dep).

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

/// Generic ODBC scaffolded backend. Returns `Error::Unsupported` for all calls.
/// Use the named type aliases below for per-database clarity.
pub struct OdbcBackend;

/// Scaffolded stream for ODBC backends — never yields rows.
pub struct OdbcStream;

impl BranchStream for OdbcStream {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        Err(Error::Unsupported(
            "OdbcBackend: odbc-api driver not wired yet".to_owned(),
        ))
    }
}

impl SqlBackend for OdbcBackend {
    type Stream<'s>
        = OdbcStream
    where
        Self: 's;

    async fn column_names(&mut self, _probe_sql: &str) -> Result<Vec<String>> {
        Err(Error::Unsupported(
            "OdbcBackend: odbc-api driver not wired yet".to_owned(),
        ))
    }

    async fn open_branch(
        &mut self,
        _sql: &str,
        _params: &[String],
    ) -> Result<OdbcStream> {
        Err(Error::Unsupported(
            "OdbcBackend: odbc-api driver not wired yet".to_owned(),
        ))
    }
}

// --- per-database named type aliases -----------------------------------------
// These give each database its own public type without duplicating trait impls.

/// IBM DB2 backend (ODBC scaffold). Alias of [`OdbcBackend`].
pub type Db2Backend = OdbcBackend;
/// H2 (embedded Java) backend (ODBC/JDBC scaffold). Alias of [`OdbcBackend`].
pub type H2Backend = OdbcBackend;
/// Apache Spark SQL backend (ODBC scaffold). Alias of [`OdbcBackend`].
pub type SparkBackend = OdbcBackend;
/// Dremio backend (ODBC scaffold). Alias of [`OdbcBackend`].
pub type DremioBackend = OdbcBackend;
/// Denodo virtual-DB backend (ODBC scaffold). Alias of [`OdbcBackend`].
pub type DenodoBackend = OdbcBackend;
/// JBoss Teiid virtual-DB backend (ODBC scaffold). Alias of [`OdbcBackend`].
pub type TeiidBackend = OdbcBackend;
