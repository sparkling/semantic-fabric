//! Runtime-neutral query-governance checkpoint port.
//!
//! The contract owns no clock, runtime, counters, or cancellation mechanism.
//! Its implementor chooses a monotonic instant type and the caller supplies the
//! observed instant. `sf-serve` currently implements only its existing absolute
//! deadline. The non-exhaustive contract may be extended only when later work
//! actually enforces additional control reasons.

/// A typed reason that query execution must stop at a checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum QueryControlError {
    #[error("query deadline exceeded")]
    DeadlineExceeded,
}

/// Pure query-control checkpoint contract at the lowest shared layer.
///
/// `Instant` is associated so the core contract does not depend on Tokio or
/// another runtime. Implementations must use one monotonic clock domain for the
/// lifetime of a query and must not refresh limits between checkpoints.
pub trait QueryControl: Send + Sync {
    type Instant: Copy + Ord;

    fn check_at(&self, now: Self::Instant) -> Result<(), QueryControlError>;
}
