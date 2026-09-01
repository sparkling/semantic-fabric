//! Absolute request-deadline primitives (ADR-0038 partial M2).
//!
//! This is intentionally smaller than the future `QueryBudget`: it bounds elapsed
//! request time and concurrent blocking compilers, but does not count source work,
//! rows, bytes, or recursion. Every phase receives the same absolute instant.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use sf_core::query_control::{QueryControl, QueryControlError};
use tokio::sync::Semaphore;
use tokio::task::{JoinError, JoinHandle};
use tokio::time::Instant;

/// The stable public-safe message for an elapsed request.
pub(crate) const TIMEOUT_MESSAGE: &str = "request timeout (ADR-0010)";

/// One absolute request deadline, minted once at ingress.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RequestDeadline(Instant);

/// The request's absolute deadline elapsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{TIMEOUT_MESSAGE}")]
pub(crate) struct DeadlineExceeded;

impl RequestDeadline {
    pub(crate) fn after(timeout: Duration) -> Self {
        Self(Instant::now() + timeout)
    }

    pub(crate) fn into_std(self) -> std::time::Instant {
        self.0.into_std()
    }

    /// Await `future` only until this request's original absolute deadline.
    pub(crate) async fn run<F>(self, future: F) -> Result<F::Output, DeadlineExceeded>
    where
        F: Future,
    {
        tokio::time::timeout_at(self.0, future)
            .await
            .map_err(|_| DeadlineExceeded)
    }

    pub(crate) fn check(self) -> Result<(), DeadlineExceeded> {
        self.check_at(Instant::now()).map_err(|_| DeadlineExceeded)
    }
}

impl QueryControl for RequestDeadline {
    type Instant = Instant;

    fn check_at(&self, now: Instant) -> Result<(), QueryControlError> {
        if now >= self.0 {
            Err(QueryControlError::DeadlineExceeded)
        } else {
            Ok(())
        }
    }
}

/// Failure before a blocking compiler produces its value.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CompilerRunError {
    #[error(transparent)]
    Deadline(#[from] DeadlineExceeded),
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
    deadline: RequestDeadline,
    permits: Arc<Semaphore>,
    work: F,
) -> Result<T, CompilerRunError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let permit = deadline
        .run(permits.acquire_owned())
        .await?
        .map_err(|_| CompilerRunError::AdmissionClosed)?;
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    });

    match deadline.run(task).await {
        Err(error) => Err(error.into()),
        Ok(Err(error)) => Err(CompilerRunError::Join(error)),
        Ok(Ok(value)) => Ok(value),
    }
}

/// Failure while awaiting an abortable request task.
#[derive(Debug, thiserror::Error)]
pub(crate) enum JoinedTaskError {
    #[error(transparent)]
    Deadline(#[from] DeadlineExceeded),
    #[error("request task join error: {0}")]
    Join(JoinError),
}

/// Await an ordinary request task at the absolute deadline and abort it when the
/// waiter times out or is itself cancelled. Blocking compiler tasks deliberately
/// use [`run_compiler`] instead: their permit must outlive a detached waiter.
pub(crate) async fn join_task<T>(
    deadline: RequestDeadline,
    task: JoinHandle<T>,
) -> Result<T, JoinedTaskError> {
    struct AbortOnDrop<T>(JoinHandle<T>);
    impl<T> Drop for AbortOnDrop<T> {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    let mut guarded = AbortOnDrop(task);
    match deadline.run(&mut guarded.0).await {
        Err(error) => Err(error.into()),
        Ok(Err(error)) => Err(JoinedTaskError::Join(error)),
        Ok(Ok(value)) => Ok(value),
    }
}
