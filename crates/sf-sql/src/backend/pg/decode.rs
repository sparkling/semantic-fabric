//! Prospective application-side PostgreSQL row decoding; native driver storage excluded.

use std::fmt::Write as _;

use sf_core::datatype;
use tokio_postgres::types::{FromSql, Type};
use tokio_postgres::Row as PgRow;

use super::{pg_xsd_code, recover_pg_from_sql_error};
use crate::backend::RawTuple;
use crate::error::{Error, Result};
use crate::source_work::SourceWork;

struct PgNumeric<'a>(&'a [u8]);

impl<'a> FromSql<'a> for PgNumeric<'a> {
    fn from_sql(
        _ty: &Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(raw))
    }

    fn accepts(ty: &Type) -> bool {
        matches!(*ty, Type::NUMERIC)
    }
}

struct PgText<'a>(&'a [u8]);

impl<'a> FromSql<'a> for PgText<'a> {
    fn from_sql(
        _ty: &Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(raw))
    }

    fn accepts(ty: &Type) -> bool {
        matches!(
            *ty,
            Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::CHAR | Type::UNKNOWN
        )
    }
}

pub(super) fn row(row: &PgRow, work: SourceWork<'_>) -> Result<RawTuple> {
    work.checkpoint()?;
    let columns = row.columns();
    let mut values = work.vector(columns.len())?;
    let mut codes = work.vector(columns.len())?;
    for (index, column) in columns.iter().enumerate() {
        work.charge(1)?;
        let ty = column.type_();
        codes.push(pg_xsd_code(ty));
        values.push(value(row, index, ty, work)?);
    }
    work.checkpoint()?;
    Ok(RawTuple { values, codes })
}

fn scalar<T: ToString>(
    value: Option<T>,
    lexical_bound: usize,
    work: SourceWork<'_>,
) -> Result<Option<String>> {
    work.charge(lexical_bound)?;
    let result = value.map(|value| value.to_string());
    work.checkpoint()?;
    Ok(result)
}

fn value(row: &PgRow, index: usize, ty: &Type, work: SourceWork<'_>) -> Result<Option<String>> {
    work.charge(1)?;
    match *ty {
        Type::BOOL => scalar(row.try_get::<_, Option<bool>>(index)?, 5, work),
        Type::INT2 => scalar(row.try_get::<_, Option<i16>>(index)?, 6, work),
        Type::INT4 => scalar(row.try_get::<_, Option<i32>>(index)?, 11, work),
        Type::INT8 => scalar(row.try_get::<_, Option<i64>>(index)?, 20, work),
        Type::FLOAT4 => scalar(row.try_get::<_, Option<f32>>(index)?, 64, work),
        Type::FLOAT8 => scalar(row.try_get::<_, Option<f64>>(index)?, 327, work),
        Type::NUMERIC => row
            .try_get::<_, Option<PgNumeric<'_>>>(index)
            .map_err(recover_pg_from_sql_error)?
            .map(|numeric| decode_numeric(numeric.0, work))
            .transpose(),
        Type::BYTEA => row
            .try_get::<_, Option<&[u8]>>(index)?
            .map(|bytes| {
                work.product(bytes.len(), 3)?;
                let capacity = bytes
                    .len()
                    .checked_mul(2)
                    .ok_or_else(|| Error::Marshal("PostgreSQL BYTEA length overflow".into()))?;
                let mut output = String::new();
                output
                    .try_reserve_exact(capacity)
                    .map_err(|_| Error::Marshal("PostgreSQL decoding allocation failed".into()))?;
                datatype::hex_binary_upper(bytes, &mut output);
                work.checkpoint()?;
                Ok(output)
            })
            .transpose(),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::CHAR | Type::UNKNOWN => {
            let Some(raw) = row.try_get::<_, Option<PgText<'_>>>(index)? else {
                return Ok(None);
            };
            work.product(raw.0.len(), 2)?;
            let value = row.try_get::<_, String>(index)?;
            work.checkpoint()?;
            Ok(Some(value))
        }
        Type::DATE => scalar(
            row.try_get::<_, Option<chrono::NaiveDate>>(index)?,
            16,
            work,
        ),
        Type::TIME => scalar(
            row.try_get::<_, Option<chrono::NaiveTime>>(index)?,
            32,
            work,
        ),
        Type::TIMETZ => super::timetz::value(row, index, work),
        Type::TIMESTAMP => scalar(
            row.try_get::<_, Option<chrono::NaiveDateTime>>(index)?,
            40,
            work,
        ),
        Type::TIMESTAMPTZ => {
            work.charge(64)?;
            let result = row
                .try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(index)?
                .map(|value| value.to_rfc3339());
            work.checkpoint()?;
            Ok(result)
        }
        _ => Err(Error::Unsupported(format!(
            "PostgreSQL result type {ty} reconstruction"
        ))),
    }
}

pub(super) fn decode_numeric(raw: &[u8], work: SourceWork<'_>) -> Result<String> {
    const POS: u16 = 0x0000;
    const NEG: u16 = 0x4000;
    const NAN: u16 = 0xC000;
    const PINF: u16 = 0xD000;
    const NINF: u16 = 0xF000;

    work.charge(8)?;
    if raw.len() < 8 {
        return Err(Error::Marshal(format!(
            "PG NUMERIC: header too short ({} bytes)",
            raw.len()
        )));
    }
    let ndigits = usize::from(u16::from_be_bytes([raw[0], raw[1]]));
    let weight = i16::from_be_bytes([raw[2], raw[3]]);
    let sign = u16::from_be_bytes([raw[4], raw[5]]);
    let dscale = u16::from_be_bytes([raw[6], raw[7]]);
    match sign {
        NAN => {
            return Err(Error::Unsupported(
                "PostgreSQL NUMERIC NaN has no xsd:decimal representation".into(),
            ))
        }
        PINF => {
            return Err(Error::Unsupported(
                "PostgreSQL NUMERIC +Infinity has no xsd:decimal representation".into(),
            ))
        }
        NINF => {
            return Err(Error::Unsupported(
                "PostgreSQL NUMERIC -Infinity has no xsd:decimal representation".into(),
            ))
        }
        POS | NEG => {}
        other => {
            return Err(Error::Marshal(format!(
                "PG NUMERIC: unrecognised sign 0x{other:04X}"
            )))
        }
    }
    let digit_bytes = ndigits
        .checked_mul(2)
        .ok_or_else(|| Error::Marshal("PG NUMERIC: digit length overflow".into()))?;
    let required = 8usize
        .checked_add(digit_bytes)
        .ok_or_else(|| Error::Marshal("PG NUMERIC: digit length overflow".into()))?;
    if raw.len() < required {
        return Err(Error::Marshal(format!(
            "PG NUMERIC: digit array truncated (need {required} bytes, have {})",
            raw.len()
        )));
    }
    for chunk in raw[8..required].chunks(2048) {
        work.charge(chunk.len())?;
    }

    let integer_groups = if weight < 0 {
        0usize
    } else {
        usize::try_from(i32::from(weight) + 1)
            .map_err(|_| Error::Marshal("PG NUMERIC: invalid weight".into()))?
    };
    let fractional_groups = usize::from(dscale).div_ceil(4);
    let output_capacity = usize::from(sign == NEG)
        .checked_add(if integer_groups == 0 {
            1
        } else {
            integer_groups * 4
        })
        .and_then(|length| length.checked_add(usize::from(dscale > 0)))
        .and_then(|length| length.checked_add(usize::from(dscale)))
        .ok_or_else(|| Error::Marshal("PG NUMERIC: output length overflow".into()))?;
    work.charge(output_capacity)?;
    let mut output = String::new();
    output
        .try_reserve_exact(output_capacity)
        .map_err(|_| Error::Marshal("PostgreSQL decoding allocation failed".into()))?;

    let digit_at = |position: i32| -> i16 {
        let offset = i32::from(weight) - position;
        if offset < 0 || offset as usize >= ndigits {
            return 0;
        }
        let start = 8 + offset as usize * 2;
        i16::from_be_bytes([raw[start], raw[start + 1]])
    };
    if sign == NEG {
        output.push('-');
    }
    if integer_groups == 0 {
        output.push('0');
    } else {
        for position in (0..=i32::from(weight)).rev() {
            work.charge(1)?;
            let digit = digit_at(position);
            if position == i32::from(weight) {
                write!(&mut output, "{digit}")
            } else {
                write!(&mut output, "{digit:04}")
            }
            .map_err(|_| Error::Marshal("PG NUMERIC: output formatting failed".into()))?;
        }
    }
    if dscale > 0 {
        output.push('.');
        for group in 0..fractional_groups {
            work.charge(1)?;
            let digit = digit_at(-1 - group as i32) as u16;
            let encoded = [
                b'0' + ((digit / 1000) % 10) as u8,
                b'0' + ((digit / 100) % 10) as u8,
                b'0' + ((digit / 10) % 10) as u8,
                b'0' + (digit % 10) as u8,
            ];
            let remaining = usize::from(dscale) - group * 4;
            output.push_str(
                std::str::from_utf8(&encoded[..remaining.min(4)])
                    .map_err(|_| Error::Marshal("PG NUMERIC: output formatting failed".into()))?,
            );
        }
    }
    work.checkpoint()?;
    Ok(output)
}

#[cfg(test)]
#[path = "decode_tests.rs"]
mod tests;
