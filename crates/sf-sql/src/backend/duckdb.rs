//! DuckDB `SqlBackend` adapter (ADR-0024 M8).
//!
//! DuckDB's Rust binding (`duckdb` crate, `bundled` feature) exposes a distinct
//! streaming Arrow cursor. `Statement::query()` is not that cursor: it executes
//! through `duckdb_execute_prepared` and may materialize the complete result
//! before its `Rows` wrapper yields the first row. `open_branch` therefore uses
//! `Statement::stream_arrow()` (`duckdb_execute_prepared_streaming`) and decodes
//! one DuckDB vector at a time. The adapter buffers at most one current Arrow
//! batch plus the cap-1 channel's owned row and a producer-blocked row; DuckDB
//! may still materialize internally for blocking operators. The `Connection` is `Send` but not
//! `Sync`, so the same cap-1 channel bridge used by `SqliteOwnedBackend`
//! (ADR-0024 §4.1) gives a `Send + 'static` stream suitable for `tokio::spawn`.
//! One DuckDB vector plus one row in flight across the channel keeps memory
//! independent of total result cardinality (ADR-0010 §C).
//! Dropping a stream requests an interrupt and schedules the blocking worker for
//! completion. Interruption is best effort during runtime shutdown.
//!
//! Verification tier: live-parity (DuckDB is embedded; no external instance
//! required). Enabled via `--features duckdb-backend`.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use duckdb::core::LogicalTypeId;
use duckdb::params_from_iter;
use duckdb::Connection;
use sf_core::datatype::XsdTypeCode;

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

mod value;

use value::{duck_arrow_value_ref, duck_value};

#[cfg(test)]
mod tests;

/// An owned, `'static` DuckDB backend over `Arc<Mutex<Connection>>`.
/// The `Mutex` is required because `Connection` is `!Sync`.
pub struct DuckDbBackend {
    conn: Arc<Mutex<Connection>>,
    max_value_bytes: Option<usize>,
    max_row_bytes: Option<usize>,
}

impl DuckDbBackend {
    /// Wrap an existing connection handle.
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self {
            conn,
            max_value_bytes: None,
            max_row_bytes: None,
        }
    }

    /// Reject variable-width SQL values whose owned lexical representation
    /// would exceed `max_value_bytes`.
    pub fn with_max_value_bytes(mut self, max_value_bytes: usize) -> Self {
        self.max_value_bytes = Some(max_value_bytes);
        self
    }

    /// Reject a decoded row once the sum of its non-NULL lexical values would
    /// exceed `max_row_bytes`.
    pub fn with_max_row_bytes(mut self, max_row_bytes: usize) -> Self {
        self.max_row_bytes = Some(max_row_bytes);
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
        let statements = crate::Dialect::DuckDb.parse(probe_sql).map_err(|error| {
            Error::Unsupported(format!("invalid DuckDB metadata probe: {error}"))
        })?;
        if statements.len() != 1
            || !matches!(
                statements.first(),
                Some(sqlparser::ast::Statement::Query(_))
            )
        {
            return Err(Error::Unsupported(
                "DuckDB metadata probes must contain exactly one read-only query".to_owned(),
            ));
        }
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
        let max_row_bytes = self.max_row_bytes;
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
                        let mut row_bytes = 0usize;
                        for (i, &logical_type) in logical_types.iter().enumerate() {
                            let value = duck_arrow_value_ref(batch.column(i), row, logical_type)?;
                            let (text, code) = duck_value(
                                value,
                                logical_type == LogicalTypeId::TimestampTZ,
                                max_value_bytes,
                            )?;
                            row_bytes = row_bytes
                                .checked_add(text.as_ref().map_or(0, String::len))
                                .ok_or_else(|| {
                                    Error::Marshal("DuckDB row lexical size overflow".to_owned())
                                })?;
                            if max_row_bytes.is_some_and(|maximum| row_bytes > maximum) {
                                return Err(Error::Marshal(format!(
                                    "DuckDB row requires {row_bytes} lexical bytes, exceeding the configured per-row limit of {} bytes",
                                    max_row_bytes.expect("maximum checked above")
                                )));
                            }
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
