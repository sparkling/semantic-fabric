//! Deadline-aware task helpers over the request-wide budget (ADR-0038 M2-Q1).

use std::sync::Arc;

use sf_core::query_control::QueryControlError;
use tokio::sync::Semaphore;
use tokio::task::{JoinError, JoinHandle};

use crate::budget::RequestBudget;

/// Failure before a blocking compiler produces its value.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CompilerRunError {
    #[error(transparent)]
    Control(#[from] QueryControlError),
    #[error("compiler admission is closed")]
    AdmissionClosed,
    #[error("compiler task join error: {0}")]
    Join(JoinError),
}

/// Bound blocking compiler concurrency and its queue wait with the request's
/// deadline. The owned permit moves into the blocking closure, so timing out the
/// async waiter cannot release capacity while detached work is still queued or
/// running.
pub(crate) async fn run_compiler<T, F>(
    budget: RequestBudget,
    permits: Arc<Semaphore>,
    work: F,
) -> Result<T, CompilerRunError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let permit = budget
        .run(permits.acquire_owned())
        .await?
        .map_err(|_| CompilerRunError::AdmissionClosed)?;
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    });

    match budget.run(task).await {
        Err(error) => Err(error.into()),
        Ok(Err(error)) => Err(CompilerRunError::Join(error)),
        Ok(Ok(value)) => Ok(value),
    }
}

/// Failure while awaiting an abortable request task.
#[derive(Debug, thiserror::Error)]
pub(crate) enum JoinedTaskError {
    #[error(transparent)]
    Control(#[from] QueryControlError),
    #[error("request task join error: {0}")]
    Join(JoinError),
}

/// Await an ordinary request task at the absolute deadline and abort it when the
/// waiter times out or is itself cancelled. Blocking compiler tasks deliberately
/// use [`run_compiler`] instead: their permit must outlive a detached waiter.
pub(crate) async fn join_task<T>(
    budget: RequestBudget,
    task: JoinHandle<T>,
) -> Result<T, JoinedTaskError> {
    struct AbortOnDrop<T>(JoinHandle<T>);
    impl<T> Drop for AbortOnDrop<T> {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    let mut guarded = AbortOnDrop(task);
    match budget.run(&mut guarded.0).await {
        Err(error) => Err(error.into()),
        Ok(Err(error)) => Err(JoinedTaskError::Join(error)),
        Ok(Ok(value)) => Ok(value),
    }
}
