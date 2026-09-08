//! Native candidate work stays owned until actual completion, not just timeout.

use super::{PgGenerationError, RequestBudget};
use sf_core::query_control::QueryControl;

pub(crate) async fn run<T, F>(budget: &RequestBudget, work: F) -> Result<T, PgGenerationError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    budget.checkpoint()?;
    let owned_budget = budget.clone();
    let worker = tokio::task::spawn_blocking(move || {
        // Keep the operation identity even if the parent task is interrupted.
        let _owned_budget = owned_budget;
        work()
    });
    let result = match worker.await {
        Ok(result) => result,
        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        Err(_) => return Err(PgGenerationError::Internal),
    };
    budget.checkpoint()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::query_control::{QueryControlError, QueryLimits};
    use std::time::Duration;
    use tokio::sync::oneshot;

    async fn settle() {
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn expired_budget_keeps_physical_worker_owned_until_completion() {
        let budget = RequestBudget::after(Duration::from_secs(60), QueryLimits::new(0, 0, 0, 0));
        let (started, start) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let worker = tokio::spawn(async move {
            run(&budget, move || {
                started.send(()).unwrap();
                // Sender drop also releases this fixture on an assertion failure.
                let _ = released.blocking_recv();
                42
            })
            .await
        });
        start.await.unwrap();
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(61)).await;
        settle().await;
        assert!(
            !worker.is_finished(),
            "deadline must not detach physical work"
        );
        release.send(()).unwrap();
        assert!(matches!(
            worker.await.unwrap(),
            Err(PgGenerationError::Control(
                QueryControlError::DeadlineExceeded
            ))
        ));
    }

    #[tokio::test]
    async fn panic_after_deadline_remains_a_panic_not_a_retryable_source_failure() {
        let budget = RequestBudget::after(Duration::from_secs(60), QueryLimits::new(0, 0, 0, 0));
        let (started, start) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let worker = tokio::spawn(async move {
            run(&budget, move || {
                started.send(()).unwrap();
                let _ = released.blocking_recv();
                panic!("candidate worker fault");
            })
            .await
        });
        start.await.unwrap();
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(61)).await;
        release.send(()).unwrap();
        assert!(worker
            .await
            .expect_err("panic must reach supervisor")
            .is_panic());
    }

    #[tokio::test]
    async fn cancelled_before_submission_does_not_execute_work() {
        let budget = RequestBudget::after(Duration::from_secs(60), QueryLimits::new(0, 0, 0, 0));
        budget.terminate(QueryControlError::Cancelled);
        assert!(matches!(
            run(&budget, || panic!("cancelled work must not start")).await,
            Err(PgGenerationError::Control(QueryControlError::Cancelled))
        ));
    }

    #[tokio::test]
    async fn panic_before_deadline_is_not_retryable() {
        let budget = RequestBudget::after(Duration::from_secs(60), QueryLimits::new(0, 0, 0, 0));
        let worker = tokio::spawn(async move { run(&budget, || panic!("candidate fault")).await });
        assert!(worker.await.unwrap_err().is_panic());
    }
}
