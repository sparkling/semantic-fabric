//! All-or-nothing resource admission owned by a [`QueryBudget`].
//!
//! Capacity and arithmetic rejection are deliberately retryable and leave the
//! ledger unchanged. They do not become the query's sticky terminal reason;
//! only an already-terminal budget rejects with [`ReservationError::QueryTerminated`].

use std::sync::Arc;

use super::{BudgetState, QueryBudget, QueryControlError, ReservationLimits};

/// A five-dimensional amount acquired before memory, files, descriptors, or
/// operator tasks are created.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReservationShape {
    retained_bytes: u64,
    spill_bytes: u64,
    spill_files: u64,
    file_descriptors: u64,
    operator_tasks: u64,
}

impl ReservationShape {
    pub const ZERO: Self = Self::new(0, 0, 0, 0, 0);

    pub const fn new(
        retained_bytes: u64,
        spill_bytes: u64,
        spill_files: u64,
        file_descriptors: u64,
        operator_tasks: u64,
    ) -> Self {
        Self {
            retained_bytes,
            spill_bytes,
            spill_files,
            file_descriptors,
            operator_tasks,
        }
    }

    pub const fn for_retained_bytes(bytes: u64) -> Self {
        Self::new(bytes, 0, 0, 0, 0)
    }

    pub const fn retained_bytes(self) -> u64 {
        self.retained_bytes
    }

    pub const fn spill_bytes(self) -> u64 {
        self.spill_bytes
    }

    pub const fn spill_files(self) -> u64 {
        self.spill_files
    }

    pub const fn file_descriptors(self) -> u64 {
        self.file_descriptors
    }

    pub const fn operator_tasks(self) -> u64 {
        self.operator_tasks
    }

    fn checked_add(self, other: Self) -> Option<Self> {
        Some(Self::new(
            self.retained_bytes.checked_add(other.retained_bytes)?,
            self.spill_bytes.checked_add(other.spill_bytes)?,
            self.spill_files.checked_add(other.spill_files)?,
            self.file_descriptors.checked_add(other.file_descriptors)?,
            self.operator_tasks.checked_add(other.operator_tasks)?,
        ))
    }

    fn checked_sub(self, other: Self) -> Option<Self> {
        Some(Self::new(
            self.retained_bytes.checked_sub(other.retained_bytes)?,
            self.spill_bytes.checked_sub(other.spill_bytes)?,
            self.spill_files.checked_sub(other.spill_files)?,
            self.file_descriptors.checked_sub(other.file_descriptors)?,
            self.operator_tasks.checked_sub(other.operator_tasks)?,
        ))
    }
}

/// Closed, redacted failures from pre-allocation resource admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ReservationError {
    #[error("query is already terminal: {0}")]
    QueryTerminated(QueryControlError),
    #[error("retained-byte reservation limit exceeded")]
    RetainedBytesExceeded,
    #[error("spill-byte reservation limit exceeded")]
    SpillBytesExceeded,
    #[error("spill-file reservation limit exceeded")]
    SpillFilesExceeded,
    #[error("file-descriptor reservation limit exceeded")]
    FileDescriptorsExceeded,
    #[error("operator-task reservation limit exceeded")]
    OperatorTasksExceeded,
    #[error("resource reservation accounting overflow")]
    AccountingOverflow,
}

#[derive(Debug, Default)]
pub(super) struct ReservationLedger {
    used: ReservationShape,
}

impl ReservationLedger {
    pub(super) const fn used(&self) -> ReservationShape {
        self.used
    }

    fn admit(
        &mut self,
        requested: ReservationShape,
        cumulative_retained_bytes: u64,
        limits: ReservationLimits,
    ) -> Result<(), ReservationError> {
        let candidate = self
            .used
            .checked_add(requested)
            .ok_or(ReservationError::AccountingOverflow)?;
        let retained_total = cumulative_retained_bytes
            .checked_add(candidate.retained_bytes)
            .ok_or(ReservationError::AccountingOverflow)?;

        if retained_total > limits.max_retained_bytes() {
            return Err(ReservationError::RetainedBytesExceeded);
        }
        if candidate.spill_bytes > limits.max_spill_bytes() {
            return Err(ReservationError::SpillBytesExceeded);
        }
        if candidate.spill_files > limits.max_spill_files() {
            return Err(ReservationError::SpillFilesExceeded);
        }
        if candidate.file_descriptors > limits.max_file_descriptors() {
            return Err(ReservationError::FileDescriptorsExceeded);
        }
        if candidate.operator_tasks > limits.max_operator_tasks() {
            return Err(ReservationError::OperatorTasksExceeded);
        }

        self.used = candidate;
        Ok(())
    }

    fn release(&mut self, shape: ReservationShape) {
        let Some(remaining) = self.used.checked_sub(shape) else {
            // Token construction and fields are private. If that invariant is
            // ever broken, retain the current usage rather than mint capacity.
            debug_assert!(false, "reservation token released unowned capacity");
            return;
        };
        self.used = remaining;
    }
}

/// Opaque ownership of capacity admitted by [`QueryBudget::reserve`].
///
/// This type is intentionally not `Clone`; its private shape cannot be enlarged.
/// Dropping it releases the exact admitted shape once.
#[derive(Debug)]
pub struct ReservationToken {
    state: Arc<BudgetState>,
    shape: Option<ReservationShape>,
}

impl ReservationToken {
    pub const fn shape(&self) -> ReservationShape {
        match self.shape {
            Some(shape) => shape,
            None => ReservationShape::ZERO,
        }
    }

    /// Release now. Consuming `self` makes a second release unrepresentable.
    pub fn release(self) {
        drop(self);
    }
}

impl Drop for ReservationToken {
    fn drop(&mut self) {
        let Some(shape) = self.shape.take() else {
            return;
        };
        let mut ledger = self
            .state
            .reservations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ledger.release(shape);
    }
}

impl QueryBudget {
    /// Atomically admit every requested dimension or change none of them.
    ///
    /// Capacity/overflow rejection is retryable before terminalization. Once
    /// the query is terminal, no new token can be acquired. The same budget
    /// identity also synchronizes cumulative retained-byte charges.
    pub fn reserve(
        &self,
        requested: ReservationShape,
    ) -> Result<ReservationToken, ReservationError> {
        self.reserve_with_hook(requested, || {})
    }

    fn reserve_with_hook(
        &self,
        requested: ReservationShape,
        before_commit: impl FnOnce(),
    ) -> Result<ReservationToken, ReservationError> {
        let terminal = self
            .0
            .terminal
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(reason) = *terminal {
            return Err(ReservationError::QueryTerminated(reason));
        }

        let mut ledger = self
            .0
            .reservations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        before_commit();
        ledger.admit(
            requested,
            self.0
                .retained_bytes
                .load(std::sync::atomic::Ordering::Acquire),
            self.0.limits.reservation_limits(),
        )?;
        let token = ReservationToken {
            state: Arc::clone(&self.0),
            shape: Some(requested),
        };

        // Explicit drops defeat early NLL release: terminalization cannot
        // linearize between the terminal check and an admitted token escaping.
        drop(ledger);
        drop(terminal);
        Ok(token)
    }

    /// Current live reservations. Cumulative [`super::QueryCharge::RetainedBytes`]
    /// charges remain observable through [`QueryBudget::consumed`].
    pub fn reserved(&self) -> ReservationShape {
        self.0
            .reservations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .used()
    }
}

#[cfg(test)]
#[path = "reservation_tests.rs"]
mod tests;
