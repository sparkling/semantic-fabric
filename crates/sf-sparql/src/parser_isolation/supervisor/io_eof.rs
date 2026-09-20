//! Exact-EOF and silent-peer observation for the parser worker's output
//! pipe, split out of `io.rs` to keep that module within the repository's
//! 500-line source limit.
//!
//! These share `BoundedWorkerIo`'s descriptors and the same immutable
//! deadline discipline as the transfer steps: every poll is clamped to the
//! remaining lifetime and the deadline is rechecked before any syscall.

use std::os::fd::{AsRawFd, OwnedFd};
use std::time::Instant;

use super::io::{legacy_poll_timeout_ms, BoundedWorkerIo, StepOutcome};
use super::io_bounds::{ensure_before_deadline, ensure_deadline_not_passed, step_poll_timeout_ms};
use super::io_poll::{poll_step, PollStep};
use super::SupervisorError;

impl BoundedWorkerIo {
    /// Observe a live peer producing no output until a short local deadline.
    ///
    /// This neither consumes nor charges bytes. The immutable lifetime
    /// deadline still governs every later I/O and containment operation.
    pub(super) fn observe_alive_and_silent_until(
        &self,
        pidfd: &OwnedFd,
        observation_deadline: Instant,
        lifetime_deadline: Instant,
    ) -> Result<(), SupervisorError> {
        if observation_deadline >= lifetime_deadline {
            return Err(SupervisorError::InvalidState(
                "request EOF observation does not fit the worker lifetime",
            ));
        }
        ensure_before_deadline(observation_deadline)?;
        let output = self
            .stdout
            .as_ref()
            .ok_or(SupervisorError::InvalidState(
                "parser worker stdout is closed",
            ))?
            .as_raw_fd();

        loop {
            let remaining = observation_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            let millis = remaining.as_millis().saturating_add(u128::from(
                !remaining.subsec_nanos().is_multiple_of(1_000_000),
            ));
            let timeout = i32::try_from(millis.min(i32::MAX as u128)).unwrap_or(i32::MAX);
            let mut descriptors = [
                libc::pollfd {
                    fd: output,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: pidfd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let result = unsafe { libc::poll(descriptors.as_mut_ptr(), 2, timeout) };
            let now = Instant::now();
            if now >= lifetime_deadline {
                return Err(SupervisorError::DeadlineExceeded);
            }
            if result == 0 {
                if now >= observation_deadline {
                    return Ok(());
                }
                continue;
            }
            if result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(SupervisorError::operation(
                    "observe parser worker before request EOF",
                )(error));
            }
            if descriptors
                .iter()
                .any(|descriptor| descriptor.revents & libc::POLLNVAL != 0)
            {
                return Err(SupervisorError::InvalidState(
                    "request EOF observation descriptor became invalid",
                ));
            }
            if descriptors[0].revents & libc::POLLIN != 0 {
                return Err(SupervisorError::InvalidState(
                    "parser worker emitted result bytes before request EOF",
                ));
            }
            if descriptors[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                return Err(SupervisorError::InvalidState(
                    "parser worker exited before request EOF",
                ));
            }
            if descriptors[0].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                return Err(SupervisorError::InvalidState(
                    "parser worker output closed before request EOF",
                ));
            }
            if descriptors.iter().any(|descriptor| descriptor.revents != 0) {
                return Err(SupervisorError::InvalidState(
                    "request EOF observation returned unexpected readiness",
                ));
            }
        }
    }

    /// Require a clean worker-output EOF without accepting a valid frame as a
    /// prefix of a longer message stream.
    pub(super) fn expect_eof(
        &mut self,
        pidfd: &OwnedFd,
        deadline: Instant,
    ) -> Result<(), SupervisorError> {
        ensure_before_deadline(deadline)?;
        loop {
            crate::parser_isolation::runtime::checkpoint()?;
            let timeout_ms = legacy_poll_timeout_ms(deadline)?;
            if self.expect_eof_step(pidfd, deadline, Some(timeout_ms))? == StepOutcome::Complete {
                return Ok(());
            }
        }
    }

    /// One bounded EOF-probe attempt; the read-side twin of [`Self::write_step`]
    /// with `expect_eof`'s exact-EOF, worker-exit-tolerant semantics.
    pub(super) fn expect_eof_step(
        &mut self,
        pidfd: &OwnedFd,
        deadline: Instant,
        poll_cap_ms: Option<i32>,
    ) -> Result<StepOutcome, SupervisorError> {
        let timeout_ms = step_poll_timeout_ms(deadline, poll_cap_ms)?;
        let descriptor = self
            .stdout
            .as_ref()
            .ok_or(SupervisorError::InvalidState(
                "parser worker stdout is closed",
            ))?
            .as_raw_fd();
        match poll_step(
            descriptor,
            pidfd.as_raw_fd(),
            libc::POLLIN,
            timeout_ms,
            true,
        )? {
            PollStep::Pending => {
                ensure_deadline_not_passed(deadline)?;
                return Ok(StepOutcome::Pending);
            }
            PollStep::Ready => {}
        }
        ensure_deadline_not_passed(deadline)?;
        let mut trailing = [0_u8; 1];
        let count = unsafe { libc::read(descriptor, trailing.as_mut_ptr().cast(), 1) };
        if count == 0 {
            return Ok(StepOutcome::Complete);
        }
        if count > 0 {
            // The unconditional one-byte probe distinguishes exact EOF from a
            // longer stream. A trailing byte is rejected, never accepted
            // output, so it does not enlarge or mutate the cumulative cap.
            return Err(SupervisorError::InvalidState(
                "parser worker emitted trailing protocol output",
            ));
        }
        let error = std::io::Error::last_os_error();
        if matches!(
            error.kind(),
            std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
        ) {
            return Ok(StepOutcome::Pending);
        }
        Err(SupervisorError::operation(
            "verify parser worker output EOF",
        )(error))
    }
}
