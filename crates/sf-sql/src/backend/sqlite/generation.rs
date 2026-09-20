//! One read transaction spanning schema admission and every execution worker.

use std::sync::{Arc, Mutex};

use sf_core::query_control::QueryControl;
use tokio::sync::Notify;

use super::cancellation::{SqliteCancellationGuard, SqliteCancellationObserver};
use super::generation_schema::SqliteGenerationSchema;
use super::owned::{
    SqliteOwnedBackend, SqliteOwnedConnection, SqliteOwnedLease, SqliteOwnedLeaseState,
};
use crate::source_work::SourceWork;
use crate::{Error, Result};

/// A physical member exclusively owned by the verified-generation lane. Unlike
/// the legacy serving member, this type never exports a raw connection or lease.
#[derive(Clone)]
pub struct SqliteGenerationConnection(SqliteOwnedConnection);

impl SqliteGenerationConnection {
    /// Own connection creation so URI/VFS options cannot disable snapshot locking.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.as_os_str().to_string_lossy().starts_with("file:") {
            return Err(invalid("URI connections are not admitted"));
        }
        let path = path
            .canonicalize()
            .map_err(|_| invalid("source file unavailable"))?;
        let connection = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Ok(Self(SqliteOwnedConnection::new(connection)))
    }

    /// Callers govern the asynchronous admission wait with their request control.
    /// Dropping that wait never releases an already submitted worker.
    pub async fn begin(
        &self,
        control: Arc<dyn QueryControl>,
        expected: Option<Arc<SqliteGenerationSchema>>,
    ) -> Result<VerifiedSqliteGenerationLease> {
        control.checkpoint()?;
        let lease = self
            .0
            .acquire()
            .await
            .map_err(|_| invalid("member is poisoned"))?;
        VerifiedSqliteGenerationLease::begin(lease, control, expected).await
    }
}

#[derive(Default)]
struct Operations {
    closing: bool,
    active: usize,
    failed: bool,
    clean: bool,
}

pub(super) struct GenerationState {
    // Keep request ownership until cleanup, before finally releasing admission.
    control: Arc<dyn QueryControl>,
    lease: Arc<SqliteOwnedLeaseState>,
    operations: Mutex<Operations>,
    changed: Notify,
}

impl GenerationState {
    pub(super) fn register(self: &Arc<Self>) -> Result<Operation> {
        let mut operations = self.operations.lock().unwrap_or_else(|p| p.into_inner());
        if operations.closing || operations.failed {
            return Err(invalid("generation is closing or poisoned"));
        }
        operations.active = operations
            .active
            .checked_add(1)
            .ok_or_else(|| invalid("operation count overflow"))?;
        Ok(Operation(self.clone()))
    }

    fn close(&self) {
        self.operations
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .closing = true;
    }

    async fn drain(&self) {
        loop {
            let changed = self.changed.notified();
            if self
                .operations
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .active
                == 0
            {
                return;
            }
            changed.await;
        }
    }
}

impl Drop for GenerationState {
    fn drop(&mut self) {
        if !self
            .operations
            .get_mut()
            .unwrap_or_else(|p| p.into_inner())
            .clean
        {
            // A runtime teardown, panic or failed rollback can never recycle this member.
            self.lease.poison();
        }
    }
}

pub(super) struct Operation(Arc<GenerationState>);

impl Drop for Operation {
    fn drop(&mut self) {
        let mut operations = self.0.operations.lock().unwrap_or_else(|p| p.into_inner());
        operations.active -= 1;
        operations.failed |= std::thread::panicking();
        drop(operations);
        self.0.changed.notify_one();
    }
}

/// An opaque read transaction. Dropping it schedules rollback; only `finish`
/// acknowledges schema revalidation and completed cleanup to the caller.
pub struct VerifiedSqliteGenerationLease {
    state: Option<Arc<GenerationState>>,
    schema: Arc<SqliteGenerationSchema>,
}

impl VerifiedSqliteGenerationLease {
    /// Capture a file-backed schema, optionally matching a previously activated
    /// expectation, before authoritative compilation. Admission is already held.
    pub(super) async fn begin(
        lease: SqliteOwnedLease,
        control: Arc<dyn QueryControl>,
        expected: Option<Arc<SqliteGenerationSchema>>,
    ) -> Result<Self> {
        let state = Arc::new(GenerationState {
            control,
            lease: lease.state,
            operations: Mutex::new(Operations::default()),
            changed: Notify::new(),
        });
        tokio::task::spawn_blocking(move || {
            let schema = {
                let conn = state
                    .lease
                    .conn
                    .lock()
                    .map_err(|_| invalid("poisoned connection"))?;
                if !conn.is_autocommit() {
                    return Err(invalid("connection already has a transaction"));
                }
                // Cancellation before BEGIN leaves a verified clean member reusable.
                state
                    .operations
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clean = true;
                state.control.checkpoint()?;
                state
                    .operations
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clean = false;
                if let Err(error) = conn.execute_batch("BEGIN DEFERRED") {
                    state
                        .operations
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clean = conn.is_autocommit();
                    return Err(error.into());
                }
                let result = (|| {
                    let cancellation = SqliteCancellationGuard::install(
                        &conn,
                        state.control.clone(),
                        SqliteCancellationObserver::default(),
                    )?;
                    let schema = SqliteGenerationSchema::observe(
                        &conn,
                        SourceWork::new(Some(state.control.as_ref())),
                    )
                    .map_err(|error| cancellation.map_error(error))?;
                    if expected
                        .as_ref()
                        .is_some_and(|expected| expected.as_ref() != &schema)
                    {
                        return Err(invalid("activated schema no longer matches"));
                    }
                    Ok(schema)
                })();
                match result {
                    Ok(schema) => schema,
                    Err(error) => {
                        if conn.execute_batch("ROLLBACK").is_ok() && conn.is_autocommit() {
                            state
                                .operations
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .clean = true;
                        }
                        return Err(error);
                    }
                }
            };
            Ok(Self {
                state: Some(state),
                schema: Arc::new(schema),
            })
        })
        .await
        .map_err(|e| invalid(&format!("begin worker failed: {e}")))?
    }

    pub fn schema(&self) -> &Arc<SqliteGenerationSchema> {
        &self.schema
    }

    /// This view shares the generation's operation barrier and physical member.
    pub fn backend(&self) -> Result<SqliteOwnedBackend> {
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| invalid("generation finished"))?;
        let mut backend = SqliteOwnedBackend::new_controlled_leased(
            SqliteOwnedLease {
                state: state.lease.clone(),
            },
            state.control.clone(),
        );
        backend.generation = Some(state.clone());
        Ok(backend)
    }

    pub async fn finish(mut self) -> Result<()> {
        let state = self
            .state
            .take()
            .ok_or_else(|| invalid("generation finished"))?;
        let expected = self.schema.clone();
        state.close();
        // Detach cleanup from cancellation of the caller awaiting its acknowledgement.
        tokio::spawn(cleanup(state, Some(expected)))
            .await
            .map_err(|e| invalid(&format!("cleanup worker failed: {e}")))?
    }
}

impl Drop for VerifiedSqliteGenerationLease {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            state.close();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(cleanup(state, None));
            }
            // Without a runtime, GenerationState's destructor poisons admission.
        }
    }
}

async fn cleanup(
    state: Arc<GenerationState>,
    expected: Option<Arc<SqliteGenerationSchema>>,
) -> Result<()> {
    state.drain().await;
    tokio::task::spawn_blocking(move || {
        let conn = state
            .lease
            .conn
            .lock()
            .map_err(|_| invalid("poisoned connection"))?;
        let validation = match expected {
            Some(expected) => (|| {
                if conn.is_autocommit() {
                    return Err(invalid("transaction ended early"));
                }
                let cancellation = SqliteCancellationGuard::install(
                    &conn,
                    state.control.clone(),
                    SqliteCancellationObserver::default(),
                )?;
                let actual = SqliteGenerationSchema::observe(
                    &conn,
                    SourceWork::new(Some(state.control.as_ref())),
                )
                .map_err(|error| cancellation.map_error(error))?;
                if &actual != expected.as_ref() {
                    return Err(invalid("schema changed within transaction"));
                }
                Ok(())
            })(),
            None => Ok(()),
        };
        // Cleanup is mandatory even after deadline/cancellation. No request VM
        // callback may interrupt rollback and allow uncertain state to be reused.
        conn.progress_handler(0, None::<fn() -> bool>)?;
        conn.execute_batch("ROLLBACK")?;
        if !conn.is_autocommit() {
            return Err(invalid("rollback did not end transaction"));
        }
        let mut operations = state.operations.lock().unwrap_or_else(|p| p.into_inner());
        if operations.failed {
            return Err(invalid("execution worker panicked"));
        }
        operations.clean = true;
        validation
    })
    .await
    .map_err(|e| invalid(&format!("rollback worker failed: {e}")))?
}

fn invalid(reason: &str) -> Error {
    Error::Introspection(format!("SQLite generation: {reason}"))
}

#[cfg(test)]
#[path = "generation_tests.rs"]
mod tests;
