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

/// How an otherwise ambiguous MySQL type identity is interpreted.
///
/// MySQL reports both authored `BOOL` and authored `TINYINT(1)` as the same
/// catalogue/wire type. Native product execution therefore treats it as an
/// integer. Only the sealed W3C SQL-2008 fixture runner may opt into the
/// compatibility convention that the width-one type originated as `BOOLEAN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MysqlTypeProfile {
    Native,
    W3cSql2008,
}

impl MysqlTypeProfile {
    pub const fn evidence_name(self) -> &'static str {
        match self {
            Self::Native => "mysql-native-v1",
            Self::W3cSql2008 => "mysql-w3c-sql-2008-v1",
        }
    }
}

/// MySQL catalogue spelling to the natural XSD type used by row execution.
///
/// This law is deliberately outside `sf_core::datatype::natural_xsd`: MySQL
/// aliases and modifiers must not change PostgreSQL or SQLite semantics.
pub fn mysql_natural_xsd(sql_type: &str, profile: MysqlTypeProfile) -> Option<XsdTypeCode> {
    use XsdTypeCode::*;
    if mysql_tinyint_one(sql_type) {
        return Some(match profile {
            MysqlTypeProfile::Native => Integer,
            MysqlTypeProfile::W3cSql2008 => Boolean,
        });
    }
    let normalized = normalize_mysql_type(sql_type);
    match normalized.as_str() {
        "BOOL" | "BOOLEAN" => Some(match profile {
            MysqlTypeProfile::Native => Integer,
            MysqlTypeProfile::W3cSql2008 => Boolean,
        }),
        "TINYINT" | "MEDIUMINT" | "YEAR" => Some(Integer),
        "DATETIME" => Some(DateTime),
        "BIT" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" => Some(HexBinary),
        "TINYTEXT" | "MEDIUMTEXT" | "LONGTEXT" | "ENUM" | "SET" | "JSON" => Some(String),
        _ => datatype::natural_xsd(&normalized),
    }
}

fn normalize_mysql_type(sql_type: &str) -> String {
    let base = sql_type.split('(').next().unwrap_or(sql_type);
    let mut normalized = String::new();
    for word in base.split_whitespace().filter(|word| {
        !word.eq_ignore_ascii_case("UNSIGNED") && !word.eq_ignore_ascii_case("ZEROFILL")
    }) {
        if !normalized.is_empty() {
            normalized.push(' ');
        }
        normalized.extend(word.chars().flat_map(char::to_uppercase));
    }
    normalized
}

fn mysql_tinyint_one(sql_type: &str) -> bool {
    let compact: String = sql_type
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .flat_map(char::to_uppercase)
        .collect();
    matches!(
        compact.as_str(),
        "TINYINT(1)" | "TINYINT(1)UNSIGNED" | "TINYINT(1)ZEROFILL" | "TINYINT(1)UNSIGNEDZEROFILL"
    )
}

/// A MySQL backend over any holder that yields `&mut Conn`. Generic over `C` so the
/// same adapter serves both lanes (mirroring `PgBackend<C>`):
///   * `MysqlBackend<&mut Conn>` — the borrowing collecting path (`select_mysql`/…).
///   * `MysqlBackend<Conn>` — the **owned, `'static`** serve lane (`select_each_mysql`
///     / `construct_each_mysql`), whose future is `tokio::spawn`ed onto a DEDICATED
///     pooled connection (design §4.2).
pub struct MysqlBackend<C> {
    conn: C,
    type_profile: MysqlTypeProfile,
}

impl<C: BorrowMut<Conn>> MysqlBackend<C> {
    /// Return the owned connection holder after the executor releases its cursor.
    /// Serving uses this to acknowledge cleanup before permitting pool reuse.
    pub fn into_inner(self) -> C {
        self.conn
    }

    pub fn new(conn: C) -> Self {
        Self {
            conn,
            type_profile: MysqlTypeProfile::Native,
        }
    }

    pub fn with_type_profile(conn: C, type_profile: MysqlTypeProfile) -> Self {
        Self { conn, type_profile }
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
        ensure_row_arity(ncols, self.codes.len())?;
        let mut values = Vec::with_capacity(ncols);
        for (i, code) in self.codes.iter().copied().enumerate() {
            // exec_mysql.rs:146 VERBATIM.
            let v: Value = row.take(i).ok_or_else(|| {
                Error::Marshal("MySQL result row column is unavailable".to_owned())
            })?;
            values.push(mysql_value_to_string(v, code)?);
        }
        let codes = self.codes.clone();
        Ok(Some(RawTuple { values, codes }))
    }
}

fn ensure_row_arity(row_columns: usize, metadata_columns: usize) -> Result<()> {
    if row_columns == metadata_columns {
        Ok(())
    } else {
        Err(Error::Marshal(
            "MySQL result metadata/row arity mismatch".to_owned(),
        ))
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

    async fn result_columns(
        &mut self,
        probe_sql: &str,
    ) -> Result<Vec<crate::backend::ResultColumn>> {
        let stmt = self.conn.borrow_mut().prep(probe_sql).await?;
        stmt.columns()
            .iter()
            .map(|column| {
                let varying_text = matches!(
                    column.column_type(),
                    ColumnType::MYSQL_TYPE_VARCHAR
                        | ColumnType::MYSQL_TYPE_STRING
                        | ColumnType::MYSQL_TYPE_VAR_STRING
                        | ColumnType::MYSQL_TYPE_TINY_BLOB
                        | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
                        | ColumnType::MYSQL_TYPE_LONG_BLOB
                        | ColumnType::MYSQL_TYPE_BLOB
                ) && mysql_xsd_code(column, self.type_profile)?
                    == Some(XsdTypeCode::String);
                Ok(crate::backend::ResultColumn {
                    name: column.name_str().into_owned(),
                    text_key: varying_text.then_some(crate::backend::TextKey::Verbatim),
                })
            })
            .collect()
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
        let codes = stmt
            .columns()
            .iter()
            .map(|column| mysql_xsd_code(column, self.type_profile))
            .collect::<Result<Vec<_>>>()?;
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
                Some(format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}"))
            } else {
                Some(format!(
                    "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{us:06}"
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

fn mysql_xsd_code(column: &Column, profile: MysqlTypeProfile) -> Result<Option<XsdTypeCode>> {
    use ColumnType::*;
    Ok(Some(match column.column_type() {
        MYSQL_TYPE_TINY
            if profile == MysqlTypeProfile::W3cSql2008 && column.column_length() == 1 =>
        {
            XsdTypeCode::Boolean
        }
        MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT | MYSQL_TYPE_LONG | MYSQL_TYPE_LONGLONG
        | MYSQL_TYPE_INT24 | MYSQL_TYPE_YEAR => XsdTypeCode::Integer,
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => XsdTypeCode::Decimal,
        MYSQL_TYPE_FLOAT | MYSQL_TYPE_DOUBLE => XsdTypeCode::Double,
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => XsdTypeCode::Date,
        MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => XsdTypeCode::Time,
        MYSQL_TYPE_TIMESTAMP
        | MYSQL_TYPE_TIMESTAMP2
        | MYSQL_TYPE_DATETIME
        | MYSQL_TYPE_DATETIME2 => XsdTypeCode::DateTime,
        MYSQL_TYPE_BIT => XsdTypeCode::HexBinary,
        MYSQL_TYPE_VARCHAR
        | MYSQL_TYPE_VAR_STRING
        | MYSQL_TYPE_STRING
        | MYSQL_TYPE_TINY_BLOB
        | MYSQL_TYPE_MEDIUM_BLOB
        | MYSQL_TYPE_LONG_BLOB
        | MYSQL_TYPE_BLOB
            if column.character_set() == 63 =>
        {
            XsdTypeCode::HexBinary
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
        | MYSQL_TYPE_JSON => XsdTypeCode::String,
        MYSQL_TYPE_NULL => return Ok(None),
        MYSQL_TYPE_TYPED_ARRAY | MYSQL_TYPE_VECTOR | MYSQL_TYPE_UNKNOWN | MYSQL_TYPE_GEOMETRY => {
            return Err(Error::Unsupported(
                "MySQL result column type is unsupported".to_owned(),
            ))
        }
    }))
}

#[cfg(test)]
#[path = "mysql/tests.rs"]
mod tests;
