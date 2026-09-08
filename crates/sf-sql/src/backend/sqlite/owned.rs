//! Owned cap-1 SQLite serve-lane bridge (ADR-0024 §4.1).

use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use sf_core::query_control::QueryControl;
use tokio::sync::{AcquireError, OwnedSemaphorePermit, Semaphore};

use crate::backend::{BranchStream, RawTuple, SqlBackend};
use crate::error::{Error, Result};

use super::cancellation::{
    SqliteCancellationEvent, SqliteCancellationGuard, SqliteCancellationObserver,
};
use super::{column_meta, marshal_row};

#[cfg(test)]
#[path = "owned_admission_tests.rs"]
mod admission_tests;
#[cfg(test)]
#[path = "owned_cancellation_lifecycle_tests.rs"]
mod cancellation_lifecycle_tests;
#[cfg(test)]
#[path = "owned_cancellation_tests.rs"]
mod cancellation_tests;

/// One physical SQLite connection paired permanently with its cap-1 async
/// admission gate. Clones preserve that identity; they never mint another gate.
#[derive(Clone)]
pub struct SqliteOwnedConnection {
    conn: Arc<Mutex<Connection>>,
    admission: Arc<Semaphore>,
}

impl SqliteOwnedConnection {
    /// Pair a physical connection with its one serving admission permit.
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: Arc::new(Mutex::new(conn)),
            admission: Arc::new(Semaphore::new(1)),
        }
    }

    /// Preserve access for legacy callers that do not participate in serving
    /// admission. Such access is outside the lease's concurrency guarantee.
    pub fn raw_connection(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }

    /// Wait asynchronously for exclusive serving admission to this connection.
    /// Dropping the acquisition future cancels only this queue wait.
    pub async fn acquire(&self) -> std::result::Result<SqliteOwnedLease, AcquireError> {
        let permit = Arc::clone(&self.admission).acquire_owned().await?;
        Ok(SqliteOwnedLease {
            state: Arc::new(SqliteOwnedLeaseState {
                conn: Arc::clone(&self.conn),
                _permit: permit,
            }),
        })
    }

    #[cfg(test)]
    fn available_permits(&self) -> usize {
        self.admission.available_permits()
    }
}

/// An opaque exclusive serving lease for one physical SQLite connection.
/// It is consumed exactly once by [`SqliteOwnedBackend`]; only that backend may
/// clone the private state into a submitted worker so the worker can outlive a
/// cancelled async caller without admitting overlapping serving work.
pub struct SqliteOwnedLease {
    state: Arc<SqliteOwnedLeaseState>,
}

struct SqliteOwnedLeaseState {
    conn: Arc<Mutex<Connection>>,
    _permit: OwnedSemaphorePermit,
}

/// An **owned, `'static`** SQLite backend over `Arc<Mutex<Connection>>` — the serve
/// lane's flavor (design §4.1). Its stream ([`SqliteReceiverStream`]) is the receive
/// end of a **cap-1** channel fed by a `spawn_blocking` cursor thread, so the sync,
/// `!Send` `Connection` never crosses a thread boundary and the stream is
/// `Send + 'static` — what lets the generic core's `for<'s> B::Stream<'s>: Send`
/// bound hold across `tokio::spawn`.
pub struct SqliteOwnedBackend {
    conn: Arc<Mutex<Connection>>,
    lease: Option<Arc<SqliteOwnedLeaseState>>,
    control: Option<Arc<dyn QueryControl>>,
    observer: SqliteCancellationObserver,
}

impl SqliteOwnedBackend {
    /// Wrap a shared connection handle (the serve lane already holds this shape).
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self {
            conn,
            lease: None,
            control: None,
            observer: SqliteCancellationObserver::default(),
        }
    }

    /// Wrap a shared connection with the exact request-scoped control identity
    /// used by exec-core. The worker clones this Arc; it never mints a control.
    /// Cancellation starts only after mutex acquisition and cannot pre-empt mutex
    /// or queue waits, SQLite busy waits, blocking UDF/VFS/I/O, or work outside an
    /// active SQLite VM. This backend exclusively owns the connection-global
    /// progress-handler slot while each locked operation runs; a prior handler is
    /// not restored.
    pub fn new_controlled(conn: Arc<Mutex<Connection>>, control: Arc<dyn QueryControl>) -> Self {
        Self {
            conn,
            lease: None,
            control: Some(control),
            observer: SqliteCancellationObserver::default(),
        }
    }

    /// Build a governed backend from a serving lease acquired before any
    /// blocking task is submitted. Each worker receives a private state clone.
    pub fn new_controlled_leased(lease: SqliteOwnedLease, control: Arc<dyn QueryControl>) -> Self {
        Self {
            conn: Arc::clone(&lease.state.conn),
            lease: Some(lease.state),
            control: Some(control),
            observer: SqliteCancellationObserver::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn new_controlled_observed(
        conn: Arc<Mutex<Connection>>,
        control: Arc<dyn QueryControl>,
        observer: SqliteCancellationObserver,
    ) -> Self {
        Self {
            conn,
            lease: None,
            control: Some(control),
            observer,
        }
    }

    #[cfg(test)]
    fn new_controlled_leased_observed(
        lease: SqliteOwnedLease,
        control: Arc<dyn QueryControl>,
        observer: SqliteCancellationObserver,
    ) -> Self {
        Self {
            conn: Arc::clone(&lease.state.conn),
            lease: Some(lease.state),
            control: Some(control),
            observer,
        }
    }
}

fn send_error(
    tx: &tokio::sync::mpsc::Sender<Result<RawTuple>>,
    control: Option<&dyn QueryControl>,
    error: Error,
) {
    // Preserve an already-classified driver/marshalling error. The checkpoint is
    // still mandatory immediately before every potentially blocking send.
    if let Some(control) = control {
        let _ = control.checkpoint();
    }
    let _ = tx.blocking_send(Err(error));
}

/// The receive end of the cap-1 bridge: each `next_row` awaits the next
/// `Result<RawTuple>` produced by the blocking cursor. `None` ⇒ clean EOF;
/// `Some(Err)` ⇒ a HARD mid-stream marshalling/driver error (design A2), never a
/// silent short read.
pub struct SqliteReceiverStream {
    rx: tokio::sync::mpsc::Receiver<Result<RawTuple>>,
}

impl BranchStream for SqliteReceiverStream {
    async fn next_row(&mut self) -> Result<Option<RawTuple>> {
        match self.rx.recv().await {
            None => Ok(None), // producer finished ⇒ clean EOF
            Some(Ok(tuple)) => Ok(Some(tuple)),
            Some(Err(e)) => Err(e), // forwarded marshalling/driver error (A2)
        }
    }
}

impl SqlBackend for SqliteOwnedBackend {
    type Stream<'s>
        = SqliteReceiverStream
    where
        Self: 's;

    async fn column_names(&mut self, probe_sql: &str) -> Result<Vec<String>> {
        // Lock + prepare inside `spawn_blocking`, mirroring `open_branch` below
        // (ADR-0024 §4.1) — NOT inline as this used to do. A `std::sync::Mutex`
        // taken inline in an async fn blocks the tokio WORKER THREAD itself (no
        // yield point), so `N` concurrent callers contending for one connection
        // can wedge every worker thread simultaneously: a genuine deadlock once
        // `N > worker_threads` (`sf-serve/tests/endpoint.rs`
        // `sqlite_pool_concurrency_receipt` doc + `column_names_spawn_blocking_
        // deadlock_regression`).
        let conn = Arc::clone(&self.conn);
        let lease = self.lease.clone();
        let control = self.control.clone();
        let observer = self.observer.clone();
        let probe_sql = probe_sql.to_owned();
        let joined = tokio::task::spawn_blocking(move || {
            let _lease = lease;
            let guard = conn.lock().unwrap_or_else(|p| p.into_inner());
            observer.observe(SqliteCancellationEvent::MutexAcquired);
            match control {
                Some(control) => {
                    control.checkpoint()?;
                    let cancellation =
                        SqliteCancellationGuard::install(&guard, Arc::clone(&control), observer)?;
                    crate::stream::sqlite_column_names(&guard, &probe_sql)
                        .map_err(|error| cancellation.map_error(error))
                }
                None => crate::stream::sqlite_column_names(&guard, &probe_sql),
            }
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
    ) -> Result<SqliteReceiverStream> {
        self.open_branch_with_metadata(sql, lexical_params, None)
            .await
    }

    async fn open_branch_with_metadata(
        &mut self,
        sql: &str,
        lexical_params: &[String],
        metadata_sql: Option<&str>,
    ) -> Result<SqliteReceiverStream> {
        // cap-1, FIFO (=_bag-preserving) channel: at most one buffered row in flight
        // + one `&Row` live on the blocking thread ⇒ ~2-row materialisation.
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<RawTuple>>(1);
        let conn = Arc::clone(&self.conn);
        let lease = self.lease.clone();
        let control = self.control.clone();
        let observer = self.observer.clone();
        let sql = sql.to_owned();
        let metadata_sql = metadata_sql.map(str::to_owned);
        let params: Vec<String> = lexical_params.to_vec();
        // The `!Send` Connection / Statement / Rows live ONLY on this blocking
        // thread; `blocking_send` on the cap-1 channel blocks the cursor until the
        // reactor consumes ⇒ explicit backpressure (strengthens bounded memory).
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            let guard = conn.lock().unwrap_or_else(|p| p.into_inner());
            observer.observe(SqliteCancellationEvent::MutexAcquired);
            let cancellation = match control.as_ref() {
                Some(control) => {
                    if let Err(error) = control.checkpoint() {
                        send_error(&tx, Some(control.as_ref()), Error::QueryControl(error));
                        return;
                    }
                    match SqliteCancellationGuard::install(
                        &guard,
                        Arc::clone(control),
                        observer.clone(),
                    ) {
                        Ok(cancellation) => Some(cancellation),
                        Err(error) => {
                            send_error(&tx, Some(control.as_ref()), error);
                            return;
                        }
                    }
                }
                None => None,
            };
            let (decl_codes, pads, nproj) =
                match column_meta(&guard, metadata_sql.as_deref().unwrap_or(&sql)) {
                    Ok(m) => m,
                    Err(e) => {
                        let error = match cancellation.as_ref() {
                            Some(cancellation) => cancellation.map_error(e),
                            None => e,
                        };
                        send_error(&tx, control.as_deref(), error);
                        return;
                    }
                };
            observer.observe(SqliteCancellationEvent::MetadataReady);
            if let Some(control) = control.as_ref() {
                if let Err(error) = control.checkpoint() {
                    send_error(&tx, Some(control.as_ref()), Error::QueryControl(error));
                    return;
                }
            }
            let mut stmt = match guard.prepare(&sql) {
                Ok(s) => s,
                Err(e) => {
                    let error = match cancellation.as_ref() {
                        Some(cancellation) => cancellation.map_rusqlite_error(e),
                        None => Error::Sqlite(e),
                    };
                    send_error(&tx, control.as_deref(), error);
                    return;
                }
            };
            if stmt.column_count() != nproj {
                send_error(
                    &tx,
                    control.as_deref(),
                    Error::Emit("SQLite metadata twin projection mismatch".into()),
                );
                return;
            }
            let mut rows = match stmt.query(rusqlite::params_from_iter(params.iter())) {
                Ok(r) => r,
                Err(e) => {
                    let error = match cancellation.as_ref() {
                        Some(cancellation) => cancellation.map_rusqlite_error(e),
                        None => Error::Sqlite(e),
                    };
                    send_error(&tx, control.as_deref(), error);
                    return;
                }
            };
            loop {
                match rows.next() {
                    Ok(Some(row)) => {
                        let item = marshal_row(row, &decl_codes, &pads, nproj);
                        observer.observe(SqliteCancellationEvent::BeforeRowSend);
                        let checkpoint = control.as_ref().map(|control| control.checkpoint());
                        let item = match (item, checkpoint) {
                            (Ok(_tuple), Some(Err(error))) => Err(Error::QueryControl(error)),
                            (Ok(tuple), _) => Ok(tuple),
                            (Err(error), _) => Err(error),
                        };
                        let terminal = item.is_err();
                        if tx.blocking_send(item).is_err() {
                            break; // receiver gone (cancel-on-drop) or error sent
                        }
                        if terminal {
                            break;
                        }
                    }
                    Ok(None) => {
                        observer.observe(SqliteCancellationEvent::BeforeEof);
                        if let Some(control) = control.as_ref() {
                            if let Err(error) = control.checkpoint() {
                                // The failing checkpoint is immediately before
                                // this send; clean EOF is otherwise channel close.
                                let _ = tx.blocking_send(Err(Error::QueryControl(error)));
                            }
                        }
                        break;
                    }
                    Err(e) => {
                        let error = match cancellation.as_ref() {
                            Some(cancellation) => cancellation.map_rusqlite_error(e),
                            None => Error::Sqlite(e),
                        };
                        send_error(&tx, control.as_deref(), error);
                        break;
                    }
                }
            }
        });
        Ok(SqliteReceiverStream { rx })
    }
}
