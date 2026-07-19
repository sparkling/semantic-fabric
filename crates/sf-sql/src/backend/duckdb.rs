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
        // Lock + prepare inside `spawn_blocking`, mirroring `open_branch` below —
        // NOT inline. Identical deadlock shape to the SQLite backend's fixed
        // `column_names` (see `backend/sqlite.rs`): a `std::sync::Mutex` taken
        // inline in an async fn blocks the tokio worker thread itself, so `N`
        // concurrent callers over one shared connection wedge every worker once
        // `N > worker_threads`.
        let conn = Arc::clone(&self.conn);
        let probe_sql = probe_sql.to_owned();
        let joined = tokio::task::spawn_blocking(move || {
            let guard = conn.lock().unwrap_or_else(|p| p.into_inner());
            let mut stmt = guard
                .prepare(&probe_sql)
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
        })
        .await;
        match joined {
            Ok(result) => result,
            Err(e) => Err(Error::Introspection(format!(
                "column_names spawn_blocking task join error: {e}"
            ))),
        }
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Smoke test: open an in-memory DuckDB, create a table, insert rows, and
    /// drive the `SqlBackend` trait's `open_branch` → `next_row` loop to verify
    /// the cap-1 channel bridge delivers every row correctly.
    ///
    /// Verification tier: live-parity (DuckDB is embedded; no external instance needed).
    #[test]
    fn duckdb_backend_streams_rows() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let conn = duckdb::Connection::open_in_memory().unwrap();
            conn.execute_batch(
                "CREATE TABLE emp (id INTEGER, name VARCHAR, salary DOUBLE);
                 INSERT INTO emp VALUES (1, 'Alice', 90000.5);
                 INSERT INTO emp VALUES (2, 'Bob', 80000.0);
                 INSERT INTO emp VALUES (3, 'Carol', NULL);",
            )
            .unwrap();
            let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)));

            // column_names probe
            let cols = backend
                .column_names("SELECT * FROM emp LIMIT 0")
                .await
                .unwrap();
            assert_eq!(cols, vec!["id", "name", "salary"]);

            // open_branch and stream all rows
            let mut stream = backend
                .open_branch("SELECT id, name, salary FROM emp ORDER BY id", &[])
                .await
                .unwrap();

            let row1 = stream.next_row().await.unwrap().unwrap();
            assert_eq!(row1.values[0].as_deref(), Some("1"));
            assert_eq!(row1.values[1].as_deref(), Some("Alice"));
            assert_eq!(row1.values[2].as_deref(), Some("90000.5"));

            let row2 = stream.next_row().await.unwrap().unwrap();
            assert_eq!(row2.values[0].as_deref(), Some("2"));
            assert_eq!(row2.values[1].as_deref(), Some("Bob"));

            let row3 = stream.next_row().await.unwrap().unwrap();
            assert_eq!(row3.values[2], None, "NULL salary should be None");

            let eof = stream.next_row().await.unwrap();
            assert!(eof.is_none(), "should be EOF after 3 rows");
        });
    }

    /// Verify that `open_branch` with parameters binds correctly.
    #[test]
    fn duckdb_backend_parameter_binding() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let conn = duckdb::Connection::open_in_memory().unwrap();
            conn.execute_batch(
                "CREATE TABLE val (n INTEGER);
                 INSERT INTO val VALUES (10);
                 INSERT INTO val VALUES (20);
                 INSERT INTO val VALUES (30);",
            )
            .unwrap();
            let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)));
            let mut stream = backend
                .open_branch("SELECT n FROM val WHERE n > ?", &["15".to_owned()])
                .await
                .unwrap();

            let r1 = stream.next_row().await.unwrap().unwrap();
            let r2 = stream.next_row().await.unwrap().unwrap();
            let eof = stream.next_row().await.unwrap();

            let mut got = vec![
                r1.values[0].as_deref().unwrap().parse::<i32>().unwrap(),
                r2.values[0].as_deref().unwrap().parse::<i32>().unwrap(),
            ];
            got.sort_unstable();
            assert_eq!(got, vec![20, 30]);
            assert!(eof.is_none());
        });
    }
}
