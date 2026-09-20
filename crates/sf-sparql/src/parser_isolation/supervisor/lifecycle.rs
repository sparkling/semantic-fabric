//! Required-pidfd ownership and process-group-before-reap cleanup.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::process::{Child, ExitStatus};
use std::time::Instant;

use super::io::{BoundedWorkerIo, StepOutcome, StepTransfer};
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

    /// One bounded, contained write attempt (see [`BoundedWorkerIo::write_step`]).
    /// `poll_timeout_ms` is the caller's own explicit short cap -- never the
    /// legacy thread-local -- so this is safe to call from an async loop that
    /// yields between `Pending` results.
    pub(super) fn reserve_write(&self, bytes: &[u8]) -> Result<StepTransfer, SupervisorError> {
        self.io.reserve_write(self.wall_deadline, bytes)
    }

    pub(super) fn reserve_read(&self, len: usize) -> Result<StepTransfer, SupervisorError> {
        self.io.reserve_read(self.wall_deadline, len)
    }

    pub(super) fn verify_write_total(&self, transfer: StepTransfer) -> Result<(), SupervisorError> {
        self.io.verify_write_total(transfer)
    }

    pub(super) fn verify_read_total(&self, transfer: StepTransfer) -> Result<(), SupervisorError> {
        self.io.verify_read_total(transfer)
    }

    pub(super) fn write_step_contained(
        &mut self,
        bytes: &[u8],
        transfer: &mut StepTransfer,
        poll_cap_ms: i32,
    ) -> Result<StepOutcome, SupervisorError> {
        let result = self
            .io
            .write_step(&self.pidfd, bytes, transfer, Some(poll_cap_ms));
        self.contain_step_result(result)
    }

    /// One bounded, contained read attempt (see [`BoundedWorkerIo::read_step`]).
    pub(super) fn read_step_contained(
        &mut self,
        output: &mut [u8],
        transfer: &mut StepTransfer,
        poll_cap_ms: i32,
    ) -> Result<StepOutcome, SupervisorError> {
        let result = self
            .io
            .read_step(&self.pidfd, output, transfer, Some(poll_cap_ms));
        self.contain_step_result(result)
    }

    /// One bounded, contained EOF-probe attempt (see
    /// [`BoundedWorkerIo::expect_eof_step`]).
    pub(super) fn expect_eof_step_contained(
        &mut self,
        poll_cap_ms: i32,
    ) -> Result<StepOutcome, SupervisorError> {
        let deadline = self.wall_deadline;
        let result = self
            .io
            .expect_eof_step(&self.pidfd, deadline, Some(poll_cap_ms));
        self.contain_step_result(result)
    }

    /// Route a caller-side failure (e.g. a request-control checkpoint) through
    /// the same containment path as an I/O failure, so a cleanup error takes
    /// precedence over the original cause instead of being suppressed by the
    /// `Drop` backstop.
    pub(super) fn contain_caller_failure(&mut self, primary: SupervisorError) -> SupervisorError {
        self.contain_live_failure(primary)
    }

    fn contain_step_result(
        &mut self,
        result: Result<StepOutcome, SupervisorError>,
    ) -> Result<StepOutcome, SupervisorError> {
        match result {
            Ok(outcome) => Ok(outcome),
            Err(primary) => Err(self.contain_live_failure(primary)),
        }
    }

    pub(super) fn observe_alive_and_silent_until(
        &mut self,
        observation_deadline: Instant,
    ) -> Result<(), SupervisorError> {
        let result = self.io.observe_alive_and_silent_until(
            &self.pidfd,
            observation_deadline,
            self.wall_deadline,
        );
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

    /// One bounded pidfd-readiness poll, capped by the caller's own explicit
    /// `timeout_ms` (not the legacy thread-local `poll_timeout`). The async
    /// bounded-step wait loops this and cooperatively yields between
    /// `Pending` results instead of blocking its own thread; once it
    /// observes `Exited` it must call [`Self::finish_wait_after_exit`].
    pub(super) fn wait_step(&mut self, poll_cap_ms: i32) -> Result<PidfdPollStep, SupervisorError> {
        if self.child.is_none() {
            return Err(SupervisorError::InvalidState("worker is already reaped"));
        }
        // Clamp to the remaining immutable lifetime and refuse to begin after
        // expiry, exactly as the synchronous `poll_pidfd` loop does.
        let deadline = self.wall_deadline;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(self.expire_and_reap());
        }
        let millis = remaining.as_millis().saturating_add(u128::from(
            !remaining.subsec_nanos().is_multiple_of(1_000_000),
        ));
        let remaining_ms = i32::try_from(millis.min(i32::MAX as u128)).unwrap_or(i32::MAX);
        let step = match poll_pidfd_once(&self.pidfd, remaining_ms.min(poll_cap_ms)) {
            Ok(step) => step,
            Err(primary) => return Err(self.contain_live_failure(primary)),
        };
        // poll uses a millisecond ceiling: never accept an exit (and never
        // proceed to sweep/reap as a success) observed only after the
        // immutable deadline elapsed, matching `poll_pidfd`'s own recheck.
        if Instant::now() >= deadline {
            return Err(self.expire_and_reap());
        }
        Ok(step)
    }

    /// The immutable lifetime elapsed: terminate, sweep and reap the child
    /// before reporting `DeadlineExceeded`, so no capacity is released before
    /// terminal cleanup and a containment failure still takes precedence.
    fn expire_and_reap(&mut self) -> SupervisorError {
        match self.terminate_and_reap() {
            Ok(_) => SupervisorError::DeadlineExceeded,
            Err(containment) => containment,
        }
    }

    /// Complete the wait after [`Self::wait_step`] observed `Exited`: sweep
    /// the process group and reap exactly, the same terminal sequence
    /// `wait_until_deadline` always ran once its own poll succeeded.
    pub(super) fn finish_wait_after_exit(&mut self) -> Result<ExitStatus, SupervisorError> {
        self.sweep_and_reap_with(inspect_exited_without_reaping)
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

    #[cfg(any(test, feature = "sql-canonicalize-evidence"))]
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

/// Outcome of exactly one bounded pidfd-readiness poll: pure, no internal
/// retry loop and no thread-local dependency. `poll_pidfd` (the synchronous
/// wait, capped by the legacy thread-local `poll_timeout`) loops this
/// exactly as it always polled; the async bounded-step wait calls it once
/// per step with its own explicit short cap and cooperatively yields
/// between `Pending` results.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PidfdPollStep {
    Exited,
    Pending,
}

pub(super) fn poll_pidfd_once(
    pidfd: &OwnedFd,
    timeout_ms: i32,
) -> Result<PidfdPollStep, SupervisorError> {
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
        return Ok(PidfdPollStep::Exited);
    }
    if result == 0 {
        return Ok(PidfdPollStep::Pending);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::Interrupted {
        return Ok(PidfdPollStep::Pending);
    }
    Err(SupervisorError::operation("poll parser worker pidfd")(
        error,
    ))
}

fn poll_pidfd(pidfd: &OwnedFd, deadline: Instant) -> Result<bool, SupervisorError> {
    loop {
        crate::parser_isolation::runtime::checkpoint()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(false);
        }
        let millis = remaining.as_millis().saturating_add(u128::from(
            !remaining.subsec_nanos().is_multiple_of(1_000_000),
        ));
        let timeout_ms = crate::parser_isolation::runtime::poll_timeout(
            i32::try_from(millis.min(i32::MAX as u128)).unwrap_or(i32::MAX),
        );
        match poll_pidfd_once(pidfd, timeout_ms)? {
            PidfdPollStep::Pending => {
                if Instant::now() >= deadline {
                    return Ok(false);
                }
                continue;
            }
            PidfdPollStep::Exited => {
                if Instant::now() >= deadline {
                    return Ok(false);
                }
                return Ok(true);
            }
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
#[path = "lifecycle_tests.rs"]
mod lifecycle_tests;
