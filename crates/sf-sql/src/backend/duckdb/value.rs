//! DuckDB Arrow-vector decoding and R2RML lexicalization.

use duckdb::arrow::array::{self, Array, ArrayRef, DictionaryArray, FixedSizeBinaryArray};
use duckdb::arrow::datatypes::{DataType, TimeUnit, UInt16Type, UInt32Type, UInt8Type};
use duckdb::core::LogicalTypeId;
use duckdb::types::{Decimal, EnumType, ValueRef};
use sf_core::datatype::XsdTypeCode;

use crate::error::{Error, Result};

/// Reconstruct the scalar [`ValueRef`] carried by one Arrow result vector.
///
/// This mirrors duckdb-rs' row decoder but is used with its genuinely
/// streaming Arrow result. DuckDB logical metadata disambiguates the shared
/// `Decimal128(38, 0)` carrier used by HUGEINT, UHUGEINT, and DECIMAL.
pub(super) fn duck_arrow_value_ref<'a>(
    column: &'a ArrayRef,
    row: usize,
    logical_type: LogicalTypeId,
) -> Result<ValueRef<'a>> {
    if column.is_null(row) {
        return Ok(ValueRef::Null);
    }
    let value = match column.data_type() {
        DataType::Utf8 => ValueRef::from(
            column
                .as_any()
                .downcast_ref::<array::StringArray>()
                .expect("DuckDB Utf8 vector has StringArray storage")
                .value(row),
        ),
        DataType::LargeUtf8 => ValueRef::from(
            column
                .as_any()
                .downcast_ref::<array::LargeStringArray>()
                .expect("DuckDB LargeUtf8 vector has LargeStringArray storage")
                .value(row),
        ),
        DataType::Binary => ValueRef::Blob(
            column
                .as_any()
                .downcast_ref::<array::BinaryArray>()
                .expect("DuckDB Binary vector has BinaryArray storage")
                .value(row),
        ),
        DataType::LargeBinary => ValueRef::Blob(
            column
                .as_any()
                .downcast_ref::<array::LargeBinaryArray>()
                .expect("DuckDB LargeBinary vector has LargeBinaryArray storage")
                .value(row),
        ),
        DataType::FixedSizeBinary(_) => ValueRef::Blob(
            column
                .as_any()
                .downcast_ref::<FixedSizeBinaryArray>()
                .expect("DuckDB fixed binary vector has FixedSizeBinaryArray storage")
                .value(row),
        ),
        DataType::Boolean => ValueRef::Boolean(
            column
                .as_any()
                .downcast_ref::<array::BooleanArray>()
                .expect("DuckDB Boolean vector has BooleanArray storage")
                .value(row),
        ),
        DataType::Int8 => ValueRef::TinyInt(
            column
                .as_any()
                .downcast_ref::<array::Int8Array>()
                .expect("DuckDB Int8 vector has Int8Array storage")
                .value(row),
        ),
        DataType::Int16 => ValueRef::SmallInt(
            column
                .as_any()
                .downcast_ref::<array::Int16Array>()
                .expect("DuckDB Int16 vector has Int16Array storage")
                .value(row),
        ),
        DataType::Int32 => ValueRef::Int(
            column
                .as_any()
                .downcast_ref::<array::Int32Array>()
                .expect("DuckDB Int32 vector has Int32Array storage")
                .value(row),
        ),
        DataType::Int64 => ValueRef::BigInt(
            column
                .as_any()
                .downcast_ref::<array::Int64Array>()
                .expect("DuckDB Int64 vector has Int64Array storage")
                .value(row),
        ),
        DataType::UInt8 => ValueRef::UTinyInt(
            column
                .as_any()
                .downcast_ref::<array::UInt8Array>()
                .expect("DuckDB UInt8 vector has UInt8Array storage")
                .value(row),
        ),
        DataType::UInt16 => ValueRef::USmallInt(
            column
                .as_any()
                .downcast_ref::<array::UInt16Array>()
                .expect("DuckDB UInt16 vector has UInt16Array storage")
                .value(row),
        ),
        DataType::UInt32 => ValueRef::UInt(
            column
                .as_any()
                .downcast_ref::<array::UInt32Array>()
                .expect("DuckDB UInt32 vector has UInt32Array storage")
                .value(row),
        ),
        DataType::UInt64 => ValueRef::UBigInt(
            column
                .as_any()
                .downcast_ref::<array::UInt64Array>()
                .expect("DuckDB UInt64 vector has UInt64Array storage")
                .value(row),
        ),
        DataType::Float32 => ValueRef::Float(
            column
                .as_any()
                .downcast_ref::<array::Float32Array>()
                .expect("DuckDB Float32 vector has Float32Array storage")
                .value(row),
        ),
        DataType::Float64 => ValueRef::Double(
            column
                .as_any()
                .downcast_ref::<array::Float64Array>()
                .expect("DuckDB Float64 vector has Float64Array storage")
                .value(row),
        ),
        DataType::Decimal32(width, scale) => decimal_value_ref(
            *width,
            *scale,
            i128::from(
                column
                    .as_any()
                    .downcast_ref::<array::Decimal32Array>()
                    .expect("DuckDB Decimal32 vector has Decimal32Array storage")
                    .value(row),
            ),
        )?,
        DataType::Decimal64(width, scale) => decimal_value_ref(
            *width,
            *scale,
            i128::from(
                column
                    .as_any()
                    .downcast_ref::<array::Decimal64Array>()
                    .expect("DuckDB Decimal64 vector has Decimal64Array storage")
                    .value(row),
            ),
        )?,
        DataType::Decimal128(width, scale) => {
            let raw = column
                .as_any()
                .downcast_ref::<array::Decimal128Array>()
                .expect("DuckDB Decimal128 vector has Decimal128Array storage")
                .value(row);
            if *width == Decimal::MAX_WIDTH && *scale == 0 {
                match logical_type {
                    LogicalTypeId::UHugeint => ValueRef::UHugeInt(raw as u128),
                    LogicalTypeId::Decimal => decimal_value_ref(*width, *scale, raw)?,
                    _ => ValueRef::HugeInt(raw),
                }
            } else {
                decimal_value_ref(*width, *scale, raw)?
            }
        }
        DataType::Timestamp(TimeUnit::Second, _) => ValueRef::Timestamp(
            duckdb::types::TimeUnit::Second,
            column
                .as_any()
                .downcast_ref::<array::TimestampSecondArray>()
                .expect("DuckDB second timestamp vector has matching storage")
                .value(row),
        ),
        DataType::Timestamp(TimeUnit::Millisecond, _) => ValueRef::Timestamp(
            duckdb::types::TimeUnit::Millisecond,
            column
                .as_any()
                .downcast_ref::<array::TimestampMillisecondArray>()
                .expect("DuckDB millisecond timestamp vector has matching storage")
                .value(row),
        ),
        DataType::Timestamp(TimeUnit::Microsecond, _) => ValueRef::Timestamp(
            duckdb::types::TimeUnit::Microsecond,
            column
                .as_any()
                .downcast_ref::<array::TimestampMicrosecondArray>()
                .expect("DuckDB microsecond timestamp vector has matching storage")
                .value(row),
        ),
        DataType::Timestamp(TimeUnit::Nanosecond, _) => ValueRef::Timestamp(
            duckdb::types::TimeUnit::Nanosecond,
            column
                .as_any()
                .downcast_ref::<array::TimestampNanosecondArray>()
                .expect("DuckDB nanosecond timestamp vector has matching storage")
                .value(row),
        ),
        DataType::Date32 => ValueRef::Date32(
            column
                .as_any()
                .downcast_ref::<array::Date32Array>()
                .expect("DuckDB Date32 vector has Date32Array storage")
                .value(row),
        ),
        DataType::Time64(TimeUnit::Microsecond) => ValueRef::Time64(
            duckdb::types::TimeUnit::Microsecond,
            column
                .as_any()
                .downcast_ref::<array::Time64MicrosecondArray>()
                .expect("DuckDB time vector has Time64MicrosecondArray storage")
                .value(row),
        ),
        DataType::Dictionary(key_type, _) => ValueRef::Enum(
            match key_type.as_ref() {
                DataType::UInt8 => EnumType::UInt8(
                    column
                        .as_any()
                        .downcast_ref::<DictionaryArray<UInt8Type>>()
                        .expect("DuckDB UInt8 enum has matching dictionary storage"),
                ),
                DataType::UInt16 => EnumType::UInt16(
                    column
                        .as_any()
                        .downcast_ref::<DictionaryArray<UInt16Type>>()
                        .expect("DuckDB UInt16 enum has matching dictionary storage"),
                ),
                DataType::UInt32 => EnumType::UInt32(
                    column
                        .as_any()
                        .downcast_ref::<DictionaryArray<UInt32Type>>()
                        .expect("DuckDB UInt32 enum has matching dictionary storage"),
                ),
                other => {
                    return Err(Error::Unsupported(format!(
                        "DuckDB enum key type {other:?} not supported"
                    )));
                }
            },
            row,
        ),
        other => {
            return Err(Error::Unsupported(format!(
                "DuckDB value type {other:?} not supported"
            )));
        }
    };
    Ok(match (logical_type, value) {
        (LogicalTypeId::Geometry, ValueRef::Blob(bytes)) => ValueRef::Geometry(bytes),
        (_, value) => value,
    })
}

fn decimal_value_ref(width: u8, scale: i8, value: i128) -> Result<ValueRef<'static>> {
    let scale = u8::try_from(scale).map_err(|_| {
        Error::Marshal(format!(
            "DuckDB decimal has unsupported negative scale {scale}"
        ))
    })?;
    let decimal = Decimal::new(width, scale, value)
        .map_err(|error| Error::Marshal(format!("invalid DuckDB decimal value: {error}")))?;
    Ok(ValueRef::Decimal(decimal))
}

/// Map a DuckDB [`ValueRef`] to a lexical string and XSD type code.
///
/// SQL scalar types with an R2RML §10 natural mapping are converted to their
/// lexical representation and XSD code. Nested types and intervals remain
/// unsupported and surface as a 501 via `exec_core::map_sql_err`.
pub(super) fn duck_value(
    v: ValueRef<'_>,
    timestamp_with_timezone: bool,
    max_value_bytes: Option<usize>,
) -> Result<(Option<String>, Option<XsdTypeCode>)> {
    use XsdTypeCode as X;
    match v {
        ValueRef::Null => Ok((None, None)),
        ValueRef::Boolean(b) => Ok((Some(b.to_string()), Some(X::Boolean))),
        ValueRef::TinyInt(i) => Ok((Some(i.to_string()), Some(X::Integer))),
        ValueRef::SmallInt(i) => Ok((Some(i.to_string()), Some(X::Integer))),
        ValueRef::Int(i) => Ok((Some(i.to_string()), Some(X::Integer))),
        ValueRef::BigInt(i) => Ok((Some(i.to_string()), Some(X::Integer))),
        ValueRef::HugeInt(i) => Ok((Some(i.to_string()), Some(X::Integer))),
        ValueRef::UHugeInt(i) => Ok((Some(i.to_string()), Some(X::Integer))),
        ValueRef::UTinyInt(u) => Ok((Some(u.to_string()), Some(X::Integer))),
        ValueRef::USmallInt(u) => Ok((Some(u.to_string()), Some(X::Integer))),
        ValueRef::UInt(u) => Ok((Some(u.to_string()), Some(X::Integer))),
        ValueRef::UBigInt(u) => Ok((Some(u.to_string()), Some(X::Integer))),
        ValueRef::Float(f) => Ok((Some(f.to_string()), Some(X::Double))),
        ValueRef::Double(d) => Ok((Some(d.to_string()), Some(X::Double))),
        ValueRef::Decimal(d) => Ok((Some(d.to_string()), Some(X::Decimal))),
        ValueRef::Date32(days) => {
            let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1)
                .expect("the Unix epoch is a valid date");
            let date = epoch
                .checked_add_signed(chrono::TimeDelta::days(i64::from(days)))
                .ok_or_else(|| Error::Marshal(format!("DuckDB date out of range: {days}")))?;
            Ok((Some(date.to_string()), Some(X::Date)))
        }
        ValueRef::Time64(unit, value) => {
            let (seconds, nanos) = split_time_unit(unit, value);
            let seconds = u32::try_from(seconds).map_err(|_| {
                Error::Marshal(format!("DuckDB time out of range: {value} {unit:?}"))
            })?;
            let time = chrono::NaiveTime::from_num_seconds_from_midnight_opt(seconds, nanos)
                .ok_or_else(|| {
                    Error::Marshal(format!("DuckDB time out of range: {value} {unit:?}"))
                })?;
            Ok((Some(time.to_string()), Some(X::Time)))
        }
        ValueRef::Timestamp(unit, value) => {
            let (seconds, nanos) = split_time_unit(unit, value);
            let timestamp = chrono::DateTime::from_timestamp(seconds, nanos).ok_or_else(|| {
                Error::Marshal(format!("DuckDB timestamp out of range: {value} {unit:?}"))
            })?;
            let lexical = if timestamp_with_timezone {
                timestamp.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
            } else {
                timestamp.naive_utc().to_string()
            };
            Ok((Some(lexical), Some(X::DateTime)))
        }
        ValueRef::Text(t) => {
            ensure_value_size("text", t.len(), max_value_bytes)?;
            let s = std::str::from_utf8(t)
                .map_err(|e| Error::Marshal(format!("duckdb non-UTF8 text: {e}")))?;
            Ok((Some(s.to_owned()), Some(X::String)))
        }
        ValueRef::Blob(b) => {
            let encoded_len = b
                .len()
                .checked_mul(2)
                .ok_or_else(|| Error::Marshal("DuckDB blob lexical size overflow".to_owned()))?;
            ensure_value_size("blob", encoded_len, max_value_bytes)?;
            let mut out = String::new();
            sf_core::datatype::hex_binary_upper(b, &mut out);
            Ok((Some(out), Some(X::HexBinary)))
        }
        ValueRef::Enum(..) => {
            let value = v
                .as_str()
                .map_err(|error| Error::Marshal(format!("duckdb enum: {error}")))?;
            ensure_value_size("enum", value.len(), max_value_bytes)?;
            Ok((Some(value.to_owned()), Some(X::String)))
        }
        // Interval, List, Struct, Union, Array, Map
        other => Err(Error::Unsupported(format!(
            "DuckDB value type {:?} not supported",
            other.data_type()
        ))),
    }
}

fn ensure_value_size(kind: &str, bytes: usize, maximum: Option<usize>) -> Result<()> {
    if maximum.is_some_and(|maximum| bytes > maximum) {
        return Err(Error::Marshal(format!(
            "DuckDB {kind} value requires {bytes} lexical bytes, exceeding the configured per-value limit of {} bytes",
            maximum.expect("maximum checked above")
        )));
    }
    Ok(())
}

fn split_time_unit(unit: duckdb::types::TimeUnit, value: i64) -> (i64, u32) {
    let (per_second, nanos_per_unit) = match unit {
        duckdb::types::TimeUnit::Second => (1, 0),
        duckdb::types::TimeUnit::Millisecond => (1_000, 1_000_000),
        duckdb::types::TimeUnit::Microsecond => (1_000_000, 1_000),
        duckdb::types::TimeUnit::Nanosecond => (1_000_000_000, 1),
    };
    let seconds = value.div_euclid(per_second);
    let nanos = value.rem_euclid(per_second) * nanos_per_unit;
    (seconds, nanos as u32)
}
