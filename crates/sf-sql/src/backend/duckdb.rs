//! DuckDB `SqlBackend` adapter (ADR-0024 M8).
//!
//! DuckDB's Rust binding (`duckdb` crate, `bundled` feature) mirrors `rusqlite`:
//! a synchronous `Connection` + `Statement` + `Rows` cursor. The `Connection` is
//! not `Send`, so the same cap-1 channel bridge pattern used by
//! `SqliteOwnedBackend` (ADR-0024 §4.1) is applied here, giving a
//! `Send + 'static` stream suitable for `tokio::spawn`.
//! One row in flight across the channel → bounded memory (ADR-0010 §C).
//!
//! Verification tier: live-parity (DuckDB is embedded; no external instance
//! required). Enabled via `--features duckdb-backend`.

use std::sync::{Arc, Mutex};

use duckdb::params_from_iter;
use duckdb::Connection;
use sf_core::datatype::XsdTypeCode;

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

/// An owned, `'static` DuckDB backend over `Arc<Mutex<Connection>>`.
/// The `Mutex` is required because `Connection` is `!Send`.
pub struct DuckDbBackend {
    conn: Arc<Mutex<Connection>>,
}

impl DuckDbBackend {
    /// Wrap an existing connection handle.
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

/// The receive end of the cap-1 bridge channel.
/// Each `next_row` awaits the next `Result<RawTuple>` from the blocking cursor.
pub struct DuckDbReceiverStream {
    rx: tokio::sync::mpsc::Receiver<Result<RawTuple>>,
}

impl BranchStream for DuckDbReceiverStream {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        match self.rx.recv().await {
            None => Ok(None),
            Some(Ok(tuple)) => Ok(Some(tuple)),
            Some(Err(e)) => Err(e),
        }
    }
}

impl SqlBackend for DuckDbBackend {
    type Stream<'s>
        = DuckDbReceiverStream
    where
        Self: 's;

    async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
        let guard = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = guard
            .prepare(probe_sql)
            .map_err(|e| Error::Marshal(format!("duckdb prepare: {e}")))?;
        // Execute with no params to populate column metadata.
        let rows = stmt
            .query(params_from_iter(std::iter::empty::<String>()))
            .map_err(|e| Error::Marshal(format!("duckdb query: {e}")))?;
        let ncols = rows.as_ref().map(|s| s.column_count()).unwrap_or(0);
        let names = (0..ncols)
            .map(|i| {
                rows.as_ref()
                    .and_then(|s| s.column_name(i).ok())
                    .map(|s| s.to_owned())
                    .unwrap_or_else(|| format!("col{i}"))
            })
            .collect();
        Ok(names)
    }

    async fn open_branch(
        &mut self,
        sql: &str,
        lexical_params: &[String],
    ) -> Result<DuckDbReceiverStream> {
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<RawTuple>>(1);
        let conn = Arc::clone(&self.conn);
        let sql = sql.to_owned();
        let params: Vec<String> = lexical_params.to_vec();

        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = match guard.prepare(&sql) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.blocking_send(Err(Error::Marshal(format!("duckdb prepare: {e}"))));
                    return;
                }
            };
            let mut rows = match stmt.query(params_from_iter(params.iter())) {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.blocking_send(Err(Error::Marshal(format!("duckdb query: {e}"))));
                    return;
                }
            };
            // Column count is available from the statement once the result is ready.
            let ncols = rows.as_ref().map(|s| s.column_count()).unwrap_or(0);
            loop {
                match rows.next() {
                    Ok(Some(row)) => {
                        let mut values = Vec::with_capacity(ncols);
                        let mut codes: Vec<Option<XsdTypeCode>> = Vec::with_capacity(ncols);
                        let mut ok = true;
                        for i in 0..ncols {
                            match row.get_ref(i) {
                                Ok(v) => match duck_value(v) {
                                    Ok((text, code)) => {
                                        values.push(text);
                                        codes.push(code);
                                    }
                                    Err(e) => {
                                        let _ = tx.blocking_send(Err(e));
                                        ok = false;
                                        break;
                                    }
                                },
                                Err(e) => {
                                    let _ = tx.blocking_send(Err(Error::Marshal(format!(
                                        "duckdb col {i}: {e}"
                                    ))));
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        if ok && tx.blocking_send(Ok(RawTuple { values, codes })).is_err() {
                            break;
                        }
                        if !ok {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tx.blocking_send(Err(Error::Marshal(format!("duckdb row: {e}"))));
                        break;
                    }
                }
            }
        });
        Ok(DuckDbReceiverStream { rx })
    }
}

/// Map a DuckDB [`duckdb::types::ValueRef`] to a lexical string + XSD type code.
///
/// Primitive scalar types are mapped directly. Complex/nested types
/// (Timestamp, Date, Time, Decimal, Interval, List, Struct, Enum, Union, Array, Map)
/// return `Error::Unsupported` — the caller will surface a 501 skip via
/// `exec_core::map_sql_err` (ADR-0024 design A2).
fn duck_value(v: duckdb::types::ValueRef<'_>) -> Result<(Option<String>, Option<XsdTypeCode>)> {
    use duckdb::types::ValueRef;
    use XsdTypeCode as X;
    match v {
        ValueRef::Null => Ok((None, None)),
        ValueRef::Boolean(b) => Ok((Some(b.to_string()), Some(X::Boolean))),
        ValueRef::TinyInt(i) => Ok((Some(i.to_string()), Some(X::Integer))),
        ValueRef::SmallInt(i) => Ok((Some(i.to_string()), Some(X::Integer))),
        ValueRef::Int(i) => Ok((Some(i.to_string()), Some(X::Integer))),
        ValueRef::BigInt(i) => Ok((Some(i.to_string()), Some(X::Integer))),
        ValueRef::HugeInt(i) => Ok((Some(i.to_string()), Some(X::Integer))),
        ValueRef::UTinyInt(u) => Ok((Some(u.to_string()), Some(X::Integer))),
        ValueRef::USmallInt(u) => Ok((Some(u.to_string()), Some(X::Integer))),
        ValueRef::UInt(u) => Ok((Some(u.to_string()), Some(X::Integer))),
        ValueRef::UBigInt(u) => Ok((Some(u.to_string()), Some(X::Integer))),
        ValueRef::Float(f) => Ok((Some(f.to_string()), Some(X::Double))),
        ValueRef::Double(d) => Ok((Some(d.to_string()), Some(X::Double))),
        ValueRef::Text(t) => {
            let s = std::str::from_utf8(t)
                .map_err(|e| Error::Marshal(format!("duckdb non-UTF8 text: {e}")))?;
            Ok((Some(s.to_owned()), Some(X::String)))
        }
        ValueRef::Blob(b) => {
            let mut out = String::new();
            sf_core::datatype::hex_binary_upper(b, &mut out);
            Ok((Some(out), Some(X::HexBinary)))
        }
        // Timestamp, Date32, Time64, Decimal, Interval, List, Struct, Enum, Union, Array, Map
        other => Err(Error::Unsupported(format!(
            "DuckDB value type {:?} not supported",
            other.data_type()
        ))),
    }
}
