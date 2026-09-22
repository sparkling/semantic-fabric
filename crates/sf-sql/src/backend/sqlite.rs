//! SQLite pull adapters (ADR-0024 §2/§4.1), sharing per-cell decoding via
//! [`decode::row`].
//! [`SqliteBackend`] borrows a connection and keeps one lazy row in flight.
//! [`SqliteOwnedBackend`] keeps its connection on a blocking worker behind a
//! cap-one channel (one buffered and one live row), preserving backpressure.
//! Both propagate marshalling errors as hard failures, never silent short reads.

use rusqlite::{Connection, Rows, Statement};
use sf_core::datatype::{self, XsdTypeCode};

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};
use crate::source_work::SourceWork;

mod cancellation;
mod decode;
mod generation;
mod generation_schema;
pub use generation::{SqliteGenerationConnection, VerifiedSqliteGenerationLease};
pub use generation_schema::SqliteGenerationSchema;
mod iri_key;
mod lexical_key;
#[cfg(test)]
mod metadata_twin_tests;
mod numeric_cmp;
mod owned;
mod text_key;
pub use text_key::{lexical_keys, LexicalKeys};

pub use owned::{
    SqliteOwnedBackend, SqliteOwnedConnection, SqliteOwnedLease, SqliteReceiverStream,
};

/// A borrowing SQLite backend over a live `&Connection`. The current branch's
/// prepared `Statement` is stored in `stmt` so [`SqliteBranch`]'s `Rows` can borrow
/// it for the branch's lifetime (the GAT `Stream<'s>`).
pub struct SqliteBackend<'c> {
    conn: &'c Connection,
    stmt: Option<Statement<'c>>,
}

impl<'c> SqliteBackend<'c> {
    /// Wrap a live connection. The connection outlives the backend.
    pub fn new(conn: &'c Connection) -> Self {
        Self { conn, stmt: None }
    }
}

/// A borrowing SQLite branch cursor: one `&Row` in flight, marshalled to a
/// [`RawTuple`] per `next_row`.
pub struct SqliteBranch<'s> {
    rows: Rows<'s>,
    /// Each projected column's §10 declared code (ADR-0015), `None` ⇒ storage-class
    /// fallback per value.
    decl_codes: Vec<Option<XsdTypeCode>>,
    /// Each projected column's `CHARACTER(n)` blank-pad length, if any.
    pads: Vec<Option<usize>>,
    nproj: usize,
    key: text_key::CharacterKeyGuard<'s>,
    lexical_key: text_key::CharacterKeyGuard<'s>,
}

impl SqliteBranch<'_> {
    async fn next_with_work(&mut self, work: SourceWork<'_>) -> Result<Option<RawTuple>> {
        work.checkpoint()?;
        let Some(row) = self
            .rows
            .next()
            .map_err(|e| self.lexical_key.map_error(self.key.map_error(e.into())))?
        else {
            self.key.finish()?;
            self.lexical_key.finish()?;
            return Ok(None);
        };
        work.checkpoint()?;
        Ok(Some(decode::row(
            row,
            &self.decl_codes,
            &self.pads,
            self.nproj,
            work,
        )?))
    }
}

impl BranchStream for SqliteBranch<'_> {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        self.next_with_work(SourceWork::new(None)).await
    }

    /// Overrides the trait default (checkpoint-only) so a supplied control is
    /// actually threaded into `marshal_row`'s per-cell charging, mirroring the
    /// PostgreSQL adapter's `next_row_controlled` / `next_with_work` split.
    async fn next_row_controlled(
        &mut self,
        control: &dyn sf_core::query_control::QueryControl,
    ) -> Result<Option<RawTuple>> {
        self.next_with_work(SourceWork::new(Some(control))).await
    }
}

impl<'c> SqlBackend for SqliteBackend<'c> {
    type Stream<'s>
        = SqliteBranch<'s>
    where
        Self: 's;

    async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
        crate::stream::sqlite_column_names(self.conn, probe_sql)
    }

    async fn result_columns(
        &mut self,
        probe_sql: &str,
    ) -> Result<Vec<crate::backend::ResultColumn>> {
        result_columns(self.conn, probe_sql)
    }

    async fn result_columns_controlled(
        &mut self,
        probe_sql: &str,
        control: &dyn sf_core::query_control::QueryControl,
    ) -> Result<Vec<crate::backend::ResultColumn>> {
        result_columns_with_control(self.conn, probe_sql, Some(control))
    }

    async fn open_branch<'s>(
        &'s mut self,
        sql: &str,
        lexical_params: &[String],
    ) -> Result<SqliteBranch<'s>> {
        self.open_branch_with_metadata(sql, lexical_params, None)
            .await
    }

    async fn open_branch_with_metadata<'s>(
        &'s mut self,
        sql: &str,
        lexical_params: &[String],
        metadata_sql: Option<&str>,
    ) -> Result<SqliteBranch<'s>> {
        self.open_branch_with_decoder(sql, lexical_params, metadata_sql, false)
            .await
    }

    async fn open_branch_with_decoder<'s>(
        &'s mut self,
        sql: &str,
        lexical_params: &[String],
        metadata_sql: Option<&str>,
        sqlite_character_keys: bool,
    ) -> Result<SqliteBranch<'s>> {
        self.open_branch_with_identity(
            sql,
            lexical_params,
            metadata_sql,
            sqlite_character_keys,
            false,
        )
        .await
    }

    async fn open_branch_with_identity<'s>(
        &'s mut self,
        sql: &str,
        lexical_params: &[String],
        metadata_sql: Option<&str>,
        sqlite_character_keys: bool,
        sqlite_lexical_keys: bool,
    ) -> Result<SqliteBranch<'s>> {
        // §10 declared codes + CHARACTER(n) pads from the prepared statement's
        // column metadata (no rows fetched), then the streaming cursor.
        let key = text_key::CharacterKeyGuard::install(self.conn, sqlite_character_keys, None)?;
        let lexical_key =
            text_key::CharacterKeyGuard::install_lexical(self.conn, sqlite_lexical_keys, None)?;
        let (decl_codes, pads, nproj) = column_meta(self.conn, metadata_sql.unwrap_or(sql), None)?;
        // Store the prepared statement in the backend so the returned Rows can
        // borrow it for the branch's lifetime (the GAT stream). The Statement
        // borrows the EXTERNAL connection (`*self.conn`, lifetime 'c), not `self`,
        // so this is not a self-referential struct.
        let stmt: Statement<'c> = self.conn.prepare(sql)?;
        if stmt.column_count() != nproj {
            return Err(Error::Emit(
                "SQLite metadata twin projection mismatch".into(),
            ));
        }
        self.stmt = Some(stmt);
        let rows = self
            .stmt
            .as_mut()
            .expect("just stored")
            .query(rusqlite::params_from_iter(lexical_params.iter()))?;
        Ok(SqliteBranch {
            rows,
            decl_codes,
            pads,
            nproj,
            key,
            lexical_key,
        })
    }
}

/// Per-column prepare-time metadata: the §10 declared codes, the `CHARACTER(n)` pad
/// lengths, and the projected column count — the tuple both flavors thread from
/// [`column_meta`] into their row marshalling.
type ColumnMeta = (Vec<Option<XsdTypeCode>>, Vec<Option<usize>>, usize);

/// Prepare `sql` and derive each projected column's §10 declared code (ADR-0015,
/// `None` ⇒ storage-class fallback per value) plus its `CHARACTER(n)` blank-pad
/// length — metadata only, no rows fetched. Shared by both flavors' `open_branch`.
fn column_meta(
    conn: &Connection,
    sql: &str,
    control: Option<&dyn sf_core::query_control::QueryControl>,
) -> Result<ColumnMeta> {
    let work = crate::source_work::SourceWork::new(control);
    let decltypes = crate::stream::sqlite_column_decltypes_controlled(conn, sql, work)?;
    work.product(decltypes.len(), 2)?;
    let decl_codes: Vec<Option<XsdTypeCode>> = decltypes
        .iter()
        .map(|d| d.as_deref().and_then(datatype::natural_xsd))
        .collect();
    let pads: Vec<Option<usize>> = decltypes
        .iter()
        .map(|d| d.as_deref().and_then(char_pad_len))
        .collect();
    let nproj = decltypes.len();
    Ok((decl_codes, pads, nproj))
}

/// Metadata facts from one prepare, preserving authored transparent COLLATE.
fn result_columns(conn: &Connection, sql: &str) -> Result<Vec<crate::backend::ResultColumn>> {
    result_columns_with_control(conn, sql, None)
}

fn result_columns_with_control(
    conn: &Connection,
    sql: &str,
    control: Option<&dyn sf_core::query_control::QueryControl>,
) -> Result<Vec<crate::backend::ResultColumn>> {
    use crate::backend::{ResultColumn, TextKey};
    let work = crate::source_work::SourceWork::new(control);
    work.checkpoint()?;
    let stmt = conn.prepare(sql)?;
    // rusqlite constructs a vector of borrowed descriptors in columns().
    work.product(
        stmt.column_count(),
        std::mem::size_of::<rusqlite::Column<'_>>(),
    )?;
    let columns = stmt.columns();
    let mut declared = work.vector(columns.len())?;
    for column in &columns {
        work.charge(1)?;
        declared.push(
            column
                .decl_type()
                .map(|name| work.string(name))
                .transpose()?,
        );
    }
    let declared = crate::stream::sqlite_metadata::recover_collated_decltypes_controlled(
        conn, sql, declared, work,
    )?;
    let mut output = work.vector(columns.len())?;
    for (column, decl) in columns.iter().zip(declared) {
        work.charge(1)?;
        let (code, padding) = match decl.as_deref() {
            Some(decl) => crate::source_work::declaration_metadata(decl, work)?,
            None => (None, None),
        };
        let text_key = padding
            .map(TextKey::SqliteCharacter)
            .or_else(|| (code == Some(XsdTypeCode::String)).then_some(TextKey::Verbatim));
        output.push(ResultColumn {
            natural_datatype: code,
            native_scalar: None,
            name: work.string(column.name())?,
            text_key,
            sqlite_decode: Some(crate::backend::SqliteDecode {
                declared: code,
                padding,
            }),
        });
    }
    work.checkpoint()?;
    Ok(output)
}

// --- per-cell marshalling (moved VERBATIM from sf-sparql::exec, design §2) -----

/// The fixed `CHARACTER(n)` pad length, if `decl` is a fixed-length char type
/// (`CHAR` / `CHARACTER` / `NCHAR`) with an explicit `(n)` — never a *varying* type.
fn char_pad_len(decl: &str) -> Option<usize> {
    let open = decl.find('(')?;
    let close = decl[open..].find(')')? + open;
    let name: String = decl[..open]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase();
    if !matches!(name.as_str(), "CHAR" | "CHARACTER" | "NCHAR") {
        return None;
    }
    decl[open + 1..close].trim().parse::<usize>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- char_pad_len -------------------------------------------------------

    #[test]
    fn char_pad_len_parses_char_with_length() {
        assert_eq!(char_pad_len("CHAR(10)"), Some(10));
        assert_eq!(char_pad_len("CHARACTER(5)"), Some(5));
        assert_eq!(char_pad_len("NCHAR(3)"), Some(3));
    }

    #[test]
    fn char_pad_len_is_case_insensitive_and_tolerates_whitespace() {
        assert_eq!(char_pad_len("char (7)"), Some(7));
        assert_eq!(char_pad_len("  Character(2)"), Some(2));
    }

    #[test]
    fn char_pad_len_rejects_varying_types() {
        // VARCHAR is a *varying*-length type — never padded, even with an
        // explicit (n).
        assert_eq!(char_pad_len("VARCHAR(10)"), None);
    }

    #[test]
    fn char_pad_len_rejects_no_length_or_malformed_decl() {
        assert_eq!(char_pad_len("CHAR"), None); // no parens at all
        assert_eq!(char_pad_len("CHAR()"), None); // empty parens
        assert_eq!(char_pad_len("TEXT"), None);
    }
}
