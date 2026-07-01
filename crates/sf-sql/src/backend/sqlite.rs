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
//! **Why borrowing, not the cap-1 `spawn_blocking` bridge.** The sync SQLite entry
//! points (`exec::select`/`ask`/`construct`/…) hold only `&Connection`, from which
//! an owned `Arc<Mutex<Connection>>` (required by `spawn_blocking`) cannot be
//! produced. The borrowing GAT stream — the reason the GAT exists (design §0 fact
//! 2) — needs no thread and no channel, keeps one row in flight, and surfaces
//! marshalling errors directly. The `'static` owned `Receiver` flavor (for the
//! serve lane, which already holds `Arc<Mutex<Connection>>`) is deferred to M5 when
//! `sf-serve` collapses onto `run::<B>` (design §4.1).

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
        let mut values = Vec::with_capacity(self.nproj);
        let mut codes = Vec::with_capacity(self.nproj);
        for (i, &decl_code) in self.decl_codes.iter().enumerate() {
            let v = row.get_ref(i)?;
            // §10 type: the declared decl type, else the value's storage class.
            let code = decl_code.or_else(|| storage_class_code(&v));
            let mut text = lexical_typed(v, code)?;
            // R2RML §10 / ADR-0015: blank-pad a fixed-length CHAR(n) value to `n`
            // so SQLite matches the SQL-standard value.
            if let (Some(n), Some(s)) = (self.pads[i], text.as_mut()) {
                for _ in s.chars().count()..n {
                    s.push(' ');
                }
            }
            values.push(text);
            codes.push(code);
        }
        Ok(Some(RawTuple { values, codes }))
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

    async fn open_branch<'s>(
        &'s mut self,
        sql: &str,
        lexical_params: &[String],
    ) -> Result<SqliteBranch<'s>> {
        // §10 declared codes + CHARACTER(n) pads from the prepared statement's
        // column metadata (no rows fetched), then the streaming cursor.
        let decltypes = sqlite_column_decltypes(self.conn, sql)?;
        let decl_codes: Vec<Option<XsdTypeCode>> = decltypes
            .iter()
            .map(|d| d.as_deref().and_then(datatype::natural_xsd))
            .collect();
        let pads: Vec<Option<usize>> = decltypes
            .iter()
            .map(|d| d.as_deref().and_then(char_pad_len))
            .collect();
        let nproj = decltypes.len();
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
