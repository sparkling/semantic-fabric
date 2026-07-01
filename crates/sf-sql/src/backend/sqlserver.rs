//! SQL Server `SqlBackend` adapter scaffold (ADR-0024 M8).
//!
//! A production implementation would use the `tiberius` crate (TDS protocol,
//! async, `@P1`-style placeholders). This scaffold compiles without the
//! `tiberius` dependency and returns [`Error::Unsupported`] for all calls,
//! proving the trait boundary compiles without a live driver.
//!
//! To activate: add `tiberius = "0.12"` + `tokio-util = { version = "0.7",
//! features = ["compat"] }` to `sf-sql`'s `[dependencies]` (optionally feature-
//! gated), replace `SqlServerBackend` with a `tiberius::Client<Compat<...>>`
//! wrapper, and drive it with `client.query(sql, &[...]).await`.
//!
//! Verification tier: compiles+unit (scaffold — no `tiberius` dep).

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

/// SQL Server scaffolded backend. Returns `Error::Unsupported` for all calls.
pub struct SqlServerBackend;

/// Scaffolded stream for SQL Server — never yields rows.
pub struct SqlServerStream;

impl BranchStream for SqlServerStream {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        Err(Error::Unsupported(
            "SqlServerBackend: tiberius driver not wired yet".to_owned(),
        ))
    }
}

impl SqlBackend for SqlServerBackend {
    type Stream<'s>
        = SqlServerStream
    where
        Self: 's;

    async fn column_names(&mut self, _probe_sql: &str) -> Result<Vec<String>> {
        Err(Error::Unsupported(
            "SqlServerBackend: tiberius driver not wired yet".to_owned(),
        ))
    }

    async fn open_branch(
        &mut self,
        _sql: &str,
        _params: &[String],
    ) -> Result<SqlServerStream> {
        Err(Error::Unsupported(
            "SqlServerBackend: tiberius driver not wired yet".to_owned(),
        ))
    }
}
