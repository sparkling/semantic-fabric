//! MySQL `SqlBackend` adapter (ADR-0024 §2, §4.2). The ONE driver whose pull cursor
//! borrows the handle: `Stream<'s>` is a native `mysql_async::QueryResult` borrowing
//! `&'s mut Conn` (the reason the GAT exists — design §0 fact 3). Built on
//! `exec_iter` + `row.take::<Value>` + `mysql_value_to_string` (moved VERBATIM from
//! the old `sf-sparql::exec_mysql` loop) — NEVER `String::from_utf8_lossy` over the
//! raw row bytes: silently masking an encoding error behind a replacement char is a
//! `=_bag` / 3-valued-logic regression (design A1). `row.take::<Value>` +
//! `mysql_value_to_string` is the one strict decode this adapter uses.
//! `mysql_async` has no server-side cursor, so this is client-buffer-free /
//! packet-bounded, not cursor-grade (design §4 / §4.2).

use std::borrow::BorrowMut;

use mysql_async::consts::ColumnType;
use mysql_async::prelude::Queryable;
use mysql_async::{BinaryProtocol, Column, Conn, Params, QueryResult, Value};
use sf_core::datatype::{self, XsdTypeCode};

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

/// A MySQL backend over any holder that yields `&mut Conn`. Generic over `C` so the
/// same adapter serves both lanes (mirroring `PgBackend<C>`):
///   * `MysqlBackend<&mut Conn>` — the borrowing collecting path (`select_mysql`/…).
///   * `MysqlBackend<Conn>` — the **owned, `'static`** serve lane (`select_each_mysql`
///     / `construct_each_mysql`), whose future is `tokio::spawn`ed onto a DEDICATED
///     pooled connection (design §4.2).
pub struct MysqlBackend<C> {
    conn: C,
}

impl<C: BorrowMut<Conn>> MysqlBackend<C> {
    pub fn new(conn: C) -> Self {
        Self { conn }
    }
}

/// A borrowing MySQL branch cursor: the native `QueryResult` streamed one row at a
/// time (no client-side `Vec<Row>`), marshalled to a [`RawTuple`] per `next_row`.
pub struct MysqlBranch<'s> {
    result: QueryResult<'s, 'static, BinaryProtocol>,
    codes: Vec<Option<XsdTypeCode>>,
}

impl BranchStream for MysqlBranch<'_> {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        let Some(mut row) = self.result.next().await? else {
            return Ok(None);
        };
        let ncols = row.len();
        let mut values = Vec::with_capacity(ncols);
        for i in 0..ncols {
            // exec_mysql.rs:146 VERBATIM.
            let v: Value = row.take(i).unwrap_or(Value::NULL);
            values.push(mysql_value_to_string(v, self.codes[i])?);
        }
        let codes = self.codes.clone();
        Ok(Some(RawTuple { values, codes }))
    }
}

impl<C: BorrowMut<Conn>> SqlBackend for MysqlBackend<C> {
    type Stream<'s>
        = MysqlBranch<'s>
    where
        Self: 's;

    async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
        crate::stream::mysql_column_names(self.conn.borrow_mut(), probe_sql).await
    }

    async fn open_branch<'s>(
        &'s mut self,
        sql: &str,
        lexical_params: &[String],
    ) -> Result<MysqlBranch<'s>> {
        // Bind each lexical value positionally (exec_mysql.rs:183 verbatim).
        let params: Vec<Value> = lexical_params
            .iter()
            .map(|s| Value::from(s.as_str()))
            .collect();
        let conn = self.conn.borrow_mut(); // &'s mut Conn
        let stmt = conn.prep(sql).await?;
        let codes = stmt.columns().iter().map(mysql_xsd_code).collect();
        // exec_iter (packet-streamed) — NOT exec() (buffer-all Vec<Row>).
        let result = conn.exec_iter(stmt, Params::Positional(params)).await?;
        Ok(MysqlBranch { result, codes })
    }
}

/// Convert a single MySQL [`Value`] cell to a raw lexical [`String`] (NULL → `None`).
/// All wire types are converted via their natural Rust representation and then
/// formatted as strings — the same principle as the PostgreSQL text-protocol path.
///
/// Prepared-statement metadata supplies the natural XSD code. Binary values are
/// uppercase-hex encoded only for a binary column; invalid UTF-8 in a text column
/// is a hard marshalling error. The same metadata distinguishes DATE from a
/// DATETIME/TIMESTAMP whose value happens to be midnight.
fn mysql_value_to_string(v: Value, code: Option<XsdTypeCode>) -> Result<Option<String>> {
    use mysql_async::Value::*;
    Ok(match v {
        NULL => None,
        Bytes(bytes) if code == Some(XsdTypeCode::HexBinary) => {
            let mut encoded = String::new();
            datatype::hex_binary_upper(&bytes, &mut encoded);
            Some(encoded)
        }
        Bytes(bytes) => Some(
            String::from_utf8(bytes)
                .map_err(|error| Error::Marshal(format!("non-UTF8 MySQL text column: {error}")))?,
        ),
        Int(i) => Some(i.to_string()),
        UInt(u) => Some(u.to_string()),
        Float(f) => Some(f.to_string()),
        Double(d) => Some(d.to_string()),
        Date(y, mo, d, h, mi, s, us) => {
            if code == Some(XsdTypeCode::Date) {
                Some(format!("{y:04}-{mo:02}-{d:02}"))
            } else if us == 0 {
                Some(format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}"))
            } else {
                Some(format!(
                    "{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{us:06}"
                ))
            }
        }
        Time(neg, days, h, mi, s, us) => {
            let sign = if neg { "-" } else { "" };
            let total_h = days * 24 + u32::from(h);
            if us == 0 {
                Some(format!("{sign}{total_h:02}:{mi:02}:{s:02}"))
            } else {
                Some(format!("{sign}{total_h:02}:{mi:02}:{s:02}.{us:06}"))
            }
        }
    })
}

fn mysql_xsd_code(column: &Column) -> Option<XsdTypeCode> {
    use ColumnType::*;
    match column.column_type() {
        MYSQL_TYPE_TINY if column.column_length() == 1 => Some(XsdTypeCode::Boolean),
        MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT | MYSQL_TYPE_LONG | MYSQL_TYPE_LONGLONG
        | MYSQL_TYPE_INT24 | MYSQL_TYPE_YEAR => Some(XsdTypeCode::Integer),
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => Some(XsdTypeCode::Decimal),
        MYSQL_TYPE_FLOAT | MYSQL_TYPE_DOUBLE => Some(XsdTypeCode::Double),
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => Some(XsdTypeCode::Date),
        MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => Some(XsdTypeCode::Time),
        MYSQL_TYPE_TIMESTAMP
        | MYSQL_TYPE_TIMESTAMP2
        | MYSQL_TYPE_DATETIME
        | MYSQL_TYPE_DATETIME2 => Some(XsdTypeCode::DateTime),
        MYSQL_TYPE_BIT => Some(XsdTypeCode::HexBinary),
        MYSQL_TYPE_VARCHAR
        | MYSQL_TYPE_VAR_STRING
        | MYSQL_TYPE_STRING
        | MYSQL_TYPE_TINY_BLOB
        | MYSQL_TYPE_MEDIUM_BLOB
        | MYSQL_TYPE_LONG_BLOB
        | MYSQL_TYPE_BLOB
            if column.character_set() == 63 =>
        {
            Some(XsdTypeCode::HexBinary)
        }
        MYSQL_TYPE_VARCHAR
        | MYSQL_TYPE_VAR_STRING
        | MYSQL_TYPE_STRING
        | MYSQL_TYPE_TINY_BLOB
        | MYSQL_TYPE_MEDIUM_BLOB
        | MYSQL_TYPE_LONG_BLOB
        | MYSQL_TYPE_BLOB
        | MYSQL_TYPE_ENUM
        | MYSQL_TYPE_SET
        | MYSQL_TYPE_JSON => Some(XsdTypeCode::String),
        MYSQL_TYPE_NULL
        | MYSQL_TYPE_TYPED_ARRAY
        | MYSQL_TYPE_VECTOR
        | MYSQL_TYPE_UNKNOWN
        | MYSQL_TYPE_GEOMETRY => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mysql_async::Value;

    fn lexical(value: Value, code: Option<XsdTypeCode>) -> Option<String> {
        mysql_value_to_string(value, code).expect("value marshals")
    }

    #[test]
    fn null_maps_to_none() {
        assert_eq!(lexical(Value::NULL, None), None);
    }

    #[test]
    fn utf8_bytes_pass_through() {
        assert_eq!(
            lexical(Value::Bytes(b"hello".to_vec()), Some(XsdTypeCode::String)),
            Some("hello".to_owned())
        );
    }

    #[test]
    fn binary_bytes_are_hex_encoded_and_text_bytes_remain_strict() {
        assert_eq!(
            lexical(Value::Bytes(vec![0xff, 0xfe]), Some(XsdTypeCode::HexBinary)),
            Some("FFFE".to_owned())
        );
        let error =
            mysql_value_to_string(Value::Bytes(vec![0xff, 0xfe]), Some(XsdTypeCode::String))
                .unwrap_err();
        assert!(matches!(error, Error::Marshal(_)), "{error:?}");
    }

    #[test]
    fn integer_and_float_variants_render_via_to_string() {
        assert_eq!(
            lexical(Value::Int(-42), Some(XsdTypeCode::Integer)),
            Some("-42".to_owned())
        );
        assert_eq!(
            lexical(Value::UInt(42), Some(XsdTypeCode::Integer)),
            Some("42".to_owned())
        );
        assert_eq!(
            lexical(Value::Float(1.5), Some(XsdTypeCode::Double)),
            Some("1.5".to_owned())
        );
        assert_eq!(
            lexical(Value::Double(2.5), Some(XsdTypeCode::Double)),
            Some("2.5".to_owned())
        );
    }

    #[test]
    fn date_with_zero_time_renders_as_bare_date() {
        assert_eq!(
            lexical(
                Value::Date(2024, 3, 15, 0, 0, 0, 0),
                Some(XsdTypeCode::Date)
            ),
            Some("2024-03-15".to_owned())
        );
    }

    #[test]
    fn metadata_distinguishes_date_from_midnight_datetime() {
        let value = Value::Date(2024, 3, 15, 0, 0, 0, 0);
        let date_only = lexical(value.clone(), Some(XsdTypeCode::Date));
        let midnight_datetime = lexical(value, Some(XsdTypeCode::DateTime));
        assert_eq!(date_only, Some("2024-03-15".to_owned()));
        assert_eq!(midnight_datetime, Some("2024-03-15 00:00:00".to_owned()));
    }

    #[test]
    fn date_with_time_no_micros_renders_iso_t_separated() {
        assert_eq!(
            lexical(
                Value::Date(2024, 3, 15, 13, 45, 30, 0),
                Some(XsdTypeCode::DateTime)
            ),
            Some("2024-03-15 13:45:30".to_owned())
        );
    }

    #[test]
    fn date_with_microseconds_renders_fractional_seconds() {
        assert_eq!(
            lexical(
                Value::Date(2024, 3, 15, 13, 45, 30, 123456),
                Some(XsdTypeCode::DateTime)
            ),
            Some("2024-03-15 13:45:30.123456".to_owned())
        );
    }

    #[test]
    fn time_zero_days_no_micros() {
        assert_eq!(
            lexical(
                Value::Time(false, 0, 13, 45, 30, 0),
                Some(XsdTypeCode::Time)
            ),
            Some("13:45:30".to_owned())
        );
    }

    #[test]
    fn time_negative_renders_leading_minus() {
        assert_eq!(
            lexical(Value::Time(true, 0, 13, 45, 30, 0), Some(XsdTypeCode::Time)),
            Some("-13:45:30".to_owned())
        );
    }

    #[test]
    fn time_days_component_folds_into_total_hours() {
        // MySQL TIME can exceed 24h (elapsed-time semantics); `days` folds into
        // the hour count rather than being dropped or rendered separately.
        assert_eq!(
            lexical(Value::Time(false, 2, 3, 0, 0, 0), Some(XsdTypeCode::Time)),
            Some("51:00:00".to_owned()) // 2*24 + 3 = 51
        );
    }

    #[test]
    fn time_with_microseconds_renders_fractional_seconds() {
        assert_eq!(
            lexical(
                Value::Time(false, 0, 13, 45, 30, 500000),
                Some(XsdTypeCode::Time)
            ),
            Some("13:45:30.500000".to_owned())
        );
    }

    #[test]
    fn prepared_column_metadata_maps_mysql_natural_types() {
        assert_eq!(
            mysql_xsd_code(&Column::new(ColumnType::MYSQL_TYPE_LONG)),
            Some(XsdTypeCode::Integer)
        );
        assert_eq!(
            mysql_xsd_code(&Column::new(ColumnType::MYSQL_TYPE_TINY).with_column_length(1)),
            Some(XsdTypeCode::Boolean)
        );
        assert_eq!(
            mysql_xsd_code(&Column::new(ColumnType::MYSQL_TYPE_VAR_STRING).with_character_set(63)),
            Some(XsdTypeCode::HexBinary)
        );
        assert_eq!(
            mysql_xsd_code(&Column::new(ColumnType::MYSQL_TYPE_VAR_STRING)),
            Some(XsdTypeCode::String)
        );
        assert_eq!(
            mysql_xsd_code(&Column::new(ColumnType::MYSQL_TYPE_DATETIME)),
            Some(XsdTypeCode::DateTime)
        );
    }
}
