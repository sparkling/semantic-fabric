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

use sf_core::datatype::XsdTypeCode;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::Client;

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};
use crate::stream::PgRowStream;

mod decode;
mod timetz;

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
        self.result_columns_controlled(probe_sql, &sf_core::query_control::UncontrolledQueryControl)
            .await
    }

    async fn result_columns_controlled(
        &mut self,
        probe_sql: &str,
        control: &dyn sf_core::query_control::QueryControl,
    ) -> Result<Vec<crate::backend::ResultColumn>> {
        let work = crate::source_work::SourceWork::new(Some(control));
        work.checkpoint()?;
        let stmt = self.client.prepare(probe_sql).await?;
        let mut output = work.vector(stmt.columns().len())?;
        for column in stmt.columns() {
            work.charge(1)?;
            output.push(crate::backend::ResultColumn {
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
                name: work.string(column.name())?,
                text_key: match *column.type_() {
                    Type::TEXT | Type::VARCHAR => Some(crate::backend::TextKey::Verbatim),
                    Type::BPCHAR => Some(crate::backend::TextKey::PostgresCharacter),
                    _ => None,
                },
            });
        }
        work.checkpoint()?;
        Ok(output)
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
        self.next_with_work(crate::source_work::SourceWork::new(None))
            .await
    }

    async fn next_row_controlled(
        &mut self,
        control: &dyn sf_core::query_control::QueryControl,
    ) -> Result<Option<RawTuple>> {
        self.next_with_work(crate::source_work::SourceWork::new(Some(control)))
            .await
    }
}

impl PgRowStream {
    async fn next_with_work(
        &mut self,
        work: crate::source_work::SourceWork<'_>,
    ) -> Result<Option<RawTuple>> {
        work.checkpoint()?;
        let row = self.try_next().await?;
        work.checkpoint()?;
        let Some(row) = row else {
            return Ok(None);
        };
        decode::row(&row, work).map(Some)
    }
}

#[cfg(test)]
mod tests;
