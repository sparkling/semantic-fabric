//! Oracle Database `SqlBackend` adapter scaffold (ADR-0024 M8).
//!
//! A production implementation would use the `oracle` crate (OCI bindings,
//! `:n` placeholders) or `odbc-api` via an Oracle ODBC driver. This scaffold
//! returns [`Error::Unsupported`] for all calls, proving the trait boundary
//! compiles without a live OCI driver.
//!
//! Verification tier: compiles+unit (scaffold — no OCI dep).

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

/// Oracle Database scaffolded backend. Returns `Error::Unsupported` for all calls.
pub struct OracleBackend;

/// Scaffolded stream for Oracle — never yields rows.
pub struct OracleStream;

impl BranchStream for OracleStream {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        Err(Error::Unsupported(
            "OracleBackend: OCI driver not wired yet".to_owned(),
        ))
    }
}

impl SqlBackend for OracleBackend {
    type Stream<'s>
        = OracleStream
    where
        Self: 's;

    async fn column_names(&mut self, _probe_sql: &str) -> Result<Vec<String>> {
        Err(Error::Unsupported(
            "OracleBackend: OCI driver not wired yet".to_owned(),
        ))
    }

    async fn open_branch(&mut self, _sql: &str, _params: &[String]) -> Result<OracleStream> {
        Err(Error::Unsupported(
            "OracleBackend: OCI driver not wired yet".to_owned(),
        ))
    }
}
