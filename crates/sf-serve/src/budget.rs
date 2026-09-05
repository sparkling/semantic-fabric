//! One request-scoped deadline, cancellation, accounting, and active-work
//! admission identity.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use tokio::sync::{watch, OwnedSemaphorePermit};
use tokio::time::Instant;

use crate::lifecycle::ShutdownPhase;

struct RequestBudgetState {
    accounting: QueryBudget,
    deadline: Option<Instant>,
    deadline_representable: bool,
    terminal: watch::Sender<Option<QueryControlError>>,
    /// Draining preserves this in-flight identity; only the forced phase cancels it.
    shutdown: Option<watch::Receiver<ShutdownPhase>>,
    /// One fail-fast serve-lane admission identity. The owned permit follows
    /// every budget clone into active producers and blocking workers, then
    /// returns only when the last such clone is dropped.
    admission: Option<OwnedSemaphorePermit>,
}

/// The single governance identity minted before request-body extraction. On the
/// serve lane it also retains one aggregate admission permit through every
/// active producer or worker clone, but not through already-produced body bytes.
#[derive(Clone)]
pub(crate) struct RequestBudget(Arc<RequestBudgetState>);

pub(crate) struct CancellationGuard(Option<RequestBudget>);

impl RequestBudget {
    #[cfg(test)]
    pub(crate) fn after(timeout: Duration, limits: QueryLimits) -> Self {
        Self::build(timeout, limits, None)
    }

    /// Mint a serving budget that observes only forced shutdown, not graceful drain.
    pub(crate) fn after_with_shutdown(
        timeout: Duration,
        limits: QueryLimits,
        shutdown: watch::Receiver<ShutdownPhase>,
    ) -> Self {
        Self::build(timeout, limits, Some(shutdown))
    }

    fn build(
        timeout: Duration,
        limits: QueryLimits,
        shutdown: Option<watch::Receiver<ShutdownPhase>>,
    ) -> Self {
        let now = Instant::now();
        let (terminal, _) = watch::channel(None);
        let deadline = now.checked_add(timeout);
        let request = Self(Arc::new(RequestBudgetState {
            accounting: QueryBudget::new(limits),
            deadline: Some(deadline.unwrap_or(now)),
            deadline_representable: deadline.is_some(),
            terminal,
            shutdown,
            admission: None,
        }));
        if deadline.is_none() {
            request.terminate(QueryControlError::AccountingOverflow);
        }
        request
    }

    pub(crate) fn uncontrolled(deadline: Option<std::time::Instant>) -> Self {
        let (terminal, _) = watch::channel(None);
        Self(Arc::new(RequestBudgetState {
            accounting: QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX)),
            deadline: deadline.map(Instant::from_std),
            deadline_representable: true,
            terminal,
            shutdown: None,
            admission: None,
        }))
    }

    /// Attach the outer serving admission permit before this budget is cloned.
    /// Returning the permit on misuse keeps this boundary fail closed without a
    /// panic or a silent capacity leak.
    pub(crate) fn retain_admission(
        &mut self,
        permit: OwnedSemaphorePermit,
    ) -> Result<(), OwnedSemaphorePermit> {
        let Some(state) = Arc::get_mut(&mut self.0) else {
            return Err(permit);
        };
        if state.admission.is_some() {
            return Err(permit);
        }
        state.admission = Some(permit);
        Ok(())
    }

    /// Await a phase without refreshing the original absolute deadline.
    pub(crate) async fn run<F>(&self, future: F) -> Result<F::Output, QueryControlError>
    where
        F: Future,
    {
        self.checkpoint()?;
        let mut terminal = self.0.terminal.subscribe();
        let shutdown = self.0.shutdown.clone();
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
            _ = wait_for_forced_shutdown(shutdown) => {
                Err(self.cancel())
            }
            output = future => {
                self.checkpoint()?;
                Ok(output)
            }
        }
    }

    /// Remaining wall-clock allowance on this request's original absolute
    /// deadline. PostgreSQL generation leases convert this to transaction-local
    /// statement, lock, and idle timeouts; they never refresh it per phase.
    pub(crate) fn remaining_duration(&self) -> Result<Option<Duration>, QueryControlError> {
        self.checkpoint()?;
        Ok(self
            .0
            .deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now())))
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
        if self.shutdown_forced() {
            return Err(self.cancel());
        }
        let shutdown = self.0.shutdown.clone();
        tokio::select! {
            biased;
            _ = wait_for_deadline(self.0.deadline) => {
                Err(self.handoff_deadline_error())
            }
            _ = wait_for_forced_shutdown(shutdown) => {
                Err(self.cancel())
            }
            output = future => {
                if self.shutdown_forced() {
                    return Err(self.cancel());
                }
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
        if self.shutdown_forced() {
            return Err(self.cancel());
        }
        if self.0.deadline.is_some_and(|deadline| now >= deadline) {
            return Err(self.terminate(QueryControlError::DeadlineExceeded));
        }
        Ok(())
    }

    fn deadline_reached(&self, now: Instant) -> bool {
        self.0.deadline.is_some_and(|deadline| now >= deadline)
    }

    fn shutdown_forced(&self) -> bool {
        self.0
            .shutdown
            .as_ref()
            .is_some_and(|shutdown| *shutdown.borrow() == ShutdownPhase::Forced)
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

async fn wait_for_forced_shutdown(mut shutdown: Option<watch::Receiver<ShutdownPhase>>) {
    let Some(shutdown) = shutdown.as_mut() else {
        std::future::pending::<()>().await;
        return;
    };
    loop {
        if *shutdown.borrow_and_update() == ShutdownPhase::Forced {
            return;
        }
        if shutdown.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
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

    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        RequestBudget::terminate(self, reason)
    }
}
