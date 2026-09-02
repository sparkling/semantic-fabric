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
    deadline_representable: bool,
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
        let deadline = now.checked_add(timeout);
        let request = Self(Arc::new(RequestBudgetState {
            accounting: QueryBudget::new(limits),
            deadline: Some(deadline.unwrap_or(now)),
            deadline_representable: deadline.is_some(),
            terminal,
        }));
        if deadline.is_none() {
            request.terminate(QueryControlError::AccountingOverflow);
        }
        request
    }

    pub(crate) fn uncontrolled(deadline: Option<std::time::Instant>) -> Self {
        let (terminal, _) = watch::channel(None);
        Self(Arc::new(RequestBudgetState {
            accounting: QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX)),
            deadline: deadline.map(Instant::from_std),
            deadline_representable: true,
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
            return Err(self.handoff_deadline_error());
        }
        tokio::select! {
            biased;
            _ = wait_for_deadline(self.0.deadline) => {
                Err(self.handoff_deadline_error())
            }
            output = future => {
                self.check_handoff_deadline_at(Instant::now())?;
                Ok(output)
            },
        }
    }

    pub(crate) fn cancel(&self) -> QueryControlError {
        let reason = if self.deadline_reached(Instant::now()) {
            QueryControlError::DeadlineExceeded
        } else {
            QueryControlError::Cancelled
        };
        self.terminate(reason)
    }

    pub(crate) fn cancellation_guard(&self) -> CancellationGuard {
        CancellationGuard(Some(self.clone()))
    }

    /// Reject an ASK whose guaranteed boolean cannot fit before any backend is
    /// selected or acquired. A positive capacity is charged by the executor.
    pub(crate) fn preflight_ask_result(&self) -> Result<(), QueryControlError> {
        self.checkpoint()?;
        if self.0.accounting.consumed(QueryCharge::ResultItems)
            >= self.0.accounting.limits().max_result_items()
        {
            return self.consume(QueryCharge::ResultItems, 1);
        }
        Ok(())
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

    fn deadline_reached(&self, now: Instant) -> bool {
        self.0.deadline.is_some_and(|deadline| now >= deadline)
    }

    fn check_handoff_deadline_at(&self, now: Instant) -> Result<(), QueryControlError> {
        if self.deadline_reached(now) {
            return Err(self.handoff_deadline_error());
        }
        match self.0.accounting.terminal() {
            Some(QueryControlError::DeadlineExceeded) => Err(QueryControlError::DeadlineExceeded),
            _ => Ok(()),
        }
    }

    /// Classify an expired, representable handoff clock independently from the
    /// sticky first accounting cause. The latter remains available internally.
    fn handoff_deadline_error(&self) -> QueryControlError {
        if !self.0.deadline_representable {
            return self
                .0
                .accounting
                .terminal()
                .unwrap_or(QueryControlError::AccountingOverflow);
        }
        let _ = self.terminate(QueryControlError::DeadlineExceeded);
        QueryControlError::DeadlineExceeded
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

    #[tokio::test(flavor = "current_thread")]
    async fn handler_output_is_rechecked_against_the_absolute_deadline() {
        let budget = RequestBudget::after(
            Duration::from_millis(1),
            QueryLimits::new(u64::MAX, u64::MAX, u64::MAX),
        );

        let result = budget
            .run_until_deadline(async {
                std::thread::sleep(Duration::from_millis(20));
                "response"
            })
            .await;

        assert_eq!(result, Err(QueryControlError::DeadlineExceeded));
    }

    #[tokio::test(start_paused = true)]
    async fn ready_handoff_before_deadline_ignores_a_sticky_resource_failure() {
        let budget = RequestBudget::after(
            Duration::from_secs(5),
            QueryLimits::new(0, u64::MAX, u64::MAX),
        );
        assert_eq!(
            budget.consume(QueryCharge::SourceWork, 1),
            Err(QueryControlError::SourceWorkExceeded)
        );

        assert_eq!(
            budget.run_until_deadline(async { "response" }).await,
            Ok("response")
        );
        assert_eq!(
            budget.checkpoint(),
            Err(QueryControlError::SourceWorkExceeded)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn expired_handoff_reports_deadline_while_accounting_keeps_first_cause() {
        let budget = RequestBudget::after(
            Duration::from_secs(5),
            QueryLimits::new(0, u64::MAX, u64::MAX),
        );
        assert_eq!(
            budget.consume(QueryCharge::SourceWork, 1),
            Err(QueryControlError::SourceWorkExceeded)
        );
        tokio::time::advance(Duration::from_secs(5)).await;

        assert_eq!(
            budget.run_until_deadline(async { "response" }).await,
            Err(QueryControlError::DeadlineExceeded)
        );
        assert_eq!(
            budget.checkpoint(),
            Err(QueryControlError::SourceWorkExceeded)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn pending_handoff_reports_deadline_after_an_earlier_resource_failure() {
        let budget = RequestBudget::after(
            Duration::from_secs(5),
            QueryLimits::new(0, u64::MAX, u64::MAX),
        );
        assert_eq!(
            budget.consume(QueryCharge::SourceWork, 1),
            Err(QueryControlError::SourceWorkExceeded)
        );
        let waiter = tokio::spawn({
            let budget = budget.clone();
            async move {
                budget
                    .run_until_deadline(std::future::pending::<()>())
                    .await
            }
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(5)).await;

        assert_eq!(
            waiter.await.expect("waiter task"),
            Err(QueryControlError::DeadlineExceeded)
        );
        assert_eq!(
            budget.checkpoint(),
            Err(QueryControlError::SourceWorkExceeded)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn unrepresentable_handoff_deadline_remains_accounting_overflow() {
        let budget = RequestBudget::after(
            Duration::MAX,
            QueryLimits::new(u64::MAX, u64::MAX, u64::MAX),
        );

        assert_eq!(
            budget.run_until_deadline(async { "response" }).await,
            Err(QueryControlError::AccountingOverflow)
        );
        assert_eq!(
            budget.checkpoint(),
            Err(QueryControlError::AccountingOverflow)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_at_or_after_deadline_records_deadline() {
        let expired = RequestBudget::after(
            Duration::from_secs(5),
            QueryLimits::new(u64::MAX, u64::MAX, u64::MAX),
        );
        tokio::time::advance(Duration::from_secs(5)).await;
        assert_eq!(expired.cancel(), QueryControlError::DeadlineExceeded);

        let live = RequestBudget::after(
            Duration::from_secs(5),
            QueryLimits::new(u64::MAX, u64::MAX, u64::MAX),
        );
        assert_eq!(live.cancel(), QueryControlError::Cancelled);
    }

    #[test]
    fn ask_preflight_rejects_zero_capacity_without_consuming() {
        let budget = RequestBudget::after(
            Duration::from_secs(5),
            QueryLimits::new(u64::MAX, 0, u64::MAX),
        );

        assert_eq!(
            budget.preflight_ask_result(),
            Err(QueryControlError::ResultItemsExceeded)
        );
        assert_eq!(budget.consumed(QueryCharge::ResultItems), 0);
    }

    #[test]
    fn ask_preflight_leaves_positive_capacity_for_the_executor() {
        let budget = RequestBudget::after(
            Duration::from_secs(5),
            QueryLimits::new(u64::MAX, 1, u64::MAX),
        );

        assert_eq!(budget.preflight_ask_result(), Ok(()));
        assert_eq!(budget.consumed(QueryCharge::ResultItems), 0);
    }
}
