//! Runtime-neutral query-governance port and atomic accounting spine.
//!
//! [`QueryBudget`] owns no clock, async runtime, database, or HTTP policy. A
//! runtime adapter supplies deadline observation and wake-up semantics while the
//! executor and serializers share this exact accounting identity.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// A governed unit charged by an observable execution boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum QueryCharge {
    /// One metadata probe, branch open, or row-pull attempt.
    SourceWork,
    /// One semantic SELECT row, CONSTRUCT triple, or ASK boolean.
    ResultItems,
    /// Bytes offered to the response serializer's bounded writer.
    SerializedBytes,
}

/// Immutable inclusive limits for one request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueryLimits {
    max_source_work: u64,
    max_result_items: u64,
    max_serialized_bytes: u64,
}

impl QueryLimits {
    pub const fn new(
        max_source_work: u64,
        max_result_items: u64,
        max_serialized_bytes: u64,
    ) -> Self {
        Self {
            max_source_work,
            max_result_items,
            max_serialized_bytes,
        }
    }

    pub const fn max_source_work(self) -> u64 {
        self.max_source_work
    }

    pub const fn max_result_items(self) -> u64 {
        self.max_result_items
    }

    pub const fn max_serialized_bytes(self) -> u64 {
        self.max_serialized_bytes
    }

    const fn limit(self, charge: QueryCharge) -> u64 {
        match charge {
            QueryCharge::SourceWork => self.max_source_work,
            QueryCharge::ResultItems => self.max_result_items,
            QueryCharge::SerializedBytes => self.max_serialized_bytes,
        }
    }
}

/// A typed terminal reason for governed query execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum QueryControlError {
    #[error("query deadline exceeded")]
    DeadlineExceeded,
    #[error("query cancelled")]
    Cancelled,
    #[error("query source-work budget exceeded")]
    SourceWorkExceeded,
    #[error("query result-item budget exceeded")]
    ResultItemsExceeded,
    #[error("query serialized-byte budget exceeded")]
    SerializedBytesExceeded,
    #[error("query budget accounting overflow")]
    AccountingOverflow,
}

impl QueryControlError {
    const fn for_limit(charge: QueryCharge) -> Self {
        match charge {
            QueryCharge::SourceWork => Self::SourceWorkExceeded,
            QueryCharge::ResultItems => Self::ResultItemsExceeded,
            QueryCharge::SerializedBytes => Self::SerializedBytesExceeded,
        }
    }
}

/// The executor-facing governance contract.
///
/// Implementations must keep one control identity for a query. `consume` is an
/// inclusive checked charge: reaching a limit succeeds; the next unit fails.
pub trait QueryControl: Send + Sync {
    fn checkpoint(&self) -> Result<(), QueryControlError>;
    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError>;
}

/// Explicit control for raw, diagnostic, and conformance APIs.
///
/// Production serving must supply a real request budget instead.
#[derive(Clone, Copy, Debug, Default)]
pub struct UncontrolledQueryControl;

impl QueryControl for UncontrolledQueryControl {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        Ok(())
    }

    fn consume(&self, _charge: QueryCharge, _amount: u64) -> Result<(), QueryControlError> {
        Ok(())
    }
}

#[derive(Debug)]
struct BudgetState {
    limits: QueryLimits,
    source_work: AtomicU64,
    result_items: AtomicU64,
    serialized_bytes: AtomicU64,
    terminal: RwLock<Option<QueryControlError>>,
}

/// Cloneable atomic accounting shared by every phase of one query.
#[derive(Clone, Debug)]
pub struct QueryBudget(Arc<BudgetState>);

impl QueryBudget {
    pub fn new(limits: QueryLimits) -> Self {
        Self(Arc::new(BudgetState {
            limits,
            source_work: AtomicU64::new(0),
            result_items: AtomicU64::new(0),
            serialized_bytes: AtomicU64::new(0),
            terminal: RwLock::new(None),
        }))
    }

    pub fn limits(&self) -> QueryLimits {
        self.0.limits
    }

    pub fn consumed(&self, charge: QueryCharge) -> u64 {
        self.counter(charge).load(Ordering::Acquire)
    }

    /// Seal a terminal reason if none exists and return the sticky first reason.
    pub fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        let mut terminal = self
            .0
            .terminal
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *terminal.get_or_insert(reason)
    }

    pub fn terminal(&self) -> Option<QueryControlError> {
        *self
            .0
            .terminal
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn counter(&self, charge: QueryCharge) -> &AtomicU64 {
        match charge {
            QueryCharge::SourceWork => &self.0.source_work,
            QueryCharge::ResultItems => &self.0.result_items,
            QueryCharge::SerializedBytes => &self.0.serialized_bytes,
        }
    }
}

impl QueryControl for QueryBudget {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        self.terminal().map_or(Ok(()), Err)
    }

    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        self.consume_with_hook(charge, amount, || {})
    }
}

impl QueryBudget {
    fn consume_with_hook(
        &self,
        charge: QueryCharge,
        amount: u64,
        before_commit: impl FnOnce(),
    ) -> Result<(), QueryControlError> {
        // A termination write excludes this read-side critical section. A charge
        // therefore linearizes wholly before a later terminal transition, or sees
        // the existing terminal and leaves every counter unchanged.
        let terminal = self
            .0
            .terminal
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(reason) = *terminal {
            return Err(reason);
        }
        if amount == 0 {
            return Ok(());
        }
        before_commit();

        let counter = self.counter(charge);
        let limit = self.0.limits.limit(charge);
        let mut current = counter.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_add(amount) else {
                drop(terminal);
                return Err(self.terminate(QueryControlError::AccountingOverflow));
            };
            if next > limit {
                drop(terminal);
                return Err(self.terminate(QueryControlError::for_limit(charge)));
            }
            match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Ok(()),
                Err(observed) => current = observed,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inclusive_limit_succeeds_then_first_failure_is_sticky() {
        let budget = QueryBudget::new(QueryLimits::new(3, 2, 5));
        budget.consume(QueryCharge::SourceWork, 3).unwrap();
        assert_eq!(budget.consumed(QueryCharge::SourceWork), 3);
        assert_eq!(
            budget.consume(QueryCharge::SourceWork, 1),
            Err(QueryControlError::SourceWorkExceeded)
        );
        assert_eq!(
            budget.consume(QueryCharge::ResultItems, 1),
            Err(QueryControlError::SourceWorkExceeded)
        );
        assert_eq!(budget.consumed(QueryCharge::ResultItems), 0);
    }

    #[test]
    fn every_dimension_has_an_exact_typed_boundary() {
        for (charge, expected) in [
            (
                QueryCharge::SourceWork,
                QueryControlError::SourceWorkExceeded,
            ),
            (
                QueryCharge::ResultItems,
                QueryControlError::ResultItemsExceeded,
            ),
            (
                QueryCharge::SerializedBytes,
                QueryControlError::SerializedBytesExceeded,
            ),
        ] {
            let budget = QueryBudget::new(QueryLimits::new(1, 1, 1));
            budget.consume(charge, 1).unwrap();
            assert_eq!(budget.consume(charge, 1), Err(expected));
        }
    }

    #[test]
    fn arithmetic_overflow_fails_closed() {
        let budget = QueryBudget::new(QueryLimits::new(u64::MAX, 1, 1));
        budget.consume(QueryCharge::SourceWork, u64::MAX).unwrap();
        assert_eq!(
            budget.consume(QueryCharge::SourceWork, 1),
            Err(QueryControlError::AccountingOverflow)
        );
        assert_eq!(budget.consumed(QueryCharge::SourceWork), u64::MAX);
    }

    #[test]
    fn concurrent_consumers_cannot_overshoot() {
        let budget = QueryBudget::new(QueryLimits::new(10_000, 1, 1));
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let budget = budget.clone();
                scope.spawn(
                    move || {
                        while budget.consume(QueryCharge::SourceWork, 1).is_ok() {}
                    },
                );
            }
        });
        assert_eq!(budget.consumed(QueryCharge::SourceWork), 10_000);
        assert_eq!(
            budget.checkpoint(),
            Err(QueryControlError::SourceWorkExceeded)
        );
    }

    #[test]
    fn cancellation_wins_and_remains_the_terminal_reason() {
        let budget = QueryBudget::new(QueryLimits::new(10, 10, 10));
        assert_eq!(
            budget.terminate(QueryControlError::Cancelled),
            QueryControlError::Cancelled
        );
        assert_eq!(
            budget.terminate(QueryControlError::DeadlineExceeded),
            QueryControlError::Cancelled
        );
        assert_eq!(budget.checkpoint(), Err(QueryControlError::Cancelled));
    }

    #[test]
    fn an_in_flight_charge_linearizes_before_termination() {
        let budget = QueryBudget::new(QueryLimits::new(1, 1, 1));
        let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
        let release = std::sync::Arc::new(std::sync::Barrier::new(2));

        std::thread::scope(|scope| {
            let charge_budget = budget.clone();
            let charge_entered = entered.clone();
            let charge_release = release.clone();
            let charge = scope.spawn(move || {
                charge_budget.consume_with_hook(QueryCharge::SourceWork, 1, || {
                    charge_entered.wait();
                    charge_release.wait();
                })
            });
            entered.wait();
            let terminate_budget = budget.clone();
            let terminate =
                scope.spawn(move || terminate_budget.terminate(QueryControlError::Cancelled));
            release.wait();

            assert_eq!(charge.join().unwrap(), Ok(()));
            assert_eq!(terminate.join().unwrap(), QueryControlError::Cancelled);
        });
        assert_eq!(budget.consumed(QueryCharge::SourceWork), 1);
        assert_eq!(budget.checkpoint(), Err(QueryControlError::Cancelled));
    }

    #[test]
    fn a_charge_rejected_after_termination_changes_no_counter() {
        let budget = QueryBudget::new(QueryLimits::new(1, 1, 1));
        budget.terminate(QueryControlError::Cancelled);

        assert_eq!(
            budget.consume(QueryCharge::SourceWork, 1),
            Err(QueryControlError::Cancelled)
        );
        assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
    }
}
