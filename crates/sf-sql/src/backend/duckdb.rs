//! DuckDB `SqlBackend` adapter (ADR-0024 M8).
//!
//! DuckDB's Rust binding (`duckdb` crate, `bundled` feature) mirrors `rusqlite`:
//! a synchronous `Connection` + `Statement` + `Rows` cursor. The `Connection` is
//! `Send` but not `Sync`, so the same cap-1 channel bridge pattern used by
//! `SqliteOwnedBackend` (ADR-0024 §4.1) is applied here, giving a
//! `Send + 'static` stream suitable for `tokio::spawn`.
//! One row in flight across the channel → bounded memory (ADR-0010 §C).
//! Dropping a stream interrupts its connection and schedules the blocking worker
//! for completion, so cancellation does not leave an unbounded DuckDB query
//! detached from its caller.
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
/// The `Mutex` is required because `Connection` is `!Sync`.
pub struct DuckDbBackend {
    conn: Arc<Mutex<Connection>>,
    max_value_bytes: Option<usize>,
}

impl DuckDbBackend {
    /// Wrap an existing connection handle.
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self {
            conn,
            max_value_bytes: None,
        }
    }

    /// Reject any single decoded SQL value larger than `max_value_bytes` before
    /// allocating its owned lexical representation.
    pub fn with_max_value_bytes(mut self, max_value_bytes: usize) -> Self {
        self.max_value_bytes = Some(max_value_bytes);
        self
    }
}

/// The receive end of the cap-1 bridge channel.
/// Each `next_row` awaits the next `Result<RawTuple>` from the blocking cursor.
pub struct DuckDbReceiverStream {
    rx: tokio::sync::mpsc::Receiver<Result<RawTuple>>,
    state: Arc<Mutex<DuckDbWorkerState>>,
    worker: Option<tokio::task::JoinHandle<()>>,
    finished: bool,
}

#[derive(Default)]
struct DuckDbWorkerState {
    cancel_requested: bool,
    running_with_connection: bool,
    interrupt: Option<Arc<duckdb::InterruptHandle>>,
}

struct DuckDbWorkerRunning {
    state: Arc<Mutex<DuckDbWorkerState>>,
}

impl Drop for DuckDbWorkerRunning {
    fn drop(&mut self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.running_with_connection = false;
        state.interrupt = None;
    }
}

impl DuckDbReceiverStream {
    async fn finish_worker(&mut self) -> Result<()> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker.await.map_err(|error| {
            Error::Marshal(format!("duckdb blocking cursor task join error: {error}"))
        })
    }

    fn cancel_worker(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.cancel_requested = true;
        if state.running_with_connection {
            // Interrupt while holding the state lock: the worker must clear
            // `running_with_connection` under this same lock before releasing
            // the connection, so this cannot spill into a subsequent query.
            if let Some(interrupt) = &state.interrupt {
                interrupt.interrupt();
            }
        }
    }
}

impl BranchStream for DuckDbReceiverStream {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        match self.rx.recv().await {
            None => {
                self.finished = true;
                self.finish_worker().await?;
                Ok(None)
            }
            Some(Ok(tuple)) => Ok(Some(tuple)),
            Some(Err(error)) => {
                self.cancel_worker();
                self.finish_worker().await?;
                self.finished = true;
                Err(error)
            }
        }
    }
}

impl Drop for DuckDbReceiverStream {
    fn drop(&mut self) {
        if !self.finished {
            self.cancel_worker();
        }
        if let Some(worker) = self.worker.take() {
            let state = Arc::clone(&self.state);
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    while interrupt_running_worker(&state) {
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                    let _ = worker.await;
                });
            } else {
                std::thread::spawn(move || {
                    while interrupt_running_worker(&state) {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    drop(worker);
                });
            }
        }
    }
}

fn interrupt_running_worker(state: &Mutex<DuckDbWorkerState>) -> bool {
    let state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.running_with_connection {
        if let Some(interrupt) = &state.interrupt {
            interrupt.interrupt();
        }
        true
    } else {
        false
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
        let max_value_bytes = self.max_value_bytes;
        let state = Arc::new(Mutex::new(DuckDbWorkerState::default()));
        let worker_state = Arc::clone(&state);
        let sql = sql.to_owned();
        let params: Vec<String> = lexical_params.to_vec();

        let worker = tokio::task::spawn_blocking(move || {
            let guard = conn.lock().unwrap_or_else(|p| p.into_inner());
            let interrupt = guard.interrupt_handle();
            {
                let mut state = worker_state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if state.cancel_requested {
                    return;
                }
                state.running_with_connection = true;
                state.interrupt = Some(interrupt);
            }
            // Declared after the connection guard so it marks the worker idle
            // before the connection can be acquired by another branch.
            let _running = DuckDbWorkerRunning {
                state: Arc::clone(&worker_state),
            };
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
            let timestamp_with_timezone = rows
                .as_ref()
                .map(|statement| {
                    (0..ncols)
                        .map(|index| {
                            statement.column_logical_type(index).id()
                                == duckdb::core::LogicalTypeId::TimestampTZ
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            loop {
                match rows.next() {
                    Ok(Some(row)) => {
                        let mut values = Vec::with_capacity(ncols);
                        let mut codes: Vec<Option<XsdTypeCode>> = Vec::with_capacity(ncols);
                        let mut ok = true;
                        for i in 0..ncols {
                            match row.get_ref(i) {
                                Ok(v) => match duck_value(
                                    v,
                                    timestamp_with_timezone.get(i).copied().unwrap_or(false),
                                    max_value_bytes,
                                ) {
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
        Ok(DuckDbReceiverStream {
            rx,
            state,
            worker: Some(worker),
            finished: false,
        })
    }
}

/// Map a DuckDB [`duckdb::types::ValueRef`] to a lexical string + XSD type code.
///
/// SQL scalar types with an R2RML §10 natural mapping are converted to their
/// lexical representation and XSD code. Nested types and intervals remain
/// unsupported and surface as a 501 via `exec_core::map_sql_err`.
fn duck_value(
    v: duckdb::types::ValueRef<'_>,
    timestamp_with_timezone: bool,
    max_value_bytes: Option<usize>,
) -> Result<(Option<String>, Option<XsdTypeCode>)> {
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
            let timestamp = chrono::DateTime::from_timestamp(seconds, nanos)
                .ok_or_else(|| {
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
            let encoded_len = b.len().checked_mul(2).ok_or_else(|| {
                Error::Marshal("DuckDB blob lexical size overflow".to_owned())
            })?;
            ensure_value_size("blob", encoded_len, max_value_bytes)?;
            let mut out = String::new();
            sf_core::datatype::hex_binary_upper(b, &mut out);
            Ok((Some(out), Some(X::HexBinary)))
        }
        ValueRef::Enum(..) => {
            let value = v
                .as_str()
                .map_err(|error| Error::Marshal(format!("duckdb enum: {error}")))?;
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

    #[tokio::test]
    async fn duckdb_backend_maps_common_r2rml_scalar_types() {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)));
        let mut stream = backend
            .open_branch(
                "SELECT \
                 170141183460469231731687303715884105728::UHUGEINT, \
                 12.340::DECIMAL(10,3), \
                 DATE '1969-12-31', \
                 TIME '01:02:03.004005', \
                 TIMESTAMP '1969-12-31 23:59:59.500000', \
                 TIMESTAMPTZ '1969-12-31 23:59:59.500000+00'",
                &[],
            )
            .await
            .unwrap();
        let row = stream.next_row().await.unwrap().unwrap();
        assert_eq!(
            row.values,
            vec![
                Some("170141183460469231731687303715884105728".into()),
                Some("12.340".into()),
                Some("1969-12-31".into()),
                Some("01:02:03.004005".into()),
                Some("1969-12-31 23:59:59.500".into()),
                Some("1969-12-31T23:59:59.500Z".into()),
            ]
        );
        assert_eq!(
            row.codes,
            vec![
                Some(XsdTypeCode::Integer),
                Some(XsdTypeCode::Decimal),
                Some(XsdTypeCode::Date),
                Some(XsdTypeCode::Time),
                Some(XsdTypeCode::DateTime),
                Some(XsdTypeCode::DateTime),
            ]
        );
        assert!(stream.next_row().await.unwrap().is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn opening_a_second_branch_never_blocks_the_async_executor() {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        let shared = Arc::new(Mutex::new(conn));
        let mut first_backend = DuckDbBackend::new(Arc::clone(&shared));
        let mut second_backend = DuckDbBackend::new(Arc::clone(&shared));
        let mut first_stream = first_backend
            .open_branch("SELECT range::INTEGER FROM range(4)", &[])
            .await
            .unwrap();
        assert!(first_stream.next_row().await.unwrap().is_some());

        let second = tokio::spawn(async move {
            second_backend
                .open_branch("SELECT 1", &[])
                .await
                .map(drop)
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), second)
            .await
            .expect("open_branch must not synchronously wait for the shared connection")
            .expect("second branch task should join")
            .expect("second branch should open");
        drop(first_stream);
    }

    #[tokio::test]
    async fn configured_value_limit_rejects_before_copying_text() {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)))
            .with_max_value_bytes(4);
        let mut stream = backend.open_branch("SELECT 'hello'", &[]).await.unwrap();
        let error = match stream.next_row().await {
            Err(error) => error,
            Ok(_) => panic!("oversized DuckDB text should be rejected"),
        };
        assert!(
            error.to_string().contains("exceeding the configured per-value limit"),
            "{error}"
        );
    }

    /// P0 deadlock regression (2e78f3f fixed `column_names` on both the SQLite and
    /// DuckDB twins; the SQLite twin got a live concurrency receipt
    /// (`sf-serve/tests/endpoint.rs::column_names_spawn_blocking_deadlock_regression`)
    /// but the DuckDB twin never did — this is that receipt. `sf-serve` has no
    /// DuckDB `Backend` variant, so this drives `DuckDbBackend` directly (the
    /// `SqlBackend` trait) rather than through the HTTP endpoint.
    ///
    /// Deterministic choreography, not a timing race: a naive "spawn N identical
    /// tasks and hope they collide" version does NOT reliably reproduce this on an
    /// under-loaded, many-core machine — every contender starts by racing for an
    /// INITIALLY FREE lock, and the OS's wake-one-waiter fairness lets them cycle
    /// through `column_names` in quick succession *before* any of them reaches
    /// `open_branch`'s long hold, so the run just serializes instead of wedging
    /// (empirically confirmed while building this test). The real bug needs the
    /// lock ALREADY held by a streaming cursor when the contenders arrive, so this
    /// test forces that ordering explicitly: get exactly one row through
    /// `open_branch` first — the `Mutex` guard spans open_branch's WHOLE
    /// `spawn_blocking` closure, so once a row is observed, the connection lock is
    /// provably held until the entire cursor is drained — THEN spawn `N_CONTENDERS
    /// >= worker_threads` fresh callers of `column_names` against that
    /// already-held lock, and only then resume draining.
    ///
    /// Pre-fix failure shape (mirrors the SQLite RED evidence exactly, same
    /// mechanism, different driver): if `column_names`'s `Mutex` lock is ever
    /// taken INLINE again (not inside its own `spawn_blocking`), each contender's
    /// lock attempt blocks ITS OWN tokio worker thread outright — once enough
    /// contenders pile on to saturate every worker thread, nothing remains free to
    /// poll the mpsc receiver that would let the cursor's `blocking_send` drain
    /// and release the lock: total starvation, not even tokio's own timer can
    /// schedule (the SQLite regression was force-killed at 90s, zero output;
    /// confirmed here by locally reintroducing the inline-lock pattern and
    /// observing the identical hang-past-timeout shape, then reverting). Wrapped
    /// in a 30s `tokio::time::timeout` so a regression is a clean test FAILURE in
    /// the common case — though a TRUE total wedge can starve the timer too, same
    /// as the SQLite precedent, in which case the test process itself must be
    /// externally killed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn column_names_concurrent_deadlock_regression() {
        const N_CONTENDERS: usize = 4; // >= worker_threads: enough to wedge every one
        const ROWS: usize = 20_000;

        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE emp (id INTEGER, name VARCHAR);
             INSERT INTO emp SELECT range, 'Person' || range FROM range({ROWS});"
        ))
        .unwrap();
        let shared = Arc::new(Mutex::new(conn));

        let run = async {
            let mut backend = DuckDbBackend::new(Arc::clone(&shared));
            backend
                .column_names("SELECT id, name FROM emp LIMIT 0")
                .await
                .expect("winner column_names");
            let mut stream = backend
                .open_branch("SELECT id, name FROM emp ORDER BY id", &[])
                .await
                .expect("winner open_branch");
            // Proves the spawn_blocking cursor has started and is now holding the
            // connection Mutex for the rest of its (long) drain.
            let row1 = stream.next_row().await.expect("winner row 1");
            assert!(row1.is_some(), "expected at least one row");

            let mut handles = Vec::with_capacity(N_CONTENDERS);
            for _ in 0..N_CONTENDERS {
                let shared = Arc::clone(&shared);
                handles.push(tokio::spawn(async move {
                    let mut backend = DuckDbBackend::new(shared);
                    let cols = backend
                        .column_names("SELECT id, name FROM emp LIMIT 0")
                        .await
                        .expect("contender column_names");
                    assert_eq!(cols, vec!["id", "name"]);
                }));
            }
            // A genuine yield (not just an immediately-ready poll) so the
            // contenders are guaranteed a chance to start and block on the
            // already-held lock BEFORE draining resumes — removing any
            // dependence on tokio's cooperative-yield timing.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;

            // The call that must hang in the pre-fix world: no worker thread
            // remains free to poll it once the contenders above have wedged
            // every one of them.
            let mut n = 1usize;
            while stream.next_row().await.expect("winner next_row").is_some() {
                n += 1;
            }
            assert_eq!(n, ROWS);

            for h in handles {
                h.await.expect("contender task join");
            }
        };

        tokio::time::timeout(std::time::Duration::from_secs(30), run)
            .await
            .expect(
                "column_names deadlock regression: contenders piling onto column_names \
                 while a cursor holds the connection Mutex mid-stream hung past 30s under \
                 worker_threads=4 — DuckDbBackend::column_names is locking its Mutex \
                 inline again",
            );
    }

    /// Dropping a stream interrupts a query before its first row and releases
    /// the shared connection for the next operation.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropped_stream_interrupts_blocking_cursor() {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        let shared = Arc::new(Mutex::new(conn));
        let mut backend = DuckDbBackend::new(Arc::clone(&shared));
        let stream = backend
            .open_branch(
                "SELECT sum(sin(i::DOUBLE)) FROM range(1000000000) values_(i)",
                &[],
            )
            .await
            .unwrap();

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if shared.try_lock().is_err() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("DuckDB cursor worker should acquire the connection");
        drop(stream);

        let shared_for_probe = Arc::clone(&shared);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::task::spawn_blocking(move || {
                let connection = shared_for_probe
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                connection.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            }),
        )
        .await
        .expect("interrupted DuckDB cursor should release the connection")
        .expect("probe worker should join")
        .expect("connection should remain usable after interruption");
    }
}
