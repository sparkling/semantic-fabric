//! Required-pidfd ownership and process-group-before-reap cleanup.

#[cfg(test)]
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::{Child, ChildStdin, ChildStdout, ExitStatus};
use std::time::{Duration, Instant};

use super::SupervisorError;

/// Live child ownership. The group leader is deliberately left unreaped until
/// its process group has been swept, preventing group-id reuse during cleanup.
pub(super) struct ParserWorkerProcess {
    pub(super) child: Option<Child>,
    pub(super) pidfd: OwnedFd,
    pub(super) process_group: libc::pid_t,
    pub(super) stdin: Option<ChildStdin>,
    pub(super) stdout: Option<ChildStdout>,
    pub(super) wall_deadline: Instant,
}

impl ParserWorkerProcess {
    #[cfg(test)]
    pub(super) fn stdin_mut(&mut self) -> Result<&mut ChildStdin, SupervisorError> {
        self.stdin
            .as_mut()
            .ok_or(SupervisorError::InvalidState("worker stdin is closed"))
    }

    #[cfg(test)]
    pub(super) fn stdout_mut(&mut self) -> Result<&mut ChildStdout, SupervisorError> {
        self.stdout
            .as_mut()
            .ok_or(SupervisorError::InvalidState("worker stdout is closed"))
    }

    #[cfg(test)]
    pub(super) fn write_all(&mut self, bytes: &[u8]) -> Result<(), SupervisorError> {
        self.stdin_mut()?
            .write_all(bytes)
            .map_err(SupervisorError::operation("write parser worker input"))
    }

    #[cfg(test)]
    pub(super) fn read_exact(&mut self, bytes: &mut [u8]) -> Result<(), SupervisorError> {
        self.stdout_mut()?
            .read_exact(bytes)
            .map_err(SupervisorError::operation("read parser worker output"))
    }

    #[cfg(test)]
    pub(super) fn close_stdin(&mut self) {
        self.stdin.take();
    }

    /// Waits only to the immutable V1 deadline established before spawn.
    pub(super) fn wait_until_deadline(&mut self) -> Result<ExitStatus, SupervisorError> {
        if self.child.is_none() {
            return Err(SupervisorError::InvalidState("worker is already reaped"));
        }
        let remaining = self.wall_deadline.saturating_duration_since(Instant::now());
        if !poll_pidfd(&self.pidfd, remaining)? {
            self.terminate_and_reap()?;
            return Err(SupervisorError::DeadlineExceeded);
        }
        self.sweep_and_reap()
    }

    pub(super) fn terminate_and_reap(&mut self) -> Result<ExitStatus, SupervisorError> {
        let child = self
            .child
            .as_mut()
            .ok_or(SupervisorError::InvalidState("worker is already reaped"))?;
        // Signal before closing stdin. Dropping the pipe first can let a
        // well-behaved worker observe EOF and exit successfully while an
        // explicit forced-termination path is still acquiring its targets.
        let group = kill_process_group(self.process_group);
        let exact = signal_pidfd(&self.pidfd, libc::SIGKILL);
        let fallback = if group.is_err() && exact.is_err() {
            child
                .kill()
                .map_err(SupervisorError::operation("fallback-kill parser worker"))
        } else {
            Ok(())
        };
        self.stdin.take();
        if group.is_err() && exact.is_err() && fallback.is_err() {
            return Err(group
                .err()
                .or_else(|| exact.err())
                .or_else(|| fallback.err())
                .expect("one signal error exists"));
        }
        let status = self.reap_exact()?;
        group?;
        exact?;
        fallback?;
        Ok(status)
    }

    #[cfg(test)]
    pub(super) fn duplicate_pidfd(&self) -> Result<OwnedFd, SupervisorError> {
        // SAFETY: fcntl duplicates the live pidfd and atomically sets CLOEXEC.
        let descriptor = unsafe { libc::fcntl(self.pidfd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 3) };
        if descriptor < 0 {
            return Err(SupervisorError::operation("duplicate parser worker pidfd")(
                std::io::Error::last_os_error(),
            ));
        }
        // SAFETY: ownership of the newly returned descriptor transfers here.
        Ok(unsafe { OwnedFd::from_raw_fd(descriptor) })
    }

    fn sweep_and_reap(&mut self) -> Result<ExitStatus, SupervisorError> {
        let child = self
            .child
            .as_ref()
            .ok_or(SupervisorError::InvalidState("worker is already reaped"))?;
        inspect_exited_without_reaping(child.id())?;
        let group = kill_process_group(self.process_group);
        let status = self.reap_exact()?;
        group?;
        Ok(status)
    }

    fn reap_exact(&mut self) -> Result<ExitStatus, SupervisorError> {
        self.stdin.take();
        self.stdout.take();
        self.child
            .take()
            .ok_or(SupervisorError::InvalidState("worker is already reaped"))?
            .wait()
            .map_err(SupervisorError::operation("reap parser worker"))
    }
}

impl Drop for ParserWorkerProcess {
    fn drop(&mut self) {
        if self.child.is_some() {
            // Last-resort containment only. Explicit lifecycle methods surface
            // failures; Drop cannot. A kernel D-state can still delay wait.
            let _ = self.terminate_and_reap();
        }
    }
}

pub(super) fn open_pidfd(pid: u32) -> Result<OwnedFd, SupervisorError> {
    // SAFETY: pidfd_open returns a new descriptor for this numeric direct child.
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if descriptor < 0 {
        return Err(SupervisorError::operation("open parser worker pidfd")(
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: ownership of the new descriptor transfers here.
    Ok(unsafe { OwnedFd::from_raw_fd(descriptor as RawFd) })
}

fn poll_pidfd(pidfd: &OwnedFd, timeout: Duration) -> Result<bool, SupervisorError> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(SupervisorError::InvalidLimits(
            "wall timeout overflows Instant",
        ))?;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let millis = remaining
            .as_millis()
            .saturating_add(u128::from(remaining.subsec_nanos() % 1_000_000 != 0));
        let timeout_ms = i32::try_from(millis.min(i32::MAX as u128)).unwrap_or(i32::MAX);
        let mut pollfd = libc::pollfd {
            fd: pidfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: pollfd is a valid one-element writable array.
        let result = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if result > 0 {
            if pollfd.revents & libc::POLLNVAL != 0 {
                return Err(SupervisorError::InvalidState("parser pidfd became invalid"));
            }
            return Ok(true);
        }
        if result == 0 {
            if Instant::now() >= deadline {
                return Ok(false);
            }
            continue;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(SupervisorError::operation("poll parser worker pidfd")(
                error,
            ));
        }
    }
}

fn inspect_exited_without_reaping(pid: u32) -> Result<(), SupervisorError> {
    let mut information = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    // SAFETY: waitid fills siginfo for this unreaped direct child; WNOWAIT keeps
    // the group leader allocated until the following group sweep.
    if unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            information.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    } != 0
    {
        return Err(SupervisorError::operation("inspect exited parser worker")(
            std::io::Error::last_os_error(),
        ));
    }
    // SAFETY: successful waitid initialized the SIGCHLD payload.
    if unsafe { information.assume_init().si_pid() } == 0 {
        return Err(SupervisorError::InvalidState(
            "pidfd was readable before the worker became waitable",
        ));
    }
    Ok(())
}

pub(super) fn signal_pidfd(pidfd: &OwnedFd, signal: libc::c_int) -> Result<(), SupervisorError> {
    // SAFETY: the live pidfd identifies the exact worker; null siginfo and zero
    // flags request ordinary signal delivery.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(SupervisorError::operation("signal exact parser worker")(
            error,
        ))
    }
}

pub(super) fn kill_process_group(process_group: libc::pid_t) -> Result<(), SupervisorError> {
    // SAFETY: the group leader remains our unreaped direct child, so the
    // negative id cannot refer to a reused process group.
    if unsafe { libc::kill(-process_group, libc::SIGKILL) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(SupervisorError::operation(
            "sweep parser worker process group",
        )(error))
    }
}

pub(super) fn terminate_unbound_child(child: &mut Child) -> Result<(), SupervisorError> {
    // Never return before attempting exact kill and wait: `Child::drop` does
    // not reap on every supported Rust platform.
    let group = i32::try_from(child.id())
        .map_err(|_| SupervisorError::InvalidState("worker PID exceeds process-group range"))
        .and_then(kill_process_group);
    let exact = child.kill().map_err(SupervisorError::operation(
        "kill worker after pidfd failure",
    ));
    let waited = child.wait().map_err(SupervisorError::operation(
        "reap worker after pidfd failure",
    ));
    group?;
    exact?;
    waited.map(|_| ())
}
