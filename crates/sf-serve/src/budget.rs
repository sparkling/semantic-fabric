//! One request-scoped deadline, cancellation signal, and accounting identity.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use tokio::sync::watch;
use tokio::time::Instant;

struct RequestBudgetState {
    accounting: QueryBudget,
    deadline: Option<Instant>,
    terminal: watch::Sender<Option<QueryControlError>>,
}

/// The single governance identity minted before request-body extraction.
#[derive(Clone)]
pub(crate) struct RequestBudget(Arc<RequestBudgetState>);

pub(crate) struct CancellationGuard(Option<RequestBudget>);

impl RequestBudget {
    pub(crate) fn after(timeout: Duration, limits: QueryLimits) -> Self {
        let now = Instant::now();
        let (terminal, _) = watch::channel(None);
        let overflow = now.checked_add(timeout).is_none();
        let request = Self(Arc::new(RequestBudgetState {
            accounting: QueryBudget::new(limits),
            deadline: Some(now.checked_add(timeout).unwrap_or(now)),
            terminal,
        }));
        if overflow {
            request.terminate(QueryControlError::AccountingOverflow);
        }
        request
    }

    pub(crate) fn uncontrolled(deadline: Option<std::time::Instant>) -> Self {
        let (terminal, _) = watch::channel(None);
        Self(Arc::new(RequestBudgetState {
            accounting: QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX)),
            deadline: deadline.map(Instant::from_std),
            terminal,
        }))
    }

    /// Await a phase without refreshing the original absolute deadline.
    pub(crate) async fn run<F>(&self, future: F) -> Result<F::Output, QueryControlError>
    where
        F: Future,
    {
        self.checkpoint()?;
        let mut terminal = self.0.terminal.subscribe();
        self.checkpoint()?;
        tokio::select! {
            biased;
            _ = wait_for_deadline(self.0.deadline) => {
                Err(self.terminate(QueryControlError::DeadlineExceeded))
            }
            changed = terminal.changed() => {
                let reason = if changed.is_ok() {
                    *terminal.borrow_and_update()
                } else {
                    None
                };
                Err(reason
                    .or_else(|| self.0.accounting.terminal())
                    .unwrap_or(QueryControlError::AccountingOverflow))
            }
            output = future => {
                self.checkpoint()?;
                Ok(output)
            }
        }
    }

    /// Race ingress/handler work only against the absolute clock. A streaming
    /// producer may seal a result/work limit immediately after the handler builds
    /// its response; ignoring non-deadline terminals here keeps the status-line
    /// handoff deterministic (stream failures are always post-200).
    pub(crate) async fn run_until_deadline<F>(
        &self,
        future: F,
    ) -> Result<F::Output, QueryControlError>
    where
        F: Future,
    {
        if self
            .0
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(self.terminate(QueryControlError::DeadlineExceeded));
        }
        tokio::select! {
            biased;
            _ = wait_for_deadline(self.0.deadline) => {
                Err(self.terminate(QueryControlError::DeadlineExceeded))
            }
            output = future => Ok(output),
        }
    }

    pub(crate) fn cancel(&self) -> QueryControlError {
        self.terminate(QueryControlError::Cancelled)
    }

    pub(crate) fn cancellation_guard(&self) -> CancellationGuard {
        CancellationGuard(Some(self.clone()))
    }

    #[cfg(test)]
    pub(crate) fn consumed(&self, charge: QueryCharge) -> u64 {
        self.0.accounting.consumed(charge)
    }

    pub(crate) fn check_at(&self, now: Instant) -> Result<(), QueryControlError> {
        if let Some(reason) = self.0.accounting.terminal() {
            return Err(reason);
        }
        if self.0.deadline.is_some_and(|deadline| now >= deadline) {
            return Err(self.terminate(QueryControlError::DeadlineExceeded));
        }
        Ok(())
    }

    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        let reason = self.0.accounting.terminate(reason);
        self.0.terminal.send_replace(Some(reason));
        reason
    }

    fn signal_error(&self, error: QueryControlError) -> QueryControlError {
        let error = self.0.accounting.terminal().unwrap_or(error);
        self.0.terminal.send_replace(Some(error));
        error
    }
}

impl CancellationGuard {
    pub(crate) fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for CancellationGuard {
    fn drop(&mut self) {
        if let Some(budget) = self.0.take() {
            budget.cancel();
        }
    }
}

async fn wait_for_deadline(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

impl QueryControl for RequestBudget {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        self.check_at(Instant::now())
    }

    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        self.checkpoint()?;
        self.0
            .accounting
            .consume(charge, amount)
            .map_err(|error| self.signal_error(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn cancellation_wakes_a_pending_phase_and_is_sticky() {
        let budget = RequestBudget::after(
            Duration::from_secs(60),
            QueryLimits::new(u64::MAX, u64::MAX, u64::MAX),
        );
        let waiter = tokio::spawn({
            let budget = budget.clone();
            async move { budget.run(std::future::pending::<()>()).await }
        });
        tokio::task::yield_now().await;

        assert_eq!(budget.cancel(), QueryControlError::Cancelled);
        assert_eq!(
            waiter.await.expect("waiter task"),
            Err(QueryControlError::Cancelled)
        );
        assert_eq!(budget.checkpoint(), Err(QueryControlError::Cancelled));
    }

    #[tokio::test(start_paused = true)]
    async fn limit_failure_wakes_an_independent_pending_phase() {
        let budget = RequestBudget::after(
            Duration::from_secs(60),
            QueryLimits::new(0, u64::MAX, u64::MAX),
        );
        let waiter = tokio::spawn({
            let budget = budget.clone();
            async move { budget.run(std::future::pending::<()>()).await }
        });
        tokio::task::yield_now().await;

        assert_eq!(
            budget.consume(QueryCharge::SourceWork, 1),
            Err(QueryControlError::SourceWorkExceeded)
        );
        assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
        assert_eq!(
            waiter.await.expect("waiter task"),
            Err(QueryControlError::SourceWorkExceeded)
        );
    }
}
