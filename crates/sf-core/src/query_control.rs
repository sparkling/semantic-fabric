//! Runtime-neutral query-governance port and atomic accounting spine.
//!
//! [`QueryBudget`] owns no clock, async runtime, database, or HTTP policy. A
//! runtime adapter supplies deadline observation and wake-up semantics while the
//! executor and serializers share this exact accounting identity.

use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

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
    const fn code(self) -> u8 {
        match self {
            Self::DeadlineExceeded => 1,
            Self::Cancelled => 2,
            Self::SourceWorkExceeded => 3,
            Self::ResultItemsExceeded => 4,
            Self::SerializedBytesExceeded => 5,
            Self::AccountingOverflow => 6,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => None,
            1 => Some(Self::DeadlineExceeded),
            2 => Some(Self::Cancelled),
            3 => Some(Self::SourceWorkExceeded),
            4 => Some(Self::ResultItemsExceeded),
            5 => Some(Self::SerializedBytesExceeded),
            6 => Some(Self::AccountingOverflow),
            _ => Some(Self::AccountingOverflow),
        }
    }

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
    terminal: AtomicU8,
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
            terminal: AtomicU8::new(0),
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
        let _ =
            self.0
                .terminal
                .compare_exchange(0, reason.code(), Ordering::AcqRel, Ordering::Acquire);
        self.terminal()
            .unwrap_or(QueryControlError::AccountingOverflow)
    }

    pub fn terminal(&self) -> Option<QueryControlError> {
        QueryControlError::from_code(self.0.terminal.load(Ordering::Acquire))
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
        self.checkpoint()?;
        if amount == 0 {
            return Ok(());
        }

        let counter = self.counter(charge);
        let limit = self.0.limits.limit(charge);
        let mut current = counter.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_add(amount) else {
                return Err(self.terminate(QueryControlError::AccountingOverflow));
            };
            if next > limit {
                return Err(self.terminate(QueryControlError::for_limit(charge)));
            }
            match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return self.checkpoint(),
                Err(observed) => {
                    self.checkpoint()?;
                    current = observed;
                }
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
}
