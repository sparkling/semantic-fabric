//! The single per-database execution seam (ADR-0024). Everything BELOW the emitted
//! SQL string. Home = `sf-sql` (alongside `dialect.rs` / `stream.rs` / `error.rs`).
//!
//! `sf-sql` already links `sf-core` and all three driver crates, and `error.rs`
//! already has `#[from]` for `rusqlite` / `tokio_postgres` / `mysql_async`, so the
//! trait adds **zero** new error plumbing (returns [`crate::error::Result`]).
//!
//! Used ONLY via static dispatch (`run::<B>()`), never `dyn SqlBackend`, so
//! async-fn-in-trait (stable ≥1.75) + GAT (stable ≥1.65) carry no object-safety
//! cost (ADR-0024 design §1). The `Send` future the generic core monomorphizes to
//! is required only at the concrete spawn site — proven in M2 for a `'static`
//! stream via the in-crate `MockBackend` probe (design §5 M2 exit gate).

use crate::error::Result;
use sf_core::datatype::XsdTypeCode;

#[cfg(feature = "duckdb-backend")]
pub mod duckdb;
pub mod hana;
pub mod monetdb;
pub mod mysql;
pub mod odbc;
pub mod oracle;
pub mod pg;
pub mod redshift;
pub mod rest;
pub mod sqlite;
pub mod sqlserver;

/// One projected result row, marshalled into the driver-agnostic lexical form the
/// term-gen core consumes (ADR-0003 R3 / ADR-0007). The adapter has ALREADY
/// extracted each cell to its lexical string (NULL ⇒ `None`) via the driver's
/// existing per-cell decoder, derived the §10 natural XSD code (ADR-0015) where the
/// driver carries type info, and applied per-dialect lexical normalisation (SQLite
/// `CHARACTER(n)` blank-pad). Owned by value; one row's `Vec`s are freed each row —
/// the exact per-row allocation the executors already perform. No driver-native
/// `Row` and no driver lifetime ever crosses this boundary.
pub struct RawTuple {
    pub values: Vec<Option<String>>,
    pub codes: Vec<Option<XsdTypeCode>>,
}

/// A bounded pull cursor over ONE emitted branch `SELECT`. One row in flight; the
/// signature CANNOT return a `Vec<Row>`, so no impl can buffer the full result set
/// (ADR-0006 / ADR-0010 §C "bounded by shape").
///
/// `async fn` in trait is deliberate: the seam is used ONLY via static dispatch
/// (`run::<B>()`), never `dyn`, so the auto-trait (`Send`) bound is applied at the
/// concrete monomorphized spawn site, not on the trait method (design §1).
#[allow(async_fn_in_trait)]
pub trait BranchStream {
    /// Next row, or `None` at end. A mid-stream marshalling failure is a HARD `Err`
    /// (never a silent short read): the SQLite bridge forwards `Result<RawTuple>`
    /// so an `Err` surfaces here rather than closing as clean EOF (design A2).
    async fn next_row(&mut self) -> Result<Option<RawTuple>>;
}

/// One driver's prepare / typed-bind / server-side-cursor surface (ADR-0024).
#[allow(async_fn_in_trait)]
pub trait SqlBackend {
    /// GAT so the stream may borrow the handle for its lifetime. PG's `PgRowStream`
    /// and the SQLite channel-bridged `Receiver` are both `'static` (satisfy any
    /// `'s` trivially); only MySQL's native stream actually borrows `&'s mut Conn`.
    type Stream<'s>: BranchStream
    where
        Self: 's;

    /// Prepare-time result-column NAMES of `probe_sql`, in projection order, for
    /// `emit::resolve_col` identifier case-folding. Metadata only — fetches no rows.
    /// `probe_sql` is built ONCE by the core via [`crate::Dialect::probe_sql`], so no
    /// SQL is generated inside this method. A per-source failure is swallowed by the
    /// caller (catalog omits the source; resolution falls back to the raw
    /// identifier), so this returns `Result` but the core never `?`-propagates it.
    async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>>;

    /// Open a server-side cursor for one emitted branch and bind `lexical_params`
    /// (= `EmittedBranch::params`, every value a `&str`) as N positional params.
    ///
    /// TYPED-BIND CONTRACT (the q12 fix, generalised): each lexical value MUST bind
    /// so it satisfies the parameter type the emitted SQL implies for that
    /// placeholder. A dynamically-typed backend binds the string as-is
    /// (SQLite/MySQL); a statically-typed backend parses the lexical form to the
    /// driver-inferred native type (PG). The core NEVER performs this coercion — it
    /// emits only `Vec<String>` and has no bind site.
    async fn open_branch<'s>(
        &'s mut self,
        sql: &str,
        lexical_params: &[String],
    ) -> Result<Self::Stream<'s>>;
}
