//! One-shot pidfd/descriptor readiness polling, split out of `io.rs` to keep
//! that module within the repository's 500-line source limit.
//!
//! `poll_step` performs exactly one `libc::poll` and classifies it without
//! retrying, which is what lets the synchronous `wait_ready` loop and the
//! async bounded-step callers share identical readiness semantics while
//! differing only in who decides to poll again.

use std::os::fd::RawFd;
use std::time::Instant;

use super::SupervisorError;

/// Outcome of exactly one bounded (`<=` the caller's `timeout_ms`) poll
/// attempt: pure, no thread-local dependency, no internal retry loop. Shared
/// by the synchronous `wait_ready` (which loops it exactly as it always
/// polled) and the async bounded-step callers (which checkpoint their own
/// real control and cooperatively yield between `Pending` steps instead of
/// blocking their calling thread for the full remaining deadline).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PollStep {
    Ready,
    Pending,
}

/// Exactly one `libc::poll` call, classified but not retried. `timeout_ms`
/// is the caller's own choice (the legacy thread-local-capped value for
/// `wait_ready`, or an explicit short cap for an async bounded step).
pub(super) fn poll_step(
    descriptor: RawFd,
    pidfd: RawFd,
    requested: libc::c_short,
    timeout_ms: i32,
    allow_worker_exit: bool,
) -> Result<PollStep, SupervisorError> {
    let mut descriptors = [
        libc::pollfd {
            fd: descriptor,
            events: requested,
            revents: 0,
        },
        libc::pollfd {
            fd: pidfd,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    // SAFETY: descriptors is a valid writable two-element pollfd array.
    let result =
        unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, timeout_ms) };
    if result == 0 {
        return Ok(PollStep::Pending);
    }
    if result < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            return Ok(PollStep::Pending);
        }
        return Err(SupervisorError::operation("poll parser worker I/O")(error));
    }
    if descriptors
        .iter()
        .any(|entry| entry.revents & libc::POLLNVAL != 0)
    {
        return Err(SupervisorError::InvalidState(
            "parser worker I/O descriptor became invalid",
        ));
    }
    let io_ready = descriptors[0].revents & (requested | libc::POLLHUP | libc::POLLERR) != 0;
    let worker_exited =
        descriptors[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0;
    // Drain bytes written before exit, but never write after pidfd reports
    // exit. A concurrently forking process can transiently inherit the
    // CLOEXEC pipe reader and otherwise make a dead worker look writable.
    if requested == libc::POLLIN && io_ready {
        return Ok(PollStep::Ready);
    }
    if worker_exited && !allow_worker_exit {
        return Err(SupervisorError::InvalidState(
            "parser worker exited before fixed I/O completed",
        ));
    }
    if worker_exited || io_ready {
        return Ok(PollStep::Ready);
    }
    Ok(PollStep::Pending)
}

fn wait_ready(
    descriptor: RawFd,
    pidfd: RawFd,
    requested: libc::c_short,
    deadline: Instant,
    allow_worker_exit: bool,
) -> Result<(), SupervisorError> {
    loop {
        crate::parser_isolation::runtime::checkpoint()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(SupervisorError::DeadlineExceeded);
        }
        let millis = remaining.as_millis().saturating_add(u128::from(
            !remaining.subsec_nanos().is_multiple_of(1_000_000),
        ));
        let timeout = crate::parser_isolation::runtime::poll_timeout(
            i32::try_from(millis.min(i32::MAX as u128)).unwrap_or(i32::MAX),
        );
        match poll_step(descriptor, pidfd, requested, timeout, allow_worker_exit)? {
            PollStep::Pending => continue,
            PollStep::Ready => {
                // poll uses a millisecond ceiling. Never accept readiness
                // observed only after the immutable deadline because that
                // rounded timeout elapsed.
                crate::parser_isolation::runtime::checkpoint()?;
                if Instant::now() >= deadline {
                    return Err(SupervisorError::DeadlineExceeded);
                }
                return Ok(());
            }
        }
    }
}
