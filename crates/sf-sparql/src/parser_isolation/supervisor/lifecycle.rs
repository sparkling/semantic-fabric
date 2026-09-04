//! Required-pidfd ownership and process-group-before-reap cleanup.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::{Child, ExitStatus};
use std::time::Instant;

use super::io::BoundedWorkerIo;
use super::SupervisorError;

/// Live child ownership. The group leader is deliberately left unreaped until
/// its process group has been swept, preventing group-id reuse during cleanup.
pub(super) struct ParserWorkerProcess {
    pub(super) child: Option<Child>,
    pub(super) pidfd: OwnedFd,
    pub(super) process_group: libc::pid_t,
    pub(super) io: BoundedWorkerIo,
    pub(super) wall_deadline: Instant,
}

impl ParserWorkerProcess {
    pub(super) fn write_all_until_deadline(&mut self, bytes: &[u8]) -> Result<(), SupervisorError> {
        let result = self.io.write_all(&self.pidfd, self.wall_deadline, bytes);
        self.contain_io_result(result)
    }

    pub(super) fn read_exact_until_deadline(
        &mut self,
        bytes: &mut [u8],
    ) -> Result<(), SupervisorError> {
        let result = self.io.read_exact(&self.pidfd, self.wall_deadline, bytes);
        self.contain_io_result(result)
    }

    /// Contain a peer whose declared output cannot fit the remaining
    /// whole-worker budget, before allocating storage for that declaration.
    pub(super) fn ensure_can_receive(&mut self, additional: usize) -> Result<(), SupervisorError> {
        let result = self.io.ensure_can_receive(self.wall_deadline, additional);
        self.contain_io_result(result)
    }

    pub(super) fn expect_stdout_eof_until_deadline(&mut self) -> Result<(), SupervisorError> {
        let result = self.io.expect_eof(&self.pidfd, self.wall_deadline);
        self.contain_io_result(result)
    }

    pub(super) fn close_stdin(&mut self) {
        self.io.close_stdin();
    }

    pub(super) const fn sent_bytes(&self) -> u64 {
        self.io.sent()
    }

    pub(super) const fn received_bytes(&self) -> u64 {
        self.io.received()
    }

    fn contain_io_result(
        &mut self,
        result: Result<(), SupervisorError>,
    ) -> Result<(), SupervisorError> {
        match result {
            Ok(()) => Ok(()),
            Err(primary) => Err(self.contain_live_failure(primary)),
        }
    }

    /// Waits only to the immutable V1 deadline established before spawn.
    pub(super) fn wait_until_deadline(&mut self) -> Result<ExitStatus, SupervisorError> {
        self.wait_until_deadline_with(poll_pidfd, inspect_exited_without_reaping)
    }

    fn wait_until_deadline_with<P, I>(
        &mut self,
        poll: P,
        inspect: I,
    ) -> Result<ExitStatus, SupervisorError>
    where
        P: FnOnce(&OwnedFd, Instant) -> Result<bool, SupervisorError>,
        I: FnOnce(u32) -> Result<(), SupervisorError>,
    {
        if self.child.is_none() {
            return Err(SupervisorError::InvalidState("worker is already reaped"));
        }
        let exited = match poll(&self.pidfd, self.wall_deadline) {
            Ok(exited) => exited,
            Err(primary) => return Err(self.contain_live_failure(primary)),
        };
        if !exited {
            self.terminate_and_reap()?;
            return Err(SupervisorError::DeadlineExceeded);
        }
        self.sweep_and_reap_with(inspect)
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
        self.io.close_stdin();
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

    fn sweep_and_reap_with<I>(&mut self, inspect: I) -> Result<ExitStatus, SupervisorError>
    where
        I: FnOnce(u32) -> Result<(), SupervisorError>,
    {
        let child_id = self
            .child
            .as_ref()
            .ok_or(SupervisorError::InvalidState("worker is already reaped"))?
            .id();
        if let Err(primary) = inspect(child_id) {
            return Err(self.contain_live_failure(primary));
        }
        let group = kill_process_group(self.process_group);
        let status = self.reap_exact()?;
        group?;
        Ok(status)
    }

    pub(super) fn contain_live_failure(&mut self, primary: SupervisorError) -> SupervisorError {
        match self.terminate_and_reap() {
            Ok(_) => primary,
            Err(containment) => containment,
        }
    }

    fn reap_exact(&mut self) -> Result<ExitStatus, SupervisorError> {
        self.io.close();
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

fn poll_pidfd(pidfd: &OwnedFd, deadline: Instant) -> Result<bool, SupervisorError> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        let millis = remaining.as_millis().saturating_add(u128::from(
            !remaining.subsec_nanos().is_multiple_of(1_000_000),
        ));
        let timeout_ms = i32::try_from(millis.min(i32::MAX as u128)).unwrap_or(i32::MAX);
        let mut pollfd = libc::pollfd {
            fd: pidfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: pollfd is a valid one-element writable array.
        let result = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if result > 0 {
            if Instant::now() >= deadline {
                return Ok(false);
            }
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

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::os::fd::{AsRawFd, OwnedFd};

    use super::*;
    use crate::parser_isolation::profile::v1_limits;
    use crate::parser_isolation::supervisor::executable::PreparedParserExecutable;
    use crate::parser_isolation::supervisor::linux::spawn_fixture;

    #[test]
    fn pidfd_poll_error_is_contained_and_reaped_before_recovery() {
        let executable = prepared_cat();
        let mut child = live_cat(&executable);
        let pidfd = child.duplicate_pidfd().expect("duplicate pidfd");

        let error = child
            .wait_until_deadline_with(
                |_, _| Err(SupervisorError::InvalidState("injected pidfd poll failure")),
                |_| Ok(()),
            )
            .expect_err("poll failure must fail closed");

        assert!(matches!(
            error,
            SupervisorError::InvalidState("injected pidfd poll failure")
        ));
        assert!(child.child.is_none());
        assert!(!pidfd_targets_live_process(&pidfd));
        successful_round_trip(&executable);
    }

    #[test]
    fn exited_inspection_error_is_contained_and_reaped_before_recovery() {
        let executable = prepared_cat();
        let mut child = live_cat(&executable);
        let pidfd = child.duplicate_pidfd().expect("duplicate pidfd");

        let error = child
            .wait_until_deadline_with(
                |_, _| Ok(true),
                |_| {
                    Err(SupervisorError::InvalidState(
                        "injected exited inspection failure",
                    ))
                },
            )
            .expect_err("inspection failure must fail closed");

        assert!(matches!(
            error,
            SupervisorError::InvalidState("injected exited inspection failure")
        ));
        assert!(child.child.is_none());
        assert!(!pidfd_targets_live_process(&pidfd));
        successful_round_trip(&executable);
    }

    fn prepared_cat() -> PreparedParserExecutable {
        PreparedParserExecutable::from_file_for_test(
            File::open("/bin/cat").expect("open cat fixture"),
            true,
        )
        .expect("prepare cat fixture")
    }

    fn live_cat(executable: &PreparedParserExecutable) -> ParserWorkerProcess {
        spawn_fixture(executable, v1_limits(), &[b"cat", b"-"]).expect("launch cat fixture")
    }

    fn successful_round_trip(executable: &PreparedParserExecutable) {
        let mut child = live_cat(executable);
        child.write_all_until_deadline(b"R").expect("write byte");
        let mut output = [0_u8; 1];
        child
            .read_exact_until_deadline(&mut output)
            .expect("read echoed byte");
        assert_eq!(output, [b'R']);
        child.close_stdin();
        assert!(child.wait_until_deadline().expect("reap cat").success());
    }

    fn pidfd_targets_live_process(pidfd: &OwnedFd) -> bool {
        // SAFETY: signal zero only probes the process identified by this live pidfd.
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                pidfd.as_raw_fd(),
                0,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result == 0 {
            true
        } else {
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
            false
        }
    }
}
