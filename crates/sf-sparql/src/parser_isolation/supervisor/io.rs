//! Cumulative byte bounds and one-deadline pipe I/O for a parser worker.

use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::process::{ChildStdin, ChildStdout};
use std::time::Instant;

use super::io_bounds::{
    admitted_total, ensure_before_deadline, ensure_deadline_not_passed, prospective_total,
    step_poll_timeout_ms,
};
use super::io_poll::{poll_step, PollStep};
use super::SupervisorError;

pub(super) const INPUT_LIMIT_MESSAGE: &str =
    "parser worker input exceeds its cumulative byte limit";
pub(super) const OUTPUT_LIMIT_MESSAGE: &str =
    "parser worker output exceeds its cumulative byte limit";

/// Outcome of one bounded `write_step`/`read_step` call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StepOutcome {
    Complete,
    Pending,
}

/// The exact timeout `wait_ready` always capped its poll to: the remaining
/// time until `deadline`, further capped by the legacy thread-local
/// `runtime::poll_timeout` (<=10ms while a compile-time `REQUEST` scope is
/// active). Only used by the synchronous `write_all`/`read_exact` loops, so
/// existing SPARQL-peer behavior is unchanged byte-for-byte; the async
/// bounded-step path computes its own short cap explicitly instead.
pub(super) fn legacy_poll_timeout_ms(deadline: Instant) -> Result<i32, SupervisorError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(SupervisorError::DeadlineExceeded);
    }
    let millis = remaining.as_millis().saturating_add(u128::from(
        !remaining.subsec_nanos().is_multiple_of(1_000_000),
    ));
    Ok(crate::parser_isolation::runtime::poll_timeout(
        i32::try_from(millis.min(i32::MAX as u128)).unwrap_or(i32::MAX),
    ))
}

pub(super) struct BoundedWorkerIo {
    stdin: Option<ChildStdin>,
    // `pub(super)` so the sibling `io_eof` module can poll the same pipe end
    // under the same deadline discipline; it is never reassigned there.
    pub(super) stdout: Option<ChildStdout>,
    // `pub(super)` only so the sibling `io_tests` module can assert on the
    // counters it drives through `descriptorless_for_test`.
    pub(super) sent: u64,
    pub(super) received: u64,
    max_sent: u64,
    max_received: u64,
}

/// One admitted, bounded transfer driven a step at a time.
///
/// The synchronous `write_all`/`read_exact` loops preflight the WHOLE buffer
/// against the immutable cumulative cap before moving a byte
/// ([`prospective_total`]), so a transfer that cannot fit is rejected before
/// any partial I/O. A caller that drives [`BoundedWorkerIo::write_step`] /
/// [`BoundedWorkerIo::read_step`] directly (the async SQL-canonicalize path)
/// must obtain this reservation first, which performs exactly the same
/// preflight and additionally carries the immutable wall deadline and the
/// expected post-transfer total. Without it the step methods are unreachable,
/// so the direct path cannot bypass either the byte cap or the deadline.
#[derive(Clone, Copy, Debug)]
pub(super) struct StepTransfer {
    /// Cumulative `sent`/`received` this transfer must end at, established
    /// before the first byte moved.
    expected_total: u64,
    /// The immutable per-worker wall deadline established before spawn.
    deadline: Instant,
    offset: usize,
    len: usize,
}

impl StepTransfer {
    pub(super) const fn offset(self) -> usize {
        self.offset
    }

    fn advance(&mut self, count: usize) {
        self.offset += count;
    }

    const fn is_complete(self) -> bool {
        self.offset == self.len
    }
}

impl BoundedWorkerIo {
    /// Creates one transport budget for the complete worker lifetime.
    ///
    /// Both ceilings include handshake bytes and every later frame header and
    /// payload; they are not raw-query or result-payload allowances.
    pub(super) fn new(
        stdin: ChildStdin,
        stdout: ChildStdout,
        max_sent: u64,
        max_received: u64,
    ) -> Result<Self, SupervisorError> {
        set_nonblocking(stdin.as_raw_fd(), "make parser worker stdin nonblocking")?;
        set_nonblocking(stdout.as_raw_fd(), "make parser worker stdout nonblocking")?;
        Ok(Self {
            stdin: Some(stdin),
            stdout: Some(stdout),
            sent: 0,
            received: 0,
            max_sent,
            max_received,
        })
    }

    /// Admit a complete outbound transfer against the immutable cumulative
    /// input cap BEFORE any byte moves, returning the reservation the step
    /// methods require. Rejecting here (rather than mid-transfer) is what
    /// keeps a too-large frame from failing only after partial I/O.
    pub(super) fn reserve_write(
        &self,
        deadline: Instant,
        bytes: &[u8],
    ) -> Result<StepTransfer, SupervisorError> {
        ensure_before_deadline(deadline)?;
        let expected_total = prospective_total(self.sent, bytes.len(), self.max_sent)
            .ok_or(SupervisorError::InvalidState(INPUT_LIMIT_MESSAGE))?;
        Ok(StepTransfer {
            expected_total,
            deadline,
            offset: 0,
            len: bytes.len(),
        })
    }

    /// Inbound twin of [`Self::reserve_write`], against the output cap.
    pub(super) fn reserve_read(
        &self,
        deadline: Instant,
        len: usize,
    ) -> Result<StepTransfer, SupervisorError> {
        ensure_before_deadline(deadline)?;
        let expected_total = prospective_total(self.received, len, self.max_received)
            .ok_or(SupervisorError::InvalidState(OUTPUT_LIMIT_MESSAGE))?;
        Ok(StepTransfer {
            expected_total,
            deadline,
            offset: 0,
            len,
        })
    }

    /// Confirm a completed transfer landed on exactly its admitted total.
    pub(super) fn verify_write_total(&self, transfer: StepTransfer) -> Result<(), SupervisorError> {
        if self.sent != transfer.expected_total {
            return Err(SupervisorError::InvalidState(
                "parser worker input accounting drifted",
            ));
        }
        Ok(())
    }

    pub(super) fn verify_read_total(&self, transfer: StepTransfer) -> Result<(), SupervisorError> {
        if self.received != transfer.expected_total {
            return Err(SupervisorError::InvalidState(
                "parser worker output accounting drifted",
            ));
        }
        Ok(())
    }

    /// A descriptor-free fixture for the pure boundary checks in
    /// `io_tests::unit_tests`: it exercises admission/accounting arithmetic
    /// only, and any method that would touch a pipe fails closed because
    /// both descriptors are absent.
    #[cfg(test)]
    pub(super) fn descriptorless_for_test(
        sent: u64,
        received: u64,
        max_sent: u64,
        max_received: u64,
    ) -> Self {
        Self {
            stdin: None,
            stdout: None,
            sent,
            received,
            max_sent,
            max_received,
        }
    }

    pub(super) const fn max_sent(&self) -> u64 {
        self.max_sent
    }

    pub(super) fn write_all(
        &mut self,
        pidfd: &OwnedFd,
        deadline: Instant,
        bytes: &[u8],
    ) -> Result<(), SupervisorError> {
        let mut transfer = self.reserve_write(deadline, bytes)?;
        while !transfer.is_complete() {
            crate::parser_isolation::runtime::checkpoint()?;
            let timeout_ms = legacy_poll_timeout_ms(deadline)?;
            if self.write_step(pidfd, bytes, &mut transfer, Some(timeout_ms))?
                == StepOutcome::Complete
            {
                break;
            }
        }
        self.verify_write_total(transfer)
    }

    pub(super) fn read_exact(
        &mut self,
        pidfd: &OwnedFd,
        deadline: Instant,
        output: &mut [u8],
    ) -> Result<(), SupervisorError> {
        let mut transfer = self.reserve_read(deadline, output.len())?;
        while !transfer.is_complete() {
            crate::parser_isolation::runtime::checkpoint()?;
            let timeout_ms = legacy_poll_timeout_ms(deadline)?;
            if self.read_step(pidfd, output, &mut transfer, Some(timeout_ms))?
                == StepOutcome::Complete
            {
                break;
            }
        }
        self.verify_read_total(transfer)
    }

    /// One bounded write attempt against an admitted [`StepTransfer`].
    ///
    /// `poll_cap_ms` is an OPTIONAL extra cap (the async path passes
    /// `Some(10)` so no single step meaningfully blocks its thread; the
    /// synchronous loop passes its legacy thread-local-derived value). The
    /// effective poll timeout is always additionally clamped to the remaining
    /// immutable lifetime, and the deadline is rechecked immediately after
    /// the poll returns and BEFORE the `write(2)`, so a step can never
    /// perform I/O after expiry. Cumulative byte accounting is bounded by
    /// the reservation's admitted total, not just `checked_add`.
    pub(super) fn write_step(
        &mut self,
        pidfd: &OwnedFd,
        bytes: &[u8],
        transfer: &mut StepTransfer,
        poll_cap_ms: Option<i32>,
    ) -> Result<StepOutcome, SupervisorError> {
        let timeout_ms = step_poll_timeout_ms(transfer.deadline, poll_cap_ms)?;
        let descriptor = self
            .stdin
            .as_ref()
            .ok_or(SupervisorError::InvalidState(
                "parser worker stdin is closed",
            ))?
            .as_raw_fd();
        match poll_step(
            descriptor,
            pidfd.as_raw_fd(),
            libc::POLLOUT,
            timeout_ms,
            false,
        )? {
            PollStep::Pending => {
                // poll uses a millisecond ceiling: never let a rounded
                // timeout carry us past the immutable deadline.
                ensure_deadline_not_passed(transfer.deadline)?;
                return Ok(StepOutcome::Pending);
            }
            PollStep::Ready => {}
        }
        ensure_deadline_not_passed(transfer.deadline)?;
        // SAFETY: descriptor is live and the remaining slice is readable.
        let count = unsafe {
            libc::write(
                descriptor,
                bytes[transfer.offset..].as_ptr().cast(),
                bytes.len() - transfer.offset,
            )
        };
        if count > 0 {
            let count = count as usize;
            self.sent = admitted_total(self.sent, count, self.max_sent, INPUT_LIMIT_MESSAGE)?;
            transfer.advance(count);
            ensure_deadline_not_passed(transfer.deadline)?;
            return Ok(if transfer.is_complete() {
                StepOutcome::Complete
            } else {
                StepOutcome::Pending
            });
        }
        if count == 0 {
            return Err(SupervisorError::InvalidState(
                "parser worker stdin accepted a zero-byte write",
            ));
        }
        let error = std::io::Error::last_os_error();
        if matches!(
            error.kind(),
            std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
        ) {
            return Ok(StepOutcome::Pending);
        }
        Err(SupervisorError::operation("write parser worker input")(
            error,
        ))
    }

    /// One bounded read attempt; the read-side twin of [`Self::write_step`].
    pub(super) fn read_step(
        &mut self,
        pidfd: &OwnedFd,
        output: &mut [u8],
        transfer: &mut StepTransfer,
        poll_cap_ms: Option<i32>,
    ) -> Result<StepOutcome, SupervisorError> {
        let timeout_ms = step_poll_timeout_ms(transfer.deadline, poll_cap_ms)?;
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
            false,
        )? {
            PollStep::Pending => {
                ensure_deadline_not_passed(transfer.deadline)?;
                return Ok(StepOutcome::Pending);
            }
            PollStep::Ready => {}
        }
        ensure_deadline_not_passed(transfer.deadline)?;
        // SAFETY: descriptor is live and the remaining slice is writable.
        let count = unsafe {
            libc::read(
                descriptor,
                output[transfer.offset..].as_mut_ptr().cast(),
                output.len() - transfer.offset,
            )
        };
        if count > 0 {
            let count = count as usize;
            self.received = admitted_total(
                self.received,
                count,
                self.max_received,
                OUTPUT_LIMIT_MESSAGE,
            )?;
            transfer.advance(count);
            ensure_deadline_not_passed(transfer.deadline)?;
            return Ok(if transfer.is_complete() {
                StepOutcome::Complete
            } else {
                StepOutcome::Pending
            });
        }
        if count == 0 {
            return Err(SupervisorError::InvalidState(
                "parser worker output ended before the fixed read completed",
            ));
        }
        let error = std::io::Error::last_os_error();
        if matches!(
            error.kind(),
            std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
        ) {
            return Ok(StepOutcome::Pending);
        }
        Err(SupervisorError::operation("read parser worker output")(
            error,
        ))
    }

    /// Check a future fixed read against the immutable lifetime budget before
    /// the caller allocates storage for peer-declared bytes.
    pub(super) fn ensure_can_receive(
        &self,
        deadline: Instant,
        additional: usize,
    ) -> Result<(), SupervisorError> {
        ensure_before_deadline(deadline)?;
        prospective_total(self.received, additional, self.max_received)
            .map(|_| ())
            .ok_or(SupervisorError::InvalidState(OUTPUT_LIMIT_MESSAGE))
    }

    pub(super) fn close_stdin(&mut self) {
        self.stdin.take();
    }

    pub(super) fn close(&mut self) {
        self.stdin.take();
        self.stdout.take();
    }

    pub(super) const fn sent(&self) -> u64 {
        self.sent
    }

    pub(super) const fn received(&self) -> u64 {
        self.received
    }
}

fn set_nonblocking(descriptor: RawFd, operation: &'static str) -> Result<(), SupervisorError> {
    // SAFETY: F_GETFL and F_SETFL inspect/update the live parent-owned pipe end.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0
    {
        return Err(SupervisorError::operation(operation)(
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}
