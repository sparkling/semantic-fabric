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

use std::sync::{Arc, Mutex};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, Rows, Statement};
use sf_core::datatype::{self, XsdTypeCode};

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};
use crate::stream::sqlite_column_decltypes;

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
}

impl BranchStream for SqliteBranch<'_> {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        let Some(row) = self.rows.next()? else {
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
        let mut text = lexical_typed(v, code)?;
        // R2RML §10 / ADR-0015: blank-pad a fixed-length CHAR(n) value to `n`
        // so SQLite matches the SQL-standard value.
        if let (Some(n), Some(s)) = (pads[i], text.as_mut()) {
            for _ in s.chars().count()..n {
                s.push(' ');
            }
        }
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

    async fn open_branch<'s>(
        &'s mut self,
        sql: &str,
        lexical_params: &[String],
    ) -> Result<SqliteBranch<'s>> {
        // §10 declared codes + CHARACTER(n) pads from the prepared statement's
        // column metadata (no rows fetched), then the streaming cursor.
        let (decl_codes, pads, nproj) = column_meta(self.conn, sql)?;
        // Store the prepared statement in the backend so the returned Rows can
        // borrow it for the branch's lifetime (the GAT stream). The Statement
        // borrows the EXTERNAL connection (`*self.conn`, lifetime 'c), not `self`,
        // so this is not a self-referential struct.
        let stmt: Statement<'c> = self.conn.prepare(sql)?;
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

// --- R2 owned cap-1 serve-lane bridge (ADR-0024 §4.1) --------------------------

/// An **owned, `'static`** SQLite backend over `Arc<Mutex<Connection>>` — the serve
/// lane's flavor (design §4.1). Its stream ([`SqliteReceiverStream`]) is the receive
/// end of a **cap-1** channel fed by a `spawn_blocking` cursor thread, so the sync,
/// `!Send` `Connection` never crosses a thread boundary and the stream is
/// `Send + 'static` — what lets the generic core's `for<'s> B::Stream<'s>: Send`
/// bound hold across `tokio::spawn`.
pub struct SqliteOwnedBackend {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteOwnedBackend {
    /// Wrap a shared connection handle (the serve lane already holds this shape).
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

/// The receive end of the cap-1 bridge: each `next_row` awaits the next
/// `Result<RawTuple>` produced by the blocking cursor. `None` ⇒ clean EOF;
/// `Some(Err)` ⇒ a HARD mid-stream marshalling/driver error (design A2), never a
/// silent short read.
pub struct SqliteReceiverStream {
    rx: tokio::sync::mpsc::Receiver<Result<RawTuple>>,
}

impl BranchStream for SqliteReceiverStream {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        match self.rx.recv().await {
            None => Ok(None), // producer finished ⇒ clean EOF
            Some(Ok(tuple)) => Ok(Some(tuple)),
            Some(Err(e)) => Err(e), // forwarded marshalling/driver error (A2)
        }
    }
}

impl SqlBackend for SqliteOwnedBackend {
    type Stream<'s>
        = SqliteReceiverStream
    where
        Self: 's;

    async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
        // Lock inline and DROP the guard before returning — no `.await` is held
        // while the `!Send` `MutexGuard` is live, so the future stays `Send`.
        let guard = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        crate::stream::sqlite_column_names(&guard, probe_sql)
    }

    async fn open_branch(
        &mut self,
        sql: &str,
        lexical_params: &[String],
    ) -> Result<SqliteReceiverStream> {
        // cap-1, FIFO (=_bag-preserving) channel: at most one buffered row in flight
        // + one `&Row` live on the blocking thread ⇒ ~2-row materialisation.
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<RawTuple>>(1);
        let conn = Arc::clone(&self.conn);
        let sql = sql.to_owned();
        let params: Vec<String> = lexical_params.to_vec();
        // The `!Send` Connection / Statement / Rows live ONLY on this blocking
        // thread; `blocking_send` on the cap-1 channel blocks the cursor until the
        // reactor consumes ⇒ explicit backpressure (strengthens bounded memory).
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().unwrap_or_else(|p| p.into_inner());
            let (decl_codes, pads, nproj) = match column_meta(&guard, &sql) {
                Ok(m) => m,
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                    return;
                }
            };
            let mut stmt = match guard.prepare(&sql) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.blocking_send(Err(Error::from(e)));
                    return;
                }
            };
            let mut rows = match stmt.query(rusqlite::params_from_iter(params.iter())) {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.blocking_send(Err(Error::from(e)));
                    return;
                }
            };
            loop {
                match rows.next() {
                    Ok(Some(row)) => {
                        let sent = match marshal_row(row, &decl_codes, &pads, nproj) {
                            Ok(tuple) => tx.blocking_send(Ok(tuple)).is_ok(),
                            Err(e) => {
                                let _ = tx.blocking_send(Err(e));
                                false // hard marshalling error ⇒ stop the cursor (A2)
                            }
                        };
                        if !sent {
                            break; // receiver gone (cancel-on-drop) or error sent
                        }
                    }
                    Ok(None) => break, // clean EOF ⇒ drop tx ⇒ next_row sees None
                    Err(e) => {
                        let _ = tx.blocking_send(Err(Error::from(e)));
                        break;
                    }
                }
            }
        });
        Ok(SqliteReceiverStream { rx })
    }
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
