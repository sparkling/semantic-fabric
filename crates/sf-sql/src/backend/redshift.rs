//! AWS Redshift `SqlBackend` adapter (ADR-0024 M8).
//!
//! Redshift speaks the PostgreSQL wire protocol; this adapter is a thin
//! type alias over [`PgBackend`](crate::backend::pg::PgBackend) — zero new
//! driver code (the ADR-0024 O(databases)-not-O(databases×features) proof point).
//!
//! Verification tier: compiles+unit (no Redshift cluster in CI; PG wire
//! parity is tested by the existing pg suite).
//!
//! To use: connect with `tokio-postgres` to the Redshift endpoint (which
//! accepts PG wire on port 5439 by default), then wrap the `Client` in a
//! [`RedshiftBackend`] just as you would a `PgBackend`. The only difference
//! is the [`crate::Dialect`] variant (`Dialect::Redshift`) which selects
//! `sqlparser::dialect::RedshiftDialect` for `rr:sqlQuery` parsing.

use crate::backend::pg::PgBackend;

/// AWS Redshift backend — thin type alias over [`PgBackend`].
///
/// Same `tokio-postgres` driver, same `$n` placeholder style, same `"`
/// identifier quoting. Select [`crate::Dialect::Redshift`] at the call site
/// to use `sqlparser::dialect::RedshiftDialect` for SQL parsing.
pub type RedshiftBackend<C> = PgBackend<C>;
