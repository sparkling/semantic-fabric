//! Cumulative byte bounds and one-deadline pipe I/O for a parser worker.

use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::process::{ChildStdin, ChildStdout};
use std::time::Instant;

use super::SupervisorError;

const INPUT_LIMIT_MESSAGE: &str = "parser worker input exceeds its cumulative byte limit";
const OUTPUT_LIMIT_MESSAGE: &str = "parser worker output exceeds its cumulative byte limit";

pub(super) struct BoundedWorkerIo {
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    sent: u64,
    received: u64,
    max_sent: u64,
    max_received: u64,
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

    pub(super) fn write_all(
        &mut self,
        pidfd: &OwnedFd,
        deadline: Instant,
        bytes: &[u8],
    ) -> Result<(), SupervisorError> {
        ensure_before_deadline(deadline)?;
        let expected = prospective_total(self.sent, bytes.len(), self.max_sent)
            .ok_or(SupervisorError::InvalidState(INPUT_LIMIT_MESSAGE))?;
        let descriptor = self
            .stdin
            .as_ref()
            .ok_or(SupervisorError::InvalidState(
                "parser worker stdin is closed",
            ))?
            .as_raw_fd();
        let mut offset = 0;
        while offset < bytes.len() {
            wait_ready(descriptor, pidfd.as_raw_fd(), libc::POLLOUT, deadline)?;
            // SAFETY: descriptor is live and the remaining slice is readable.
            let count = unsafe {
                libc::write(
                    descriptor,
                    bytes[offset..].as_ptr().cast(),
                    bytes.len() - offset,
                )
            };
            if count > 0 {
                let count = count as usize;
                offset += count;
                self.sent = self
                    .sent
                    .checked_add(count as u64)
                    .ok_or(SupervisorError::InvalidState(INPUT_LIMIT_MESSAGE))?;
                if Instant::now() >= deadline {
                    return Err(SupervisorError::DeadlineExceeded);
                }
                continue;
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
                continue;
            }
            return Err(SupervisorError::operation("write parser worker input")(
                error,
            ));
        }
        if self.sent != expected {
            return Err(SupervisorError::InvalidState(
                "parser worker input accounting drifted",
            ));
        }
        Ok(())
    }

    pub(super) fn read_exact(
        &mut self,
        pidfd: &OwnedFd,
        deadline: Instant,
        output: &mut [u8],
    ) -> Result<(), SupervisorError> {
        ensure_before_deadline(deadline)?;
        let expected = prospective_total(self.received, output.len(), self.max_received)
            .ok_or(SupervisorError::InvalidState(OUTPUT_LIMIT_MESSAGE))?;
        let descriptor = self
            .stdout
            .as_ref()
            .ok_or(SupervisorError::InvalidState(
                "parser worker stdout is closed",
            ))?
            .as_raw_fd();
        let mut offset = 0;
        while offset < output.len() {
            wait_ready(descriptor, pidfd.as_raw_fd(), libc::POLLIN, deadline)?;
            // SAFETY: descriptor is live and the remaining slice is writable.
            let count = unsafe {
                libc::read(
                    descriptor,
                    output[offset..].as_mut_ptr().cast(),
                    output.len() - offset,
                )
            };
            if count > 0 {
                let count = count as usize;
                offset += count;
                self.received = self
                    .received
                    .checked_add(count as u64)
                    .ok_or(SupervisorError::InvalidState(OUTPUT_LIMIT_MESSAGE))?;
                if Instant::now() >= deadline {
                    return Err(SupervisorError::DeadlineExceeded);
                }
                continue;
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
                continue;
            }
            return Err(SupervisorError::operation("read parser worker output")(
                error,
            ));
        }
        if self.received != expected {
            return Err(SupervisorError::InvalidState(
                "parser worker output accounting drifted",
            ));
        }
        Ok(())
    }

    pub(super) fn close_stdin(&mut self) {
        self.stdin.take();
    }

    pub(super) fn close(&mut self) {
        self.stdin.take();
        self.stdout.take();
    }

    #[cfg(test)]
    pub(super) const fn sent(&self) -> u64 {
        self.sent
    }

    #[cfg(test)]
    pub(super) const fn received(&self) -> u64 {
        self.received
    }
}

fn ensure_before_deadline(deadline: Instant) -> Result<(), SupervisorError> {
    if Instant::now() >= deadline {
        Err(SupervisorError::DeadlineExceeded)
    } else {
        Ok(())
    }
}

fn prospective_total(current: u64, additional: usize, maximum: u64) -> Option<u64> {
    let additional = u64::try_from(additional).ok()?;
    let total = current.checked_add(additional)?;
    (total <= maximum).then_some(total)
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

fn wait_ready(
    descriptor: RawFd,
    pidfd: RawFd,
    requested: libc::c_short,
    deadline: Instant,
) -> Result<(), SupervisorError> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(SupervisorError::DeadlineExceeded);
        }
        let millis = remaining.as_millis().saturating_add(u128::from(
            !remaining.subsec_nanos().is_multiple_of(1_000_000),
        ));
        let timeout = i32::try_from(millis.min(i32::MAX as u128)).unwrap_or(i32::MAX);
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
            unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, timeout) };
        if result == 0 {
            continue;
        }
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(SupervisorError::operation("poll parser worker I/O")(error));
        }
        // poll uses a millisecond ceiling. Never accept readiness observed only
        // after the immutable deadline because that rounded timeout elapsed.
        if Instant::now() >= deadline {
            return Err(SupervisorError::DeadlineExceeded);
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
            return Ok(());
        }
        if worker_exited {
            return Err(SupervisorError::InvalidState(
                "parser worker exited before fixed I/O completed",
            ));
        }
        if io_ready {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use super::prospective_total;

    #[test]
    fn prospective_totals_are_checked_before_io() {
        assert_eq!(prospective_total(3, 2, 5), Some(5));
        assert_eq!(prospective_total(3, 3, 5), None);
        assert_eq!(prospective_total(u64::MAX, 1, u64::MAX), None);
    }
}
