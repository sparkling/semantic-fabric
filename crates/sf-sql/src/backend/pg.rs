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

use std::ops::Deref;

use sf_core::datatype::{self, XsdTypeCode};
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{Client, Row as PgRow};

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};
use crate::stream::PgRowStream;

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
        Type::NUMERIC => Some(Decimal),
        Type::DATE => Some(Date),
        Type::TIME => Some(Time),
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

/// Extract column `idx` of `row` as its raw lexical string (NULL ⇒ `None`),
/// fetched in the most type-faithful driver form (ADR-0015) — integers/floats as
/// their native Rust type, `bytea` uppercase-hex-encoded, booleans as
/// `true`/`false` (never PostgreSQL's `t`/`f`). XSD-canonicalisation of the
/// lexical form is the downstream sf-core chokepoint's concern. A type the
/// reader does not cover surfaces as a hard [`Error::Unsupported`] (turned into a
/// documented `501` skip by the conformance / serve layer).
fn pg_value(row: &PgRow, idx: usize, ty: &Type) -> Result<Option<String>> {
    let s = match *ty {
        Type::BOOL => row.try_get::<_, Option<bool>>(idx)?.map(|b| b.to_string()),
        Type::INT2 => row.try_get::<_, Option<i16>>(idx)?.map(|v| v.to_string()),
        Type::INT4 => row.try_get::<_, Option<i32>>(idx)?.map(|v| v.to_string()),
        Type::INT8 => row.try_get::<_, Option<i64>>(idx)?.map(|v| v.to_string()),
        Type::FLOAT4 => row.try_get::<_, Option<f32>>(idx)?.map(|v| v.to_string()),
        Type::FLOAT8 => row.try_get::<_, Option<f64>>(idx)?.map(|v| v.to_string()),
        Type::BYTEA => row.try_get::<_, Option<Vec<u8>>>(idx)?.map(|b| {
            let mut out = std::string::String::new();
            datatype::hex_binary_upper(&b, &mut out);
            out
        }),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::CHAR | Type::UNKNOWN => {
            row.try_get::<_, Option<std::string::String>>(idx)?
        }
        _ => {
            return Err(Error::Unsupported(format!(
                "PostgreSQL result type {ty} reconstruction"
            )))
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
