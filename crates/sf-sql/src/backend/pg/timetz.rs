//! PostgreSQL `TIMETZ` binary decoding.

use sf_core::datatype::{self, XsdTypeCode};
use tokio_postgres::types::{FromSql, Type};
use tokio_postgres::Row;

use super::recover_pg_from_sql_error;
use crate::error::{Error, Result};
use crate::source_work::SourceWork;

const MICROS_PER_SECOND: i64 = 1_000_000;
const SECONDS_PER_MINUTE: i64 = 60;
const SECONDS_PER_HOUR: i64 = 3_600;
const MICROS_PER_DAY: i64 = 24 * SECONDS_PER_HOUR * MICROS_PER_SECOND;
const PG_MAX_OFFSET_SECONDS: u32 = (16 * SECONDS_PER_HOUR - 1) as u32;
const XSD_MAX_OFFSET_MINUTES: i32 = 14 * 60;

struct PgTimeTz<'a>(&'a [u8]);

impl<'a> FromSql<'a> for PgTimeTz<'a> {
    fn from_sql(
        _ty: &Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(raw))
    }

    fn accepts(ty: &Type) -> bool {
        matches!(*ty, Type::TIMETZ)
    }
}

/// Decode `timetz_send`'s binary payload: microseconds since midnight followed
/// by the fixed offset in seconds *west* of UTC. PostgreSQL permits offsets with
/// seconds and magnitudes up to 15:59:59, while XSD lexical offsets have minute
/// resolution and a ±14:00 bound. The XSD `time` value retains its local fields
/// and timezone offset, so valid PostgreSQL values outside that XSD domain are
/// rejected rather than shifted to a different value.
pub(super) fn decode_pg_timetz(raw: &[u8], work: SourceWork<'_>) -> Result<String> {
    work.charge(raw.len())?;
    if raw.len() != 12 {
        return Err(Error::Marshal(format!(
            "PG TIMETZ: expected 12-byte payload, got {} bytes",
            raw.len()
        )));
    }

    let micros = i64::from_be_bytes(
        raw[..8]
            .try_into()
            .map_err(|_| Error::Marshal("PG TIMETZ: invalid time field".to_owned()))?,
    );
    let seconds_west = i32::from_be_bytes(
        raw[8..]
            .try_into()
            .map_err(|_| Error::Marshal("PG TIMETZ: invalid zone field".to_owned()))?,
    );
    if !(0..=MICROS_PER_DAY).contains(&micros) {
        return Err(Error::Marshal(format!(
            "PG TIMETZ: time field {micros} is outside 00:00:00..24:00:00"
        )));
    }

    if seconds_west.unsigned_abs() > PG_MAX_OFFSET_SECONDS {
        return Err(Error::Marshal(format!(
            "PG TIMETZ: zone field {seconds_west} is outside -15:59:59..+15:59:59"
        )));
    }
    if seconds_west % SECONDS_PER_MINUTE as i32 != 0 {
        return Err(Error::Unsupported(
            "PostgreSQL TIMETZ second-resolution offsets have no xsd:time representation"
                .to_owned(),
        ));
    }

    // PostgreSQL stores seconds west of UTC; XSD writes minutes east of UTC.
    let minutes_east = -(seconds_west / SECONDS_PER_MINUTE as i32);
    if minutes_east.unsigned_abs() > XSD_MAX_OFFSET_MINUTES as u32 {
        return Err(Error::Unsupported(
            "PostgreSQL TIMETZ offsets beyond +/-14:00 have no xsd:time representation".to_owned(),
        ));
    }

    let seconds = micros / MICROS_PER_SECOND;
    let fractional_micros = micros % MICROS_PER_SECOND;
    let hour = seconds / SECONDS_PER_HOUR;
    let minute = seconds / SECONDS_PER_MINUTE % SECONDS_PER_MINUTE;
    let second = seconds % SECONDS_PER_MINUTE;
    // One bounded allowance covers both the intermediate lexical buffer and
    // canonical output produced by the shared datatype formatter.
    work.charge(128)?;
    let mut lexical = String::new();
    lexical
        .try_reserve_exact(32)
        .map_err(|_| Error::Marshal("PostgreSQL TIMETZ allocation failed".into()))?;
    use std::fmt::Write as _;
    write!(&mut lexical, "{hour:02}:{minute:02}:{second:02}")
        .map_err(|_| Error::Marshal("PG TIMETZ: output formatting failed".into()))?;
    if fractional_micros != 0 {
        let fraction = format!("{fractional_micros:06}");
        lexical.push('.');
        lexical.push_str(fraction.trim_end_matches('0'));
    }
    if minutes_east == 0 {
        lexical.push('Z');
    } else {
        let magnitude = minutes_east.unsigned_abs();
        let sign = if minutes_east < 0 { '-' } else { '+' };
        write!(
            &mut lexical,
            "{sign}{:02}:{:02}",
            magnitude / 60,
            magnitude % 60
        )
        .map_err(|_| Error::Marshal("PG TIMETZ: output formatting failed".into()))?;
    }

    // Keep the adapter contract honest at its boundary.
    let mut canonical = String::new();
    datatype::canonical_lexical(&lexical, XsdTypeCode::Time, &mut canonical)
        .map_err(|e| Error::Marshal(format!("PG TIMETZ: invalid decoded xsd:time: {e}")))?;
    work.checkpoint()?;
    Ok(canonical)
}

pub(super) fn value(row: &Row, idx: usize, work: SourceWork<'_>) -> Result<Option<String>> {
    row.try_get::<_, Option<PgTimeTz<'_>>>(idx)
        .map_err(recover_pg_from_sql_error)?
        .map(|value| decode_pg_timetz(value.0, work))
        .transpose()
}
