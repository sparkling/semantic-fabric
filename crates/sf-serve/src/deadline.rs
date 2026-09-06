//! Deadline-aware task helpers over the request-wide budget (ADR-0038 M2-Q1).

use std::sync::Arc;

use sf_core::query_control::QueryControlError;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
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

/// One server-wide compiler slot reserved before verified source acquisition.
///
/// The value is intentionally opaque and non-cloneable. Moving it into a
/// blocking worker keeps the slot occupied if its async waiter is cancelled or
/// reaches the request deadline.
pub(crate) struct CompilerReservation {
    _permit: OwnedSemaphorePermit,
}

/// Bound blocking compiler concurrency and its queue wait with the request's
/// deadline. The owned permit moves into the blocking closure, so timing out the
/// async waiter cannot release capacity while detached work is still queued or
/// running. The closure receives a clone of the same request budget identity.
pub(crate) async fn run_compiler<T, F>(
    budget: RequestBudget,
    permits: Arc<Semaphore>,
    work: F,
) -> Result<T, CompilerRunError>
where
    T: Send + 'static,
    F: FnOnce(RequestBudget) -> T + Send + 'static,
{
    let reservation = reserve_compiler(&budget, permits).await?;
    run_compiler_reserved_inner(budget, reservation, work, || {})
        .await
        .map(|(value, _reservation)| value)
}

/// Run bounded preflight work and retain the same compiler slot for a later
/// authoritative compile. Callers must obtain this before opening a verified
/// database transaction, so the authoritative compiler never queues while
/// relation locks are held.
pub(crate) async fn run_compiler_retaining<T, F>(
    budget: RequestBudget,
    permits: Arc<Semaphore>,
    work: F,
) -> Result<(T, CompilerReservation), CompilerRunError>
where
    T: Send + 'static,
    F: FnOnce(RequestBudget) -> T + Send + 'static,
{
    let reservation = reserve_compiler(&budget, permits).await?;
    run_compiler_reserved_inner(budget, reservation, work, || {}).await
}

/// Run authoritative work through a slot already reserved before source I/O.
pub(crate) async fn run_reserved_compiler<T, F>(
    budget: RequestBudget,
    reservation: CompilerReservation,
    work: F,
) -> Result<T, CompilerRunError>
where
    T: Send + 'static,
    F: FnOnce(RequestBudget) -> T + Send + 'static,
{
    run_compiler_reserved_inner(budget, reservation, work, || {})
        .await
        .map(|(value, _reservation)| value)
}

#[cfg(test)]
pub(crate) async fn run_compiler_observed<T, F, O>(
    budget: RequestBudget,
    permits: Arc<Semaphore>,
    work: F,
    on_submitted: O,
) -> Result<T, CompilerRunError>
where
    T: Send + 'static,
    F: FnOnce(RequestBudget) -> T + Send + 'static,
    O: FnOnce() + Send,
{
    let reservation = reserve_compiler(&budget, permits).await?;
    run_compiler_reserved_inner(budget, reservation, work, on_submitted)
        .await
        .map(|(value, _reservation)| value)
}

async fn reserve_compiler(
    budget: &RequestBudget,
    permits: Arc<Semaphore>,
) -> Result<CompilerReservation, CompilerRunError> {
    budget
        .run(permits.acquire_owned())
        .await?
        .map(|permit| CompilerReservation { _permit: permit })
        .map_err(|_| CompilerRunError::AdmissionClosed)
}

async fn run_compiler_reserved_inner<T, F, O>(
    budget: RequestBudget,
    reservation: CompilerReservation,
    work: F,
    on_submitted: O,
) -> Result<(T, CompilerReservation), CompilerRunError>
where
    T: Send + 'static,
    F: FnOnce(RequestBudget) -> T + Send + 'static,
    O: FnOnce() + Send,
{
    let worker_budget = budget.clone();
    let task = tokio::task::spawn_blocking(move || {
        // A timed-out async waiter must not return aggregate request capacity
        // while its detached compiler is still queued or running.
        let worker_lifetime = worker_budget.clone();
        let value = work(worker_budget);
        (value, reservation, worker_lifetime)
    });
    on_submitted();

    match budget.run(task).await {
        Err(error) => Err(error.into()),
        Ok(Err(error)) => Err(CompilerRunError::Join(error)),
        Ok(Ok((value, reservation, _worker_lifetime))) => Ok((value, reservation)),
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
