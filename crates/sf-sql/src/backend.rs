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

/// Decoder-preserving text comparison recipe, learned from a live prepare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKey {
    Verbatim,
    SqliteCharacter(usize),
    PostgresCharacter,
}

/// Live wire-decoder identity. Each consumer must separately qualify its value
/// or lexical recipe; a descriptor alone is not source-key authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeScalarKey {
    Integer,
    PostgresBoolean,
    PostgresBytea,
    PostgresNumeric,
    PostgresFloat4,
    PostgresFloat8,
    MysqlBinaryBytes,
    MysqlBit,
    MysqlDecimal,
    /// Value-only binary32 descriptor; no generic lexical or pooled authority.
    MysqlFloat4,
    /// Value-only binary64 authority; Rust shortest lexical identity is unproven.
    MysqlFloat8,
    /// Direct native field only; temporal materialization needs lexical preservation.
    MysqlDate,
    MysqlDateTime,
    MysqlTimestamp,
    MysqlTime,
}

/// Exact SQLite row decoder learned from a live prepare. `declared: None`
/// means authoritative per-cell storage-class fallback, not missing evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SqliteDecode {
    pub declared: Option<XsdTypeCode>,
    pub padding: Option<usize>,
}

/// Live prepare-time evidence for comparison decoration, not schema constraints.
#[derive(Clone, Debug)]
pub struct ResultColumn {
    pub name: String,
    /// Natural RDF datatype from this native prepare/type profile. This is
    /// datatype evidence only, never authority for a lexical comparison recipe.
    pub natural_datatype: Option<XsdTypeCode>,
    /// Exact decoder descriptor, authorized only by native result metadata.
    pub native_scalar: Option<NativeScalarKey>,
    /// How to expose the decoder's exact text value to relational comparisons.
    /// Unknown and non-text families are never guessed or blanket-cast.
    pub text_key: Option<TextKey>,
    pub sqlite_decode: Option<SqliteDecode>,
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

    /// Govern application-side decoding where supported by the adapter.
    /// Native driver allocation is outside this logical source-work boundary.
    async fn next_row_controlled(
        &mut self,
        control: &dyn sf_core::query_control::QueryControl,
    ) -> Result<Option<RawTuple>> {
        control.checkpoint()?;
        let row = self.next_row().await?;
        control.checkpoint()?;
        Ok(row)
    }
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
    /// SQL is generated inside this method. Every error is propagated by the core,
    /// which also validates missing, duplicate, and case-fold-ambiguous live columns
    /// before opening any branch cursor.
    async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>>;

    /// Same metadata-only probe, optionally retaining native varying-text facts.
    async fn result_columns(&mut self, probe_sql: &str) -> Result<Vec<ResultColumn>> {
        Ok(self
            .column_names(probe_sql)
            .await?
            .into_iter()
            .map(|name| ResultColumn {
                natural_datatype: None,
                name,
                native_scalar: None,
                text_key: None,
                sqlite_decode: None,
            })
            .collect())
    }

    /// Request-controlled metadata seam. The compatibility default only checks
    /// terminal state; it does not qualify an adapter's metadata allocations.
    /// Admitted serving adapters must override this or retain the identical
    /// request control in their owned metadata worker.
    async fn result_columns_controlled(
        &mut self,
        probe_sql: &str,
        control: &dyn sf_core::query_control::QueryControl,
    ) -> Result<Vec<ResultColumn>> {
        control.checkpoint()?;
        let columns = self.result_columns(probe_sql).await?;
        control.checkpoint()?;
        Ok(columns)
    }

    /// `metadata_sql` is a compiler-generated, prepare-only twin with identical
    /// output positions, omitting only engine-added comparison decorations.
    /// SQLite uses it because COLLATE otherwise erases declared result types.
    /// Other adapters retain their native result metadata and ignore the twin.
    async fn open_branch_with_metadata<'s>(
        &'s mut self,
        sql: &str,
        lexical_params: &[String],
        _metadata_sql: Option<&str>,
    ) -> Result<Self::Stream<'s>> {
        self.open_branch(sql, lexical_params).await
    }

    /// Compiler-owned decoder requirements, distinct from authored SQL text.
    async fn open_branch_with_decoder<'s>(
        &'s mut self,
        sql: &str,
        lexical_params: &[String],
        metadata_sql: Option<&str>,
        _sqlite_character_keys: bool,
    ) -> Result<Self::Stream<'s>> {
        self.open_branch_with_metadata(sql, lexical_params, metadata_sql)
            .await
    }

    /// Additional compiler-owned lexical keys; the default backend has no such
    /// callback. Never activate this by inspecting authored SQL text.
    async fn open_branch_with_identity<'s>(
        &'s mut self,
        sql: &str,
        lexical_params: &[String],
        metadata_sql: Option<&str>,
        sqlite_character_keys: bool,
        _sqlite_lexical_keys: bool,
    ) -> Result<Self::Stream<'s>> {
        self.open_branch_with_decoder(sql, lexical_params, metadata_sql, sqlite_character_keys)
            .await
    }

    /// Per-open consumer hint. `early_stop` means this one caller may stop
    /// before EOF (ASK, or a LIMIT it applies itself), so an adapter may produce
    /// rows only on demand instead of prefetching. The executor chooses it from
    /// plan shape for each open; it is never inferred from SQL text nor kept as
    /// backend state. The default ignores the hint and forwards every identity
    /// and metadata argument unchanged.
    async fn open_branch_with_demand<'s>(
        &'s mut self,
        sql: &str,
        lexical_params: &[String],
        metadata_sql: Option<&str>,
        sqlite_character_keys: bool,
        sqlite_lexical_keys: bool,
        _early_stop: bool,
    ) -> Result<Self::Stream<'s>> {
        self.open_branch_with_identity(
            sql,
            lexical_params,
            metadata_sql,
            sqlite_character_keys,
            sqlite_lexical_keys,
        )
        .await
    }

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
