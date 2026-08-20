//! DuckDB `SqlBackend` adapter (ADR-0024 M8).
//!
//! DuckDB's Rust binding (`duckdb` crate, `bundled` feature) exposes a distinct
//! streaming Arrow cursor. `Statement::query()` is not that cursor: it executes
//! through `duckdb_execute_prepared` and may materialize the complete result
//! before its `Rows` wrapper yields the first row. `open_branch` therefore uses
//! `Statement::stream_arrow()` (`duckdb_execute_prepared_streaming`) and decodes
//! one bounded DuckDB vector at a time. The `Connection` is `Send` but not
//! `Sync`, so the same cap-1 channel bridge used by `SqliteOwnedBackend`
//! (ADR-0024 §4.1) gives a `Send + 'static` stream suitable for `tokio::spawn`.
//! One DuckDB vector plus one row in flight across the channel keeps memory
//! independent of total result cardinality (ADR-0010 §C).
//! Dropping a stream interrupts its connection and schedules the blocking worker
//! for completion, so cancellation does not leave an unbounded DuckDB query
//! detached from its caller.
//!
//! Verification tier: live-parity (DuckDB is embedded; no external instance
//! required). Enabled via `--features duckdb-backend`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use duckdb::Connection;
use duckdb::arrow::array::{self, Array, ArrayRef, DictionaryArray, FixedSizeBinaryArray};
use duckdb::arrow::datatypes::{DataType, TimeUnit, UInt8Type, UInt16Type, UInt32Type};
use duckdb::core::LogicalTypeId;
use duckdb::params_from_iter;
use duckdb::types::{Decimal, EnumType, ValueRef};
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
        // `Dialect::probe_sql` passes an `rr:sqlQuery` through verbatim.  Calling
        // DuckDB's non-streaming `Statement::query` on that text materializes the
        // complete result merely to discover its column names.  Keep metadata
        // discovery independent of source cardinality just like `open_branch`'s
        // logical-type probe below.
        let probe_sql = streaming_metadata_sql(probe_sql);
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
            // duckdb-rs exposes exact logical result types only after a
            // statement has executed.  Arrow's physical schema alone cannot
            // distinguish HUGEINT, UHUGEINT, and DECIMAL(38, 0), so execute a
            // zero-row wrapper first.  DuckDB plans this query but LIMIT 0
            // prevents it from consuming the source result; the real query
            // below still uses the genuinely streaming execution API.
            let metadata_sql = streaming_metadata_sql(&sql);
            let mut metadata_stmt = match guard.prepare(&metadata_sql) {
                Ok(stmt) => stmt,
                Err(error) => {
                    let _ = tx.blocking_send(Err(Error::Marshal(format!(
                        "duckdb metadata prepare: {error}"
                    ))));
                    return;
                }
            };
            let metadata_rows = match metadata_stmt.query(params_from_iter(params.iter())) {
                Ok(rows) => rows,
                Err(error) => {
                    let _ = tx.blocking_send(Err(Error::Marshal(format!(
                        "duckdb metadata query: {error}"
                    ))));
                    return;
                }
            };
            let Some(metadata) = metadata_rows.as_ref() else {
                let _ = tx.blocking_send(Err(Error::Marshal(
                    "DuckDB metadata query did not expose a result schema".to_owned(),
                )));
                return;
            };
            let ncols = metadata.column_count();
            let logical_types = (0..ncols)
                .map(|index| metadata.column_logical_type(index).id())
                .collect::<Vec<_>>();
            drop(metadata_rows);
            drop(metadata_stmt);

            let mut stmt = match guard.prepare(&sql) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.blocking_send(Err(Error::Marshal(format!("duckdb prepare: {e}"))));
                    return;
                }
            };
            let mut batches = match stmt.stream_arrow(params_from_iter(params.iter())) {
                Ok(batches) => batches,
                Err(e) => {
                    let _ = tx.blocking_send(Err(Error::Marshal(format!("duckdb query: {e}"))));
                    return;
                }
            };
            // ArrowStream::next currently reports a late DuckDB fetch error by
            // panicking. Keep that implementation detail inside the blocking
            // worker and surface it as the same hard stream failure as other
            // marshalling errors.
            let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<()> {
                for batch in &mut batches {
                    if batch.num_columns() != ncols {
                        return Err(Error::Marshal(format!(
                            "DuckDB streaming batch has {} columns, expected {ncols}",
                            batch.num_columns()
                        )));
                    }
                    for row in 0..batch.num_rows() {
                        let mut values = Vec::with_capacity(ncols);
                        let mut codes: Vec<Option<XsdTypeCode>> = Vec::with_capacity(ncols);
                        for (i, &logical_type) in logical_types.iter().enumerate() {
                            let value = duck_arrow_value_ref(batch.column(i), row, logical_type)?;
                            let (text, code) = duck_value(
                                value,
                                logical_type == LogicalTypeId::TimestampTZ,
                                max_value_bytes,
                            )?;
                            values.push(text);
                            codes.push(code);
                        }
                        if tx.blocking_send(Ok(RawTuple { values, codes })).is_err() {
                            return Ok(());
                        }
                    }
                }
                Ok(())
            }));
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    let _ = tx.blocking_send(Err(error));
                }
                Err(_) => {
                    let _ = tx.blocking_send(Err(Error::Marshal(
                        "DuckDB streaming fetch panicked".to_owned(),
                    )));
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

fn streaming_metadata_sql(sql: &str) -> String {
    let sql = sql.trim();
    let sql = sql.strip_suffix(';').map(str::trim_end).unwrap_or(sql);
    format!("SELECT * FROM ({sql}) AS __nova_stream_schema LIMIT 0")
}

/// Reconstruct the scalar [`ValueRef`] carried by one Arrow result vector.
///
/// This mirrors duckdb-rs' row decoder but is used with its genuinely
/// streaming Arrow result. DuckDB logical metadata disambiguates the shared
/// `Decimal128(38, 0)` carrier used by HUGEINT, UHUGEINT, and DECIMAL.
fn duck_arrow_value_ref<'a>(
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
    use XsdTypeCode as X;
    use duckdb::types::ValueRef;
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

    /// `rr:sqlQuery` metadata probes arrive without a row limit.  Discovering
    /// their schema must not execute the result-producing expressions: the dump
    /// executor calls this before it opens the real streaming branch, and a
    /// cardinality-sized probe defeats streaming before the first quad exists.
    #[test]
    fn duckdb_column_names_uses_a_zero_row_probe() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let conn = duckdb::Connection::open_in_memory().unwrap();
            let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)));

            let cols = backend
                .column_names(
                    "SELECT i, CASE WHEN i >= 0 THEN error('metadata query consumed a row') END AS boom FROM range(100000000) AS values(i)",
                )
                .await
                .unwrap();

            assert_eq!(cols, vec!["i", "boom"]);
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

        let second =
            tokio::spawn(
                async move { second_backend.open_branch("SELECT 1", &[]).await.map(drop) },
            );
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
        let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn))).with_max_value_bytes(4);
        let mut stream = backend.open_branch("SELECT 'hello'", &[]).await.unwrap();
        let error = match stream.next_row().await {
            Err(error) => error,
            Ok(_) => panic!("oversized DuckDB text should be rejected"),
        };
        assert!(
            error
                .to_string()
                .contains("exceeding the configured per-value limit"),
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
