//! Pure deadline and cumulative-byte bound arithmetic for the parser
//! worker's bounded-step I/O.
//!
//! Split out of `io.rs` to keep that module within the repository's
//! 500-line source limit. Nothing here touches a descriptor: these are the
//! admission and clamping decisions the step methods consult before any
//! syscall, which is what makes them independently testable.

use std::time::Instant;

use super::SupervisorError;

pub(super) fn ensure_before_deadline(deadline: Instant) -> Result<(), SupervisorError> {
    crate::parser_isolation::runtime::checkpoint()?;
    if Instant::now() >= deadline {
        Err(SupervisorError::DeadlineExceeded)
    } else {
        Ok(())
    }
}

/// The poll timeout for one bounded step: always clamped to the remaining
/// immutable lifetime, and further clamped by the caller's optional cap.
/// Returns `DeadlineExceeded` when the lifetime has already elapsed, so a
/// step can never begin after expiry.
pub(super) fn step_poll_timeout_ms(
    deadline: Instant,
    cap_ms: Option<i32>,
) -> Result<i32, SupervisorError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(SupervisorError::DeadlineExceeded);
    }
    let millis = remaining.as_millis().saturating_add(u128::from(
        !remaining.subsec_nanos().is_multiple_of(1_000_000),
    ));
    let remaining_ms = i32::try_from(millis.min(i32::MAX as u128)).unwrap_or(i32::MAX);
    Ok(match cap_ms {
        Some(cap) => remaining_ms.min(cap),
        None => remaining_ms,
    })
}

pub(super) fn ensure_deadline_not_passed(deadline: Instant) -> Result<(), SupervisorError> {
    if Instant::now() >= deadline {
        Err(SupervisorError::DeadlineExceeded)
    } else {
        Ok(())
    }
}

/// Grow a cumulative counter by one observed transfer, refusing to exceed the
/// immutable whole-worker ceiling. The reservation admits the transfer up
/// front; this is the per-syscall backstop that keeps the counters bounded
/// even if a peer somehow delivered more than its admitted share.
pub(super) fn admitted_total(
    current: u64,
    count: usize,
    maximum: u64,
    message: &'static str,
) -> Result<u64, SupervisorError> {
    prospective_total(current, count, maximum).ok_or(SupervisorError::InvalidState(message))
}

pub(super) fn prospective_total(current: u64, additional: usize, maximum: u64) -> Option<u64> {
    let additional = u64::try_from(additional).ok()?;
    let total = current.checked_add(additional)?;
    (total <= maximum).then_some(total)
}
