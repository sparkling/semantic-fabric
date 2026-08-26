//! Bounded admission and cooperative cancellation for synchronous compilation.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sf_sparql::{with_compile_control, CompileCancellation, CompileControl, Error as SparqlError};
use tokio::sync::Semaphore;

/// Failure modes the HTTP layer maps to its stable protocol responses.
#[derive(Debug)]
pub(crate) enum CompileFailure {
    AdmissionTimeout,
    Unavailable,
    Deadline,
    Join(tokio::task::JoinError),
    Planner(SparqlError),
}

struct CancelOnDrop(CompileCancellation);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Runs one synchronous compiler behind bounded admission and one absolute
/// deadline. The permit lives in the blocking worker, so a timed-out or dropped
/// caller cannot accidentally admit replacement work until cooperative
/// cancellation has actually stopped the abandoned compilation.
pub(crate) async fn run<T, F>(
    permits: Arc<Semaphore>,
    timeout: Duration,
    max_expansion_work: usize,
    compile: F,
) -> Result<T, CompileFailure>
where
    T: Send + 'static,
    F: FnOnce(CompileCancellation) -> sf_sparql::Result<T> + Send + 'static,
{
    let started = Instant::now();
    let permit = match tokio::time::timeout(timeout, permits.acquire_owned()).await {
        Err(_) => return Err(CompileFailure::AdmissionTimeout),
        Ok(Err(_)) => return Err(CompileFailure::Unavailable),
        Ok(Ok(permit)) => permit,
    };

    let remaining = timeout.saturating_sub(started.elapsed());
    let cancellation = CompileCancellation::new();
    let cancel_on_drop = CancelOnDrop(cancellation.clone());
    let control = CompileControl::new(cancellation.clone(), max_expansion_work);
    let mut worker = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        with_compile_control(control, || compile(cancellation))
    });

    let joined = match tokio::time::timeout(remaining, &mut worker).await {
        Ok(joined) => joined,
        Err(_) => {
            cancel_on_drop.0.cancel();
            return Err(CompileFailure::Deadline);
        }
    };
    match joined {
        Err(error) => Err(CompileFailure::Join(error)),
        Ok(Err(error)) => Err(CompileFailure::Planner(error)),
        Ok(Ok(value)) => Ok(value),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    fn waiting_compile(
        started: Arc<AtomicBool>,
    ) -> impl FnOnce(CompileCancellation) -> sf_sparql::Result<()> + Send + 'static {
        move |cancellation| {
            started.store(true, Ordering::Release);
            while !cancellation.is_cancelled() {
                std::thread::yield_now();
            }
            Err(SparqlError::Cancelled)
        }
    }

    async fn wait_until_started(started: &AtomicBool) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while !started.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("blocking compiler should start");
    }

    async fn prove_replacement_is_admitted(permits: Arc<Semaphore>) {
        let value = run(
            Arc::clone(&permits),
            Duration::from_secs(1),
            usize::MAX,
            |_| Ok(7usize),
        )
        .await
        .expect("cancelled compiler must release its permit");
        assert_eq!(value, 7);
        assert_eq!(permits.available_permits(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn deadline_cancels_worker_before_permit_is_reused() {
        let permits = Arc::new(Semaphore::new(1));
        let started = Arc::new(AtomicBool::new(false));
        let first = tokio::spawn(run(
            Arc::clone(&permits),
            Duration::from_millis(200),
            usize::MAX,
            waiting_compile(Arc::clone(&started)),
        ));
        wait_until_started(&started).await;

        assert!(matches!(
            first.await.expect("compiler task joins"),
            Err(CompileFailure::Deadline)
        ));
        prove_replacement_is_admitted(permits).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropped_caller_cancels_worker_and_releases_permit() {
        let permits = Arc::new(Semaphore::new(1));
        let started = Arc::new(AtomicBool::new(false));
        let first = tokio::spawn(run(
            Arc::clone(&permits),
            Duration::from_secs(30),
            usize::MAX,
            waiting_compile(Arc::clone(&started)),
        ));
        wait_until_started(&started).await;
        first.abort();
        assert!(first
            .await
            .expect_err("caller task should be cancelled")
            .is_cancelled());

        prove_replacement_is_admitted(permits).await;
    }
}
