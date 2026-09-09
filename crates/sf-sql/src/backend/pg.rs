//! PostgreSQL `SqlBackend` adapter (ADR-0024 §2). A **`'static`, async** pull cursor
//! over `tokio_postgres`'s `query_raw` server-side portal (never the buffer-all
//! `query()`), so memory stays cursor-grade / bounded by result *shape* (ADR-0006 /
//! ADR-0010 §C). The per-cell marshalling (`pg_xsd_code` §10 code derivation,
//! `pg_value` lexical extraction) and the q12 typed-column bind wrapper
//! (`LexicalParam: ToSql`) are moved here **verbatim** from the old
//! `sf-sparql::exec_pg` PG loop (design §2 pg row).
//!
//! An uncovered PostgreSQL result type is a HARD [`Error::Unsupported`] returned by
//! `next_row` — preserved as a distinct variant so `exec_core::map_sql_err` maps it
//! back to `sf_sparql::Error::Unsupported` (501 skip), keeping the pre-M3
//! conformance classification byte-identical.

use std::error::Error as _;
use std::ops::Deref;

use sf_core::datatype::{self, XsdTypeCode};
use tokio_postgres::types::{FromSql, ToSql, Type};
use tokio_postgres::{Client, Row as PgRow};

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};
use crate::stream::PgRowStream;

mod timetz;

use timetz::pg_timetz_value;

/// A PostgreSQL backend over any handle that derefs to a live [`Client`]. Generic
/// over the holder `C` so the same adapter serves both lanes:
///
///   * `PgBackend<&Client>` — the borrowing collecting path (`select_pg` / `ask_pg`
///     / …), driven to completion in place.
///   * `PgBackend<Arc<Client>>` — the **`'static`** streaming serve lane
///     (`select_each_pg` / `construct_each_pg`), whose future is `tokio::spawn`ed;
///     a `'static` backend is what lets the generic core's `Send` bound
///     (`for<'s> B::Stream<'s>: Send`, ADR-0024 §1.103) hold across the spawn.
///
/// The returned [`PgRowStream`] is `'static` (owns its portal), so it satisfies the
/// GAT `Stream<'s>` for any `'s`.
pub struct PgBackend<C> {
    client: C,
}

impl<C: Deref<Target = Client>> PgBackend<C> {
    /// Wrap a live client handle (`&Client` or `Arc<Client>`).
    pub fn new(client: C) -> Self {
        Self { client }
    }
}

/// The §10 natural XSD type implied by a PostgreSQL result-column type
/// (ADR-0015). Text-like types carry no implied datatype (plain literal) and map
/// to [`XsdTypeCode::String`]; an unrecognised type yields `None`, which the
/// reconstruction treats as a plain literal.
fn pg_xsd_code(ty: &Type) -> Option<XsdTypeCode> {
    use XsdTypeCode::*;
    match *ty {
        Type::BOOL => Some(Boolean),
        Type::INT2 | Type::INT4 | Type::INT8 => Some(Integer),
        Type::FLOAT4 | Type::FLOAT8 => Some(Double),
        // M3 fix 2 (was TRACKED RESIDUE): `pg_value` below now decodes `NUMERIC`'s binary
        // wire format by hand (`decode_pg_numeric` + the `PgNumeric` FromSql wrapper) —
        // `postgres-types` 0.2.14 has no `rust_decimal`/decimal `FromSql` route at all (no
        // feature flag to enable), unlike DATE/TIME/TIMESTAMP's `chrono` route, so a sound
        // fix needed hand-rolled parsing. A live NUMERIC column now reads as an exact
        // `xsd:decimal` lexical string, never a float. NaN/±Infinity have no `xsd:decimal`
        // representation and still hard-501 via `Error::Unsupported` — sound per ADR-0007
        // (an honest error, never a wrong answer).
        Type::NUMERIC => Some(Decimal),
        Type::DATE => Some(Date),
        Type::TIME | Type::TIMETZ => Some(Time),
        Type::TIMESTAMP | Type::TIMESTAMPTZ => Some(DateTime),
        Type::BYTEA => Some(HexBinary),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::CHAR | Type::UNKNOWN => {
            Some(String)
        }
        _ => None,
    }
}

/// A bound parameter carried as its **lexical SPARQL form** (`&str`), serialised
/// to whatever PostgreSQL type the prepared statement infers for the placeholder.
///
/// All emitted `$n` values arrive as strings (`EmittedBranch::params`), but a
/// FILTER constant compared against a typed column lowers to a bare placeholder —
/// e.g. `FILTER(?d = 1)` over an `INT4` column emits `"direction_id" = $1`, where
/// PostgreSQL infers `$1` as `INT4`. Binding the raw Rust `String` there fails
/// *client-side* (`String` does not `accepts(INT4)`), aborting the already-200
/// response body mid-stream. This wrapper inspects the driver-supplied `ty` at
/// serialise time and parses the lexical form into the native Rust type
/// (delegating to that type's own `ToSql`), so integer/float/boolean placeholders
/// bind correctly. Text-like (and any other) placeholders fall through to the
/// plain string binding — byte-identical to the previous behaviour, so the
/// passing text/string FILTER paths are untouched. Values stay bound parameters
/// (ADR-0010 R1) — never interpolated.
#[derive(Debug)]
struct LexicalParam<'a>(&'a str);

impl ToSql for LexicalParam<'_> {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> std::result::Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    {
        match *ty {
            Type::BOOL => self.0.parse::<bool>()?.to_sql(ty, out),
            Type::INT2 => self.0.parse::<i16>()?.to_sql(ty, out),
            Type::INT4 => self.0.parse::<i32>()?.to_sql(ty, out),
            Type::INT8 => self.0.parse::<i64>()?.to_sql(ty, out),
            Type::FLOAT4 => self.0.parse::<f32>()?.to_sql(ty, out),
            Type::FLOAT8 => self.0.parse::<f64>()?.to_sql(ty, out),
            // Text-like and everything else: bind the raw lexical string, exactly
            // as the previous `&String` binding did.
            _ => self.0.to_sql(ty, out),
        }
    }

    // Accept every placeholder type; `to_sql` dispatches on the actual `ty`, so
    // the driver never rejects the bind before we get to parse it.
    fn accepts(_ty: &Type) -> bool {
        true
    }

    tokio_postgres::types::to_sql_checked!();
}

/// A hand-decoded PostgreSQL `NUMERIC` binary value (M3 fix 2, was the TRACKED
/// RESIDUE noted on [`pg_xsd_code`]): the arbitrary-precision decimal LEXICAL
/// STRING reconstructed from PG's wire format — `postgres-types` 0.2.14 has no
/// `NUMERIC`/decimal `FromSql` route at all (no feature flag to enable). This
/// type's ONLY job is that lexical string, NEVER a float; XSD-canonicalisation
/// happens downstream at the shared `sf-sparql` reconstruction chokepoint,
/// identically to every other dialect (mirrors [`pg_value`]'s own contract: every
/// arm returns a raw lexical string, not a `Term`).
struct PgNumeric(String);

impl<'a> FromSql<'a> for PgNumeric {
    fn from_sql(
        _ty: &Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(PgNumeric(decode_pg_numeric(raw)?))
    }

    fn accepts(ty: &Type) -> bool {
        matches!(*ty, Type::NUMERIC)
    }
}

/// Decode a PostgreSQL `NUMERIC` binary wire value into its arbitrary-precision
/// decimal LEXICAL STRING — never through a float (M3 fix 2). Wire format (PG's
/// `numeric_send`/`numeric_recv`): `u16 ndigits`, `i16 weight`, `u16 sign`, `u16
/// dscale`, then `ndigits` × `i16` base-10000 digits (most significant first);
/// `value = sign * Σ digits[i] * 10000^(weight-i)`. `dscale` is the DISPLAY
/// fractional-digit count independent of how many digit groups are actually
/// stored — a trailing all-zero group, on EITHER side of the decimal point (e.g.
/// `100000000` stored as a single digit at `weight=2`; a `dscale` needing more
/// fractional digits than are stored), is never transmitted on the wire, only
/// implied.
fn decode_pg_numeric(raw: &[u8]) -> Result<String> {
    fn be_i16(b: &[u8]) -> Result<i16> {
        b.try_into()
            .map(i16::from_be_bytes)
            .map_err(|_| Error::Marshal("PG NUMERIC: truncated header".to_owned()))
    }
    fn be_u16(b: &[u8]) -> Result<u16> {
        b.try_into()
            .map(u16::from_be_bytes)
            .map_err(|_| Error::Marshal("PG NUMERIC: truncated header".to_owned()))
    }

    // `numeric_send`'s sign field: the only two "normal" values, plus NaN and (PG
    // 14+) the two infinities — none of which has an `xsd:decimal` lexical form.
    const POS: u16 = 0x0000;
    const NEG: u16 = 0x4000;
    const NAN: u16 = 0xC000;
    const PINF: u16 = 0xD000;
    const NINF: u16 = 0xF000;

    if raw.len() < 8 {
        return Err(Error::Marshal(format!(
            "PG NUMERIC: header too short ({} bytes)",
            raw.len()
        )));
    }
    let ndigits = usize::from(be_u16(&raw[0..2])?);
    let weight = be_i16(&raw[2..4])?;
    let sign = be_u16(&raw[4..6])?;
    let dscale = be_u16(&raw[6..8])?;
    match sign {
        NAN => {
            return Err(Error::Unsupported(
                "PostgreSQL NUMERIC NaN has no xsd:decimal representation".to_owned(),
            ));
        }
        PINF => {
            return Err(Error::Unsupported(
                "PostgreSQL NUMERIC +Infinity has no xsd:decimal representation".to_owned(),
            ));
        }
        NINF => {
            return Err(Error::Unsupported(
                "PostgreSQL NUMERIC -Infinity has no xsd:decimal representation".to_owned(),
            ));
        }
        POS | NEG => {}
        other => {
            return Err(Error::Marshal(format!(
                "PG NUMERIC: unrecognised sign 0x{other:04X}"
            )));
        }
    }
    if raw.len() < 8 + ndigits * 2 {
        return Err(Error::Marshal(format!(
            "PG NUMERIC: digit array truncated (need {} bytes, have {})",
            8 + ndigits * 2,
            raw.len()
        )));
    }
    let mut digits = Vec::with_capacity(ndigits);
    for i in 0..ndigits {
        digits.push(i32::from(be_i16(&raw[8 + i * 2..10 + i * 2])?));
    }

    // The base-10000 digit at place-value `position` (i.e. contributing
    // `digit * 10000^position`) — 0 for any position outside the stored
    // `[weight-ndigits+1, weight]` range (an implicit leading/trailing zero group).
    let digit_at = |position: i32| -> i32 {
        let i = i32::from(weight) - position;
        if i >= 0 && (i as usize) < digits.len() {
            digits[i as usize]
        } else {
            0
        }
    };

    let mut s = String::new();
    if sign == NEG {
        s.push('-');
    }
    if weight < 0 {
        s.push('0'); // no integer part at all
    } else {
        let mut first = true;
        for position in (0..=i32::from(weight)).rev() {
            let g = digit_at(position);
            if first {
                s.push_str(&g.to_string()); // the leading group: no zero-pad
                first = false;
            } else {
                s.push_str(&format!("{g:04}")); // every later group: 4-digit zero-pad
            }
        }
    }
    if dscale > 0 {
        s.push('.');
        let groups_needed = usize::from(dscale).div_ceil(4);
        let mut frac = String::with_capacity(groups_needed * 4);
        for k in 0..groups_needed {
            frac.push_str(&format!("{:04}", digit_at(-1 - k as i32)));
        }
        frac.truncate(dscale as usize);
        s.push_str(&frac);
    }
    Ok(s)
}

/// Extract column `idx` of `row` as its raw lexical string (NULL ⇒ `None`),
/// fetched in the most type-faithful driver form (ADR-0015) — integers/floats as
/// their native Rust type, `bytea` uppercase-hex-encoded, booleans as
/// `true`/`false` (never PostgreSQL's `t`/`f`). XSD-canonicalisation of the
/// lexical form is the downstream sf-core chokepoint's concern. A type the
/// reader does not cover surfaces as a hard [`Error::Unsupported`] (turned into a
/// documented `501` skip by the conformance / serve layer).
fn recover_pg_from_sql_error(error: tokio_postgres::Error) -> Error {
    let recovered = error
        .source()
        .and_then(|source| source.downcast_ref::<Error>())
        .and_then(|source| match source {
            Error::Marshal(message) => Some(Error::Marshal(message.clone())),
            Error::Unsupported(message) => Some(Error::Unsupported(message.clone())),
            _ => None,
        });
    recovered.unwrap_or_else(|| Error::from(error))
}

fn pg_value(row: &PgRow, idx: usize, ty: &Type) -> Result<Option<String>> {
    let s = match *ty {
        Type::BOOL => row.try_get::<_, Option<bool>>(idx)?.map(|b| b.to_string()),
        Type::INT2 => row.try_get::<_, Option<i16>>(idx)?.map(|v| v.to_string()),
        Type::INT4 => row.try_get::<_, Option<i32>>(idx)?.map(|v| v.to_string()),
        Type::INT8 => row.try_get::<_, Option<i64>>(idx)?.map(|v| v.to_string()),
        Type::FLOAT4 => row.try_get::<_, Option<f32>>(idx)?.map(|v| v.to_string()),
        Type::FLOAT8 => row.try_get::<_, Option<f64>>(idx)?.map(|v| v.to_string()),
        // NUMERIC (M3 fix 2, was the TRACKED RESIDUE on `pg_xsd_code`): hand-decoded
        // via `PgNumeric`'s `FromSql` (`decode_pg_numeric`) — `postgres-types` has no
        // decimal `FromSql` route at all, so this match previously fell to the `_ =>`
        // hard-501 below on ANY NUMERIC column. NaN/±Infinity have no `xsd:decimal`
        // representation, while malformed wire is `Error::Marshal` — but
        // `tokio_postgres::Row::try_get` re-wraps ANY `FromSql` failure as
        // its own `Kind::FromSql` error (`tokio_postgres::Error::from_sql`), so a bare
        // `?` here would flatten straight to the generic `#[from] tokio_postgres::Error`
        // conversion (`Error::Postgres`), silently demoting either classification.
        // The ORIGINAL decoder error survives one more `.source()` hop down
        // (`tokio_postgres::Error`'s `cause`, confirmed against `postgres-types`'
        // `Option<T>::from_sql`, which passes a `Some`-case error through unchanged) —
        // recover it before falling back.
        Type::NUMERIC => row
            .try_get::<_, Option<PgNumeric>>(idx)
            .map_err(recover_pg_from_sql_error)?
            .map(|numeric| numeric.0),
        Type::BYTEA => row.try_get::<_, Option<Vec<u8>>>(idx)?.map(|b| {
            let mut out = std::string::String::new();
            datatype::hex_binary_upper(&b, &mut out);
            out
        }),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::CHAR | Type::UNKNOWN => {
            row.try_get::<_, Option<std::string::String>>(idx)?
        }
        // DATE/TIME/TIMETZ/TIMESTAMP[TZ] (pg_value/pg_xsd_code parity fix): `pg_xsd_code` above
        // has claimed these as Date/Time/DateTime since the adapter's introduction, but
        // this match had no extraction arm for them — any PostgreSQL DATE/TIME/TIMESTAMP
        // column hard-501'd on read (`_ =>` below). Decode via chrono's binary `FromSql`
        // (`with-chrono-0_4`, the standard postgres-types route — no hand-rolled wire
        // parsing) and emit the SAME lexical shapes the other backends produce, so the
        // shared `canonical_lexical`/`natural_literal` reconstruction path (which already
        // handles a space-separated TIMESTAMP via `normalize_timestamp`) parses them
        // identically regardless of source dialect.
        Type::DATE => row
            .try_get::<_, Option<chrono::NaiveDate>>(idx)?
            .map(|d| d.to_string()), // "YYYY-MM-DD"
        Type::TIME => row
            .try_get::<_, Option<chrono::NaiveTime>>(idx)?
            .map(|t| t.to_string()), // "HH:MM:SS[.ffffff]"
        Type::TIMETZ => pg_timetz_value(row, idx)?,
        Type::TIMESTAMP => row
            .try_get::<_, Option<chrono::NaiveDateTime>>(idx)?
            .map(|dt| dt.to_string()), // "YYYY-MM-DD HH:MM:SS[.ffffff]" (space; normalize_timestamp handles it)
        Type::TIMESTAMPTZ => row
            .try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(idx)?
            .map(|dt| dt.to_rfc3339()), // ISO-8601 'T'-separated with a numeric UTC offset
        _ => {
            return Err(Error::Unsupported(format!(
                "PostgreSQL result type {ty} reconstruction"
            )));
        }
    };
    Ok(s)
}

impl<C: Deref<Target = Client>> SqlBackend for PgBackend<C> {
    // PgRowStream owns its portal (design §0 fact 1: `'static`), so it satisfies
    // any `'s` trivially — no driver lifetime crosses the seam.
    type Stream<'s>
        = PgRowStream
    where
        Self: 's;

    async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
        let stmt = self.client.prepare(probe_sql).await?;
        Ok(stmt.columns().iter().map(|c| c.name().to_owned()).collect())
    }

    async fn result_columns(
        &mut self,
        probe_sql: &str,
    ) -> Result<Vec<crate::backend::ResultColumn>> {
        let stmt = self.client.prepare(probe_sql).await?;
        Ok(stmt
            .columns()
            .iter()
            .map(|column| crate::backend::ResultColumn {
                natural_datatype: pg_xsd_code(column.type_()),
                native_scalar: match *column.type_() {
                    Type::INT2 | Type::INT4 | Type::INT8 => Some(super::NativeScalarKey::Integer),
                    Type::BOOL => Some(super::NativeScalarKey::PostgresBoolean),
                    Type::BYTEA => Some(super::NativeScalarKey::PostgresBytea),
                    Type::NUMERIC => Some(super::NativeScalarKey::PostgresNumeric),
                    Type::FLOAT4 => Some(super::NativeScalarKey::PostgresFloat4),
                    Type::FLOAT8 => Some(super::NativeScalarKey::PostgresFloat8),
                    _ => None,
                },
                sqlite_decode: None,
                name: column.name().to_owned(),
                text_key: match *column.type_() {
                    Type::TEXT | Type::VARCHAR => Some(crate::backend::TextKey::Verbatim),
                    Type::BPCHAR => Some(crate::backend::TextKey::PostgresCharacter),
                    _ => None,
                },
            })
            .collect())
    }

    async fn open_branch(&mut self, sql: &str, lexical_params: &[String]) -> Result<PgRowStream> {
        // Each emitted `$n` value is a lexical string, but a FILTER constant may
        // bind against a typed column (INT4/FLOAT8/BOOL/…); `LexicalParam` parses
        // it to the placeholder's inferred PG type at bind time (see its docs).
        let lex: Vec<LexicalParam> = lexical_params.iter().map(|s| LexicalParam(s)).collect();
        let params: Vec<&(dyn ToSql + Sync)> =
            lex.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        // query_raw (server-side portal) — never the buffer-all query() (ADR-0010 §C).
        PgRowStream::open(&self.client, sql, &params).await
    }
}

impl BranchStream for PgRowStream {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        let Some(row) = self.try_next().await? else {
            return Ok(None);
        };
        // The emitted SQL projects exactly `e.projection` columns (each `AS c{i}`),
        // so the row's columns align with it positionally.
        let cols = row.columns();
        let mut values = Vec::with_capacity(cols.len());
        let mut codes = Vec::with_capacity(cols.len());
        for (i, col) in cols.iter().enumerate() {
            let ty = col.type_();
            codes.push(pg_xsd_code(ty));
            values.push(pg_value(&row, i, ty)?);
        }
        Ok(Some(RawTuple { values, codes }))
    }
}

#[cfg(test)]
mod tests;
