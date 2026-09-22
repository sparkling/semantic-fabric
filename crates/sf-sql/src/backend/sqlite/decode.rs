//! Per-cell SQLite value decoding (design §2, moved out of `sqlite.rs` to keep
//! it under the file-size limit) — the single home for `marshal_row`, shared by
//! the borrowing [`super::SqliteBranch`] and the owned [`super::SqliteOwnedBackend`]
//! worker-thread bridge. `work`'s control (if any) is charged for the row/code
//! vectors and every cell, mirroring the PostgreSQL adapter's `pg::decode::row`;
//! `SourceWork::new(None)` preserves the previous uncontrolled behavior exactly.

use rusqlite::types::ValueRef;
use sf_core::datatype::{self, XsdTypeCode};

use crate::backend::RawTuple;
use crate::error::{Error, Result};
use crate::source_work::SourceWork;

use super::text_key;

/// Marshal one `rusqlite` `&Row` into a driver-agnostic [`RawTuple`]: per
/// projected column, resolve the §10 type (declared code, else storage-class
/// fallback), read the lexical value ([`lexical_typed`], `hexBinary` blob →
/// uppercase-hex), then blank-pad a fixed-length `CHARACTER(n)` value to `n`
/// (R2RML §10 / ADR-0015).
pub(super) fn row(
    row: &rusqlite::Row<'_>,
    decl_codes: &[Option<XsdTypeCode>],
    pads: &[Option<usize>],
    nproj: usize,
    work: SourceWork<'_>,
) -> Result<RawTuple> {
    work.checkpoint()?;
    let mut values = work.vector(nproj)?;
    let mut codes = work.vector(nproj)?;
    for (i, &decl_code) in decl_codes.iter().enumerate() {
        work.charge(1)?;
        let v = row.get_ref(i)?;
        // §10 type: the declared decl type, else the value's storage class.
        let code = decl_code.or_else(|| storage_class_code(&v));
        let text = match pads[i] {
            Some(width) => text_key::character(v, width, work.control())?,
            None => lexical_typed(v, code, work)?,
        };
        values.push(text);
        codes.push(code);
    }
    work.checkpoint()?;
    Ok(RawTuple { values, codes })
}

/// The §10 type implied by a value's SQLite storage class — the affinity fallback
/// for a column with no declared type (ADR-0015): `INTEGER → xsd:integer`,
/// `REAL → xsd:double`, `BLOB → xsd:hexBinary`; text / NULL carry no implied type.
pub(super) fn storage_class_code(v: &ValueRef<'_>) -> Option<XsdTypeCode> {
    match v {
        ValueRef::Integer(_) => Some(XsdTypeCode::Integer),
        ValueRef::Real(_) => Some(XsdTypeCode::Double),
        ValueRef::Blob(_) => Some(XsdTypeCode::HexBinary),
        ValueRef::Text(_) | ValueRef::Null => None,
    }
}

/// Read a SQLite value as its lexical string (NULL ⇒ `None`). Datatype
/// canonicalisation (R2RML §10) is `sf-core`'s concern; this is the raw lexical
/// extraction. A non-UTF-8 text column / an unhandled BLOB is a hard [`Error::Marshal`].
/// Stays uncharged: `text_key::character` calls it directly after computing its
/// own independent (input + width) charge, so charging inside `lexical` too
/// would double-charge that path.
pub(super) fn lexical(v: ValueRef<'_>) -> Result<Option<String>> {
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
/// hard [`Error::Marshal`]. `work` is charged a conservative worst-case bound
/// before formatting/copying (never after — the allocation itself must stay
/// bounded), then checkpointed.
pub(super) fn lexical_typed(
    v: ValueRef<'_>,
    code: Option<XsdTypeCode>,
    work: SourceWork<'_>,
) -> Result<Option<String>> {
    if let ValueRef::Blob(bytes) = v {
        if code == Some(XsdTypeCode::HexBinary) {
            work.product(bytes.len(), 2)?;
            let mut out = String::new();
            datatype::hex_binary_upper(bytes, &mut out);
            work.checkpoint()?;
            return Ok(Some(out));
        }
    }
    let bound = match v {
        ValueRef::Null => 0,
        ValueRef::Integer(_) => 20,
        // f64::to_string()'s exact worst case (signed subnormal near zero, e.g.
        // -5e-324) -- the previous unbounded 32-byte catch-all undercharged by
        // ~10x.
        ValueRef::Real(_) => 327,
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => bytes.len(),
    };
    work.charge(bound)?;
    let result = lexical(v)?;
    work.checkpoint()?;
    Ok(result)
}

#[cfg(test)]
#[path = "decode_tests.rs"]
mod tests;
