//! MySQL `SqlBackend` adapter (ADR-0024 §2, §4.2). The ONE driver whose pull cursor
//! borrows the handle: `Stream<'s>` is a native `mysql_async::QueryResult` borrowing
//! `&'s mut Conn` (the reason the GAT exists — design §0 fact 3). Built on
//! `exec_iter` + `row.take::<Value>` + `mysql_value_to_string` (moved VERBATIM from
//! the old `sf-sparql::exec_mysql` loop), NEVER `stream::mysql_for_each` — whose
//! `from_utf8_lossy` decode is a `=_bag` / 3-valued-logic regression (design A1).
//! `mysql_async` has no server-side cursor, so this is client-buffer-free /
//! packet-bounded, not cursor-grade (design §4 / §4.2).

use std::borrow::BorrowMut;

use mysql_async::prelude::Queryable;
use mysql_async::{BinaryProtocol, Conn, Params, QueryResult, Value};

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::Result;

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
            values.push(mysql_value_to_string(v));
        }
        // v1: text protocol carries no per-row wire types ⇒ all codes None
        // (exec_mysql.rs:187 verbatim).
        let codes = vec![None; ncols];
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
        // exec_iter (packet-streamed) — NOT exec() (buffer-all Vec<Row>).
        let result = conn.exec_iter(stmt, Params::Positional(params)).await?;
        Ok(MysqlBranch { result })
    }
}

/// Convert a single MySQL [`Value`] cell to a raw lexical [`String`] (NULL → `None`).
/// All wire types are converted via their natural Rust representation and then
/// formatted as strings — the same principle as the PostgreSQL text-protocol path.
///
/// **Bytes:** only valid UTF-8 sequences are accepted; non-UTF-8 bytes (BLOB /
/// VARBINARY) yield `None` (unbound) rather than a silently-corrupted literal.
/// Callers that need binary BLOB values should add an `rr:datatype` declaration
/// and a custom term-gen hook (ADR-0014 follow-up).
///
/// **Date midnight:** `Value::Date` with all-zero time fields is produced by both
/// MySQL `DATE` columns and `DATETIME`/`TIMESTAMP` columns whose value is exactly
/// midnight. Without per-column wire-type metadata (available from `stmt.columns()`
/// but not yet threaded through the v1 executor) the two are indistinguishable.
/// The emitted lexical form `"YYYY-MM-DD"` is correct for `DATE` columns mapped with
/// `rr:datatype xsd:date`; for `DATETIME` midnight mapped with `rr:datatype xsd:dateTime`
/// this produces an invalid xsd:dateTime lexical form — a known v1 limitation tracked
/// under ADR-0014.
fn mysql_value_to_string(v: Value) -> Option<String> {
    use mysql_async::Value::*;
    match v {
        NULL => None,
        // Reject non-UTF-8 bytes rather than silently corrupting BLOB data.
        Bytes(b) => String::from_utf8(b).ok(),
        Int(i) => Some(i.to_string()),
        UInt(u) => Some(u.to_string()),
        Float(f) => Some(f.to_string()),
        Double(d) => Some(d.to_string()),
        Date(y, mo, d, h, mi, s, us) => {
            if h == 0 && mi == 0 && s == 0 && us == 0 {
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
    }
}
