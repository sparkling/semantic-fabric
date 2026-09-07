//! Runtime-neutral query-governance port and atomic accounting spine. Adapters
//! supply time while every phase shares one [`QueryBudget`] identity.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

mod reservation;

pub use reservation::{ReservationError, ReservationShape, ReservationToken};

/// A governed unit charged by an observable execution boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum QueryCharge {
    CompilerWork,
    SourceWork,
    ResultItems,
    SerializedBytes,
    /// Positive growth in a retained-memory high-water mark.
    RetainedBytes,
}

/// Immutable inclusive limits for one request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueryLimits {
    max_compiler_work: u64,
    max_source_work: u64,
    max_result_items: u64,
    max_serialized_bytes: u64,
    reservation: ReservationLimits,
}

/// Inclusive per-query limits for resources that must be acquired before use.
///
/// The default is deliberately unbounded so existing [`QueryLimits::new`]
/// callers retain their original behavior until they opt into this prototype.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReservationLimits {
    max_retained_bytes: u64,
    max_spill_bytes: u64,
    max_spill_files: u64,
    max_file_descriptors: u64,
    max_operator_tasks: u64,
}

impl ReservationLimits {
    pub const UNBOUNDED: Self = Self::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX);

    pub const fn new(
        max_retained_bytes: u64,
        max_spill_bytes: u64,
        max_spill_files: u64,
        max_file_descriptors: u64,
        max_operator_tasks: u64,
    ) -> Self {
        Self {
            max_retained_bytes,
            max_spill_bytes,
            max_spill_files,
            max_file_descriptors,
            max_operator_tasks,
        }
    }

    pub const fn max_retained_bytes(self) -> u64 {
        self.max_retained_bytes
    }

    pub const fn max_spill_bytes(self) -> u64 {
        self.max_spill_bytes
    }

    pub const fn max_spill_files(self) -> u64 {
        self.max_spill_files
    }

    pub const fn max_file_descriptors(self) -> u64 {
        self.max_file_descriptors
    }

    pub const fn max_operator_tasks(self) -> u64 {
        self.max_operator_tasks
    }

    const fn with_max_retained_bytes(mut self, maximum: u64) -> Self {
        self.max_retained_bytes = maximum;
        self
    }
}

impl Default for ReservationLimits {
    fn default() -> Self {
        Self::UNBOUNDED
    }
}

impl QueryLimits {
    /// Construct the original cumulative limits. Retained bytes remain
    /// unbounded until [`Self::with_max_retained_bytes`] is called; serving does so.
    pub const fn new(
        max_compiler_work: u64,
        max_source_work: u64,
        max_result_items: u64,
        max_serialized_bytes: u64,
    ) -> Self {
        Self {
            max_compiler_work,
            max_source_work,
            max_result_items,
            max_serialized_bytes,
            reservation: ReservationLimits::UNBOUNDED,
        }
    }

    pub const fn with_max_retained_bytes(mut self, maximum: u64) -> Self {
        self.reservation = self.reservation.with_max_retained_bytes(maximum);
        self
    }

    /// Replace every reserve-before-allocate limit in one coherent value.
    pub const fn with_reservation_limits(mut self, limits: ReservationLimits) -> Self {
        self.reservation = limits;
        self
    }

    pub const fn max_compiler_work(self) -> u64 {
        self.max_compiler_work
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

    pub const fn max_retained_bytes(self) -> u64 {
        self.reservation.max_retained_bytes()
    }

    pub const fn reservation_limits(self) -> ReservationLimits {
        self.reservation
    }

    const fn limit(self, charge: QueryCharge) -> u64 {
        match charge {
            QueryCharge::CompilerWork => self.max_compiler_work,
            QueryCharge::SourceWork => self.max_source_work,
            QueryCharge::ResultItems => self.max_result_items,
            QueryCharge::SerializedBytes => self.max_serialized_bytes,
            QueryCharge::RetainedBytes => self.reservation.max_retained_bytes(),
        }
    }
}

macro_rules! define_query_control_error {
    ($($(#[$meta:meta])* $variant:ident => $message:literal),+ $(,)?) => {
        /// A typed terminal reason for governed query execution.
        #[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
        #[non_exhaustive]
        pub enum QueryControlError {
            $($(#[$meta])* #[error($message)] $variant),+
        }

        impl QueryControlError {
            /// Compile-time cardinality for downstream explicit-mapping tables.
            pub const VARIANT_COUNT: usize = [$(stringify!($variant)),+].len();
            /// Every currently defined variant, generated from the enum source list.
            pub const VARIANTS: [Self; Self::VARIANT_COUNT] = [$(Self::$variant),+];
        }
    };
}

define_query_control_error! {
    DeadlineExceeded => "query deadline exceeded",
    Cancelled => "query cancelled",
    CompilerEnvelopeExceeded => "query compiler safety envelope exceeded",
    CompilerResourceExhausted => "query compiler resource exhausted",
    CompilerWorkExceeded => "query compiler-work budget exceeded",
    SourceWorkExceeded => "query source-work budget exceeded",
    ResultItemsExceeded => "query result-item budget exceeded",
    SerializedBytesExceeded => "query serialized-byte budget exceeded",
    RetainedBytesExceeded => "query retained-byte budget exceeded",
    AccountingOverflow => "query budget accounting overflow",
}

impl QueryControlError {
    const fn for_limit(charge: QueryCharge) -> Self {
        match charge {
            QueryCharge::CompilerWork => Self::CompilerWorkExceeded,
            QueryCharge::SourceWork => Self::SourceWorkExceeded,
            QueryCharge::ResultItems => Self::ResultItemsExceeded,
            QueryCharge::SerializedBytes => Self::SerializedBytesExceeded,
            QueryCharge::RetainedBytes => Self::RetainedBytesExceeded,
        }
    }
}

/// The executor-facing governance contract.
///
/// `consume` is inclusive: reaching a limit succeeds and the next unit fails.
/// Stateful implementations return their sticky first terminal cause.
pub trait QueryControl: Send + Sync {
    fn checkpoint(&self) -> Result<(), QueryControlError>;
    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError>;
    fn terminate(&self, reason: QueryControlError) -> QueryControlError;
}

/// Explicit control for raw/diagnostic APIs; production supplies a real budget.
#[derive(Clone, Copy, Debug, Default)]
pub struct UncontrolledQueryControl;

impl QueryControl for UncontrolledQueryControl {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        Ok(())
    }

    fn consume(&self, _charge: QueryCharge, _amount: u64) -> Result<(), QueryControlError> {
        Ok(())
    }

    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        reason
    }
}

#[derive(Debug)]
struct BudgetState {
    limits: QueryLimits,
    compiler_work: AtomicU64,
    source_work: AtomicU64,
    result_items: AtomicU64,
    serialized_bytes: AtomicU64,
    retained_bytes: AtomicU64,
    // Operations needing both locks always take `terminal` read first and this
    // mutex second. Token Drop takes only this mutex and never terminal.
    reservations: Mutex<reservation::ReservationLedger>,
    terminal: RwLock<Option<QueryControlError>>,
}

/// Cloneable atomic accounting shared by every phase of one query.
#[derive(Clone, Debug)]
pub struct QueryBudget(Arc<BudgetState>);

impl QueryBudget {
    pub fn new(limits: QueryLimits) -> Self {
        Self(Arc::new(BudgetState {
            limits,
            compiler_work: AtomicU64::new(0),
            source_work: AtomicU64::new(0),
            result_items: AtomicU64::new(0),
            serialized_bytes: AtomicU64::new(0),
            retained_bytes: AtomicU64::new(0),
            reservations: Mutex::new(reservation::ReservationLedger::default()),
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
            QueryCharge::CompilerWork => &self.0.compiler_work,
            QueryCharge::SourceWork => &self.0.source_work,
            QueryCharge::ResultItems => &self.0.result_items,
            QueryCharge::SerializedBytes => &self.0.serialized_bytes,
            QueryCharge::RetainedBytes => &self.0.retained_bytes,
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

    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        QueryBudget::terminate(self, reason)
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

        if charge == QueryCharge::RetainedBytes {
            // Cumulative retained-memory high-water charges and live retained
            // reservations share one limit. The reservation lock is their
            // linearization point, so neither path can independently admit the
            // full capacity.
            let reservations = self
                .0
                .reservations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let current = self.0.retained_bytes.load(Ordering::Acquire);
            let Some(next) = current.checked_add(amount) else {
                drop(reservations);
                drop(terminal);
                return Err(self.terminate(QueryControlError::AccountingOverflow));
            };
            let Some(combined) = next.checked_add(reservations.used().retained_bytes()) else {
                drop(reservations);
                drop(terminal);
                return Err(self.terminate(QueryControlError::AccountingOverflow));
            };
            if combined > self.0.limits.max_retained_bytes() {
                drop(reservations);
                drop(terminal);
                return Err(self.terminate(QueryControlError::RetainedBytesExceeded));
            }
            self.0.retained_bytes.store(next, Ordering::Release);
            return Ok(());
        }

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
#[path = "query_control/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "query_control/envelope_tests.rs"]
mod envelope_tests;
