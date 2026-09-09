//! SQLite `SqlBackend` adapter (ADR-0024 §2). A **borrowing, synchronous** pull
//! cursor: `SqliteBackend` holds `&Connection`, `open_branch` prepares the emitted
//! branch and stores its `Statement` in the backend, and `SqliteBranch` drives
//! `rusqlite`'s lazy `Rows` cursor — **one `&Row` in flight**, so memory is
//! independent of result size (ADR-0006). The per-cell marshalling
//! (`storage_class_code` / `lexical_typed` / `CHARACTER(n)` blank-pad) is moved here
//! **verbatim** from the old `sf-sparql::exec` SQLite loop (design §2 sqlite row).
//!
//! **A2 (design §2):** a mid-row marshalling failure (non-UTF-8 text, BLOB in a
//! non-`hexBinary` position) is a HARD [`Error::Marshal`] returned by `next_row`,
//! never a silent short read.
//!
//! **Two flavors, one marshalling.** The sync SQLite entry points
//! (`exec::select`/`ask`/`construct`/…) hold only `&Connection`, from which an owned
//! `Arc<Mutex<Connection>>` cannot be produced — they use the borrowing
//! [`SqliteBackend`], whose GAT stream (the reason the GAT exists, design §0 fact 2)
//! needs no thread and no channel, keeps one `&Row` in flight, and surfaces
//! marshalling errors directly. The **serve lane** (which holds
//! `Arc<Mutex<Connection>>`) uses [`SqliteOwnedBackend`] (design §4.1): the sync,
//! `!Send` `Connection` lives only on a `spawn_blocking` thread behind a **cap-1**
//! channel, so the owned `Receiver` stream is `Send + 'static` and the core future
//! stays `Send` across `tokio::spawn`. `blocking_send` on the cap-1 channel blocks
//! the cursor thread until the reactor consumes ⇒ explicit backpressure that
//! *strengthens* the bounded-memory guarantee (≈2 rows materialised). Both flavors
//! share the exact per-cell marshalling ([`marshal_row`]).

use rusqlite::types::ValueRef;
use rusqlite::{Connection, Rows, Statement};
use sf_core::datatype::{self, XsdTypeCode};

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};
use crate::stream::sqlite_column_decltypes;

mod cancellation;
mod iri_key;
mod lexical_key;
#[cfg(test)]
mod metadata_twin_tests;
mod numeric_cmp;
mod owned;
mod text_key;

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

impl BranchStream for SqliteBranch<'_> {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        let Some(row) = self
            .rows
            .next()
            .map_err(|e| self.lexical_key.map_error(self.key.map_error(e.into())))?
        else {
            self.key.finish()?;
            self.lexical_key.finish()?;
            return Ok(None);
        };
        Ok(Some(marshal_row(
            row,
            &self.decl_codes,
            &self.pads,
            self.nproj,
        )?))
    }
}

/// Marshal one `rusqlite` `&Row` into a driver-agnostic [`RawTuple`] (design §2 —
/// the single SQLite per-cell marshalling home, shared by the borrowing
/// [`SqliteBranch`] and the owned [`SqliteOwnedBackend`] bridge): per projected
/// column, resolve the §10 type (declared code, else storage-class fallback), read
/// the lexical value ([`lexical_typed`], `hexBinary` blob → uppercase-hex), then
/// blank-pad a fixed-length `CHARACTER(n)` value to `n` (R2RML §10 / ADR-0015).
fn marshal_row(
    row: &rusqlite::Row<'_>,
    decl_codes: &[Option<XsdTypeCode>],
    pads: &[Option<usize>],
    nproj: usize,
) -> Result<RawTuple> {
    let mut values = Vec::with_capacity(nproj);
    let mut codes = Vec::with_capacity(nproj);
    for (i, &decl_code) in decl_codes.iter().enumerate() {
        let v = row.get_ref(i)?;
        // §10 type: the declared decl type, else the value's storage class.
        let code = decl_code.or_else(|| storage_class_code(&v));
        let text = match pads[i] {
            Some(width) => text_key::character(v, width, None)?,
            None => lexical_typed(v, code)?,
        };
        values.push(text);
        codes.push(code);
    }
    Ok(RawTuple { values, codes })
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
        let (decl_codes, pads, nproj) = column_meta(self.conn, metadata_sql.unwrap_or(sql))?;
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
fn column_meta(conn: &Connection, sql: &str) -> Result<ColumnMeta> {
    let decltypes = sqlite_column_decltypes(conn, sql)?;
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
    use crate::backend::{ResultColumn, TextKey};
    let stmt = conn.prepare(sql)?;
    let columns = stmt.columns();
    let declared = columns
        .iter()
        .map(|c| c.decl_type().map(str::to_owned))
        .collect();
    let declared = crate::stream::sqlite_metadata::recover_collated_decltypes(conn, sql, declared)?;
    Ok(columns
        .iter()
        .zip(declared)
        .map(|(column, decl)| {
            let text_key = decl.as_deref().and_then(|decl| {
                if let Some(width) = char_pad_len(decl) {
                    Some(TextKey::SqliteCharacter(width))
                } else if datatype::natural_xsd(decl) == Some(XsdTypeCode::String) {
                    Some(TextKey::Verbatim)
                } else {
                    None
                }
            });
            ResultColumn {
                name: column.name().to_owned(),
                text_key,
                sqlite_decode: Some(crate::backend::SqliteDecode {
                    declared: decl.as_deref().and_then(datatype::natural_xsd),
                    padding: decl.as_deref().and_then(char_pad_len),
                }),
            }
        })
        .collect())
}

// --- per-cell marshalling (moved VERBATIM from sf-sparql::exec, design §2) -----

/// The §10 type implied by a value's SQLite storage class — the affinity fallback
/// for a column with no declared type (ADR-0015): `INTEGER → xsd:integer`,
/// `REAL → xsd:double`, `BLOB → xsd:hexBinary`; text / NULL carry no implied type.
fn storage_class_code(v: &ValueRef<'_>) -> Option<XsdTypeCode> {
    match v {
        ValueRef::Integer(_) => Some(XsdTypeCode::Integer),
        ValueRef::Real(_) => Some(XsdTypeCode::Double),
        ValueRef::Blob(_) => Some(XsdTypeCode::HexBinary),
        ValueRef::Text(_) | ValueRef::Null => None,
    }
}

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

/// Read a SQLite value as its lexical string (NULL ⇒ `None`). Datatype
/// canonicalisation (R2RML §10) is `sf-core`'s concern; this is the raw lexical
/// extraction. A non-UTF-8 text column / an unhandled BLOB is a hard [`Error::Marshal`].
fn lexical(v: ValueRef<'_>) -> Result<Option<String>> {
    Ok(match v {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) => Some(
            std::str::from_utf8(t)
                .map_err(|e| Error::Marshal(format!("non-UTF8 text column: {e}")))?
                .to_owned(),
        ),
        ValueRef::Blob(_) => return Err(Error::Marshal("BLOB column reconstruction".to_owned())),
    })
}

/// Extract a column value with its target §10 type in view: a `BLOB` feeding an
/// `xsd:hexBinary` column is uppercase-hex-encoded here (ADR-0015); every other
/// storage class is read by [`lexical`]. A blob in a non-hexBinary position is a
/// hard [`Error::Marshal`].
fn lexical_typed(v: ValueRef<'_>, code: Option<XsdTypeCode>) -> Result<Option<String>> {
    if let ValueRef::Blob(bytes) = v {
        if code == Some(XsdTypeCode::HexBinary) {
            let mut out = String::new();
            datatype::hex_binary_upper(bytes, &mut out);
            return Ok(Some(out));
        }
    }
    lexical(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- storage_class_code -----------------------------------------------

    #[test]
    fn storage_class_code_integer_maps_to_xsd_integer() {
        assert_eq!(
            storage_class_code(&ValueRef::Integer(42)),
            Some(XsdTypeCode::Integer)
        );
    }

    #[test]
    fn storage_class_code_real_maps_to_xsd_double() {
        assert_eq!(
            storage_class_code(&ValueRef::Real(1.5)),
            Some(XsdTypeCode::Double)
        );
    }

    #[test]
    fn storage_class_code_blob_maps_to_hex_binary() {
        assert_eq!(
            storage_class_code(&ValueRef::Blob(&[1, 2, 3])),
            Some(XsdTypeCode::HexBinary)
        );
    }

    #[test]
    fn storage_class_code_text_and_null_carry_no_implied_type() {
        assert_eq!(storage_class_code(&ValueRef::Text(b"hi")), None);
        assert_eq!(storage_class_code(&ValueRef::Null), None);
    }

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

    // --- lexical --------------------------------------------------------------

    #[test]
    fn lexical_null_is_none() {
        assert_eq!(lexical(ValueRef::Null).unwrap(), None);
    }

    #[test]
    fn lexical_integer_and_real_render_via_to_string() {
        assert_eq!(lexical(ValueRef::Integer(7)).unwrap(), Some("7".to_owned()));
        assert_eq!(
            lexical(ValueRef::Real(1.5)).unwrap(),
            Some("1.5".to_owned())
        );
    }

    #[test]
    fn lexical_valid_utf8_text_passes_through() {
        assert_eq!(
            lexical(ValueRef::Text(b"hello")).unwrap(),
            Some("hello".to_owned())
        );
    }

    #[test]
    fn lexical_non_utf8_text_is_a_hard_marshal_error() {
        let invalid = &[0xff, 0xfe][..];
        let err = lexical(ValueRef::Text(invalid)).unwrap_err();
        assert!(
            matches!(err, Error::Marshal(_)),
            "expected Marshal, got {err:?}"
        );
    }

    #[test]
    fn lexical_bare_blob_is_a_hard_marshal_error() {
        // lexical() (unlike lexical_typed()) has no target-type context, so it
        // can never soundly decide a BLOB is hexBinary — always errors.
        let err = lexical(ValueRef::Blob(&[1, 2, 3])).unwrap_err();
        assert!(
            matches!(err, Error::Marshal(_)),
            "expected Marshal, got {err:?}"
        );
    }

    // --- lexical_typed --------------------------------------------------------

    #[test]
    fn lexical_typed_blob_with_hexbinary_target_encodes_uppercase_hex() {
        let out = lexical_typed(
            ValueRef::Blob(&[0xde, 0xad, 0xbe, 0xef]),
            Some(XsdTypeCode::HexBinary),
        )
        .unwrap();
        assert_eq!(out, Some("DEADBEEF".to_owned()));
    }

    #[test]
    fn lexical_typed_blob_without_hexbinary_target_still_errors() {
        // A BLOB feeding a non-hexBinary-typed column (or no declared type) has
        // no sound rendering — falls through to lexical()'s hard error.
        let err = lexical_typed(ValueRef::Blob(&[1, 2, 3]), None).unwrap_err();
        assert!(matches!(err, Error::Marshal(_)));
        let err2 =
            lexical_typed(ValueRef::Blob(&[1, 2, 3]), Some(XsdTypeCode::String)).unwrap_err();
        assert!(matches!(err2, Error::Marshal(_)));
    }

    #[test]
    fn lexical_typed_non_blob_delegates_to_lexical_regardless_of_code() {
        assert_eq!(
            lexical_typed(ValueRef::Integer(9), Some(XsdTypeCode::HexBinary)).unwrap(),
            Some("9".to_owned())
        );
    }
}
