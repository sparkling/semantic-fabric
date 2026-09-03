//! Query-local SQLite VM cancellation and exact-cause classification.

use std::sync::{Arc, OnceLock};

use rusqlite::{Connection, ErrorCode};
use sf_core::query_control::{QueryControl, QueryControlError};

use crate::error::Error;

/// Balance cancellation latency against callback overhead. The callback only
/// checkpoints; VM instructions are deliberately not source-work charges.
const PROGRESS_INTERVAL_OPS: i32 = 1_000;

/// Deterministic lifecycle events exposed only inside SQLite unit tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SqliteCancellationEvent {
    MutexAcquired,
    ProgressCallbackEntered,
    MetadataReady,
    BeforeRowSend,
    BeforeEof,
}

/// Private observer for proving cancellation lifecycle ordering. Callback
/// storage exists only in unit-test builds; the production representation is
/// zero-sized and `observe` compiles to a no-op.
#[derive(Clone, Default)]
pub(super) struct SqliteCancellationObserver {
    #[cfg(test)]
    callback: Option<Arc<dyn Fn(SqliteCancellationEvent) + Send + Sync>>,
}

impl SqliteCancellationObserver {
    #[cfg(test)]
    pub(super) fn new(callback: Arc<dyn Fn(SqliteCancellationEvent) + Send + Sync>) -> Self {
        Self {
            callback: Some(callback),
        }
    }

    #[inline(always)]
    pub(super) fn observe(&self, event: SqliteCancellationEvent) {
        #[cfg(not(test))]
        let _ = event;
        #[cfg(test)]
        if let Some(callback) = self.callback.as_ref() {
            callback(event);
        }
    }
}

/// Owns one connection-global progress handler for exactly one locked operation.
///
/// Declare this after acquiring the connection mutex and before statements/rows.
/// Rust's reverse drop order then destroys rows and statements first, removes the
/// handler here, and only afterward releases the connection mutex.
pub(super) struct SqliteCancellationGuard<'connection> {
    connection: &'connection Connection,
    callback_cause: Arc<OnceLock<QueryControlError>>,
}

impl<'connection> SqliteCancellationGuard<'connection> {
    pub(super) fn install(
        connection: &'connection Connection,
        control: Arc<dyn QueryControl>,
        observer: SqliteCancellationObserver,
    ) -> Result<Self, Error> {
        let callback_cause = Arc::new(OnceLock::new());
        let callback_marker = Arc::clone(&callback_cause);
        connection.progress_handler(
            PROGRESS_INTERVAL_OPS,
            Some(move || {
                observer.observe(SqliteCancellationEvent::ProgressCallbackEntered);
                match control.checkpoint() {
                    Ok(()) => false,
                    Err(cause) => {
                        let _ = callback_marker.set(cause);
                        true
                    }
                }
            }),
        )?;
        Ok(Self {
            connection,
            callback_cause,
        })
    }

    /// Translate `SQLITE_INTERRUPT` only when this operation's own callback
    /// recorded why it requested the interrupt. External interrupts remain
    /// ordinary SQLite driver failures.
    pub(super) fn map_rusqlite_error(&self, error: rusqlite::Error) -> Error {
        if is_operation_interrupted(&error) {
            if let Some(cause) = self.callback_cause.get() {
                return Error::QueryControl(*cause);
            }
        }
        Error::Sqlite(error)
    }

    pub(super) fn map_error(&self, error: Error) -> Error {
        match error {
            Error::Sqlite(error) => self.map_rusqlite_error(error),
            other => other,
        }
    }
}

impl Drop for SqliteCancellationGuard<'_> {
    fn drop(&mut self) {
        // `num_ops < 1` disables the handler. Ignore a teardown diagnostic in
        // Drop; the locked connection cannot be reused until this call returns.
        let _ = self.connection.progress_handler(0, None::<fn() -> bool>);
    }
}

fn is_operation_interrupted(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(code, _) if code.code == ErrorCode::OperationInterrupted
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::query_control::{QueryBudget, QueryLimits};

    #[test]
    fn guard_removes_progress_handler_during_unwind() {
        let connection = Connection::open_in_memory().unwrap();
        let budget = Arc::new(QueryBudget::new(QueryLimits::new(
            u64::MAX,
            u64::MAX,
            u64::MAX,
        )));
        let control: Arc<dyn QueryControl> = budget.clone();

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = SqliteCancellationGuard::install(
                &connection,
                control,
                SqliteCancellationObserver::default(),
            )
            .unwrap();
            panic!("exercise guard teardown");
        }));
        assert!(unwound.is_err());

        budget.terminate(QueryControlError::Cancelled);
        let max: i64 = connection
            .query_row(
                "WITH RECURSIVE c(x) AS (VALUES(0) UNION ALL \
                 SELECT x + 1 FROM c WHERE x < 10000) SELECT max(x) FROM c",
                [],
                |row| row.get(0),
            )
            .expect("unwind removed the old operation's progress handler");
        assert_eq!(max, 10000);
    }

    #[test]
    fn empty_operation_marker_never_reads_later_budget_state() {
        let connection = Connection::open_in_memory().unwrap();
        let budget = Arc::new(QueryBudget::new(QueryLimits::new(
            u64::MAX,
            u64::MAX,
            u64::MAX,
        )));
        let control: Arc<dyn QueryControl> = budget.clone();
        let guard = SqliteCancellationGuard::install(
            &connection,
            control,
            SqliteCancellationObserver::default(),
        )
        .unwrap();

        budget.terminate(QueryControlError::Cancelled);
        let interrupted = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_INTERRUPT),
            None,
        );
        assert!(matches!(
            guard.map_rusqlite_error(interrupted),
            Error::Sqlite(rusqlite::Error::SqliteFailure(code, _))
                if code.code == ErrorCode::OperationInterrupted
        ));
    }
}
