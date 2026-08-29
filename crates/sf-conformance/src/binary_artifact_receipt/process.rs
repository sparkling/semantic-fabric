//! Bounded subprocess execution for the capture lane.

use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
#[cfg(target_os = "linux")]
pub(super) const MAX_EXECVEAT_ARGUMENTS: usize = 1024;
#[cfg(target_os = "linux")]
pub(super) const MAX_EXECVEAT_ARGUMENT_BYTES: usize = 128 * 1024;

mod capture;
#[cfg(target_os = "linux")]
mod execveat;
use capture::{collect, first_error, has_error, join, read_limited, set_result};
#[cfg(target_os = "linux")]
pub(super) use execveat::Request as ExecveatRequest;

#[derive(Debug)]
pub(super) struct Output {
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

pub(super) fn run(
    mut command: Command,
    label: &str,
    max_stdout: u64,
    max_stderr: u64,
    timeout: Duration,
) -> Result<Output, String> {
    configure(&mut command);
    run_configured(command, label, max_stdout, max_stderr, timeout, false)
}

#[cfg(target_os = "linux")]
pub(super) fn run_execveat(
    request: ExecveatRequest,
    label: &str,
    max_stdout: u64,
    max_stderr: u64,
    timeout: Duration,
) -> Result<Output, String> {
    let mut command = Command::new("/proc/self/exe");
    configure(&mut command);
    execveat::install(&mut command, request)?;
    run_configured(command, label, max_stdout, max_stderr, timeout, true)
}

fn configure(command: &mut Command) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        // SAFETY: `umask(2)` is async-signal-safe and mutates only the child
        // between fork and exec. Capture outputs must not inherit a permissive
        // operator umask.
        unsafe {
            command.pre_exec(|| {
                libc::umask(0o022);
                Ok(())
            });
        }
    }
}

fn run_configured(
    mut command: Command,
    label: &str,
    max_stdout: u64,
    max_stderr: u64,
    timeout: Duration,
    require_pidfd: bool,
) -> Result<Output, String> {
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn {label}: {error}"))?;
    let pidfd = if require_pidfd {
        match open_pidfd(child.id()) {
            Ok(pidfd) => Some(pidfd),
            Err(error) => {
                let cleanup = terminate(&mut child, None);
                return Err(with_cleanup(
                    format!("bind {label} pidfd: {error}"),
                    cleanup,
                ));
            }
        }
    } else {
        None
    };
    let stdout = take_pipe(
        child.stdout.take(),
        &mut child,
        pidfd.as_ref(),
        format!("{label} stdout is unavailable"),
    )?;
    let stderr = take_pipe(
        child.stderr.take(),
        &mut child,
        pidfd.as_ref(),
        format!("{label} stderr is unavailable"),
    )?;
    let cancel = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::sync_channel(2);
    let stdout_reader = match read_limited(
        stdout,
        max_stdout,
        label.to_owned(),
        "stdout",
        sender.clone(),
        Arc::clone(&cancel),
    ) {
        Ok(reader) => reader,
        Err(error) => {
            let cleanup = terminate(&mut child, pidfd.as_ref());
            return Err(with_cleanup(error, cleanup));
        }
    };
    let stderr_reader = match read_limited(
        stderr,
        max_stderr,
        label.to_owned(),
        "stderr",
        sender,
        Arc::clone(&cancel),
    ) {
        Ok(reader) => reader,
        Err(error) => {
            cancel.store(true, Ordering::Release);
            let cleanup = terminate(&mut child, pidfd.as_ref());
            let readers = stdout_reader
                .join()
                .map_err(|_| format!("{label} stdout reader panicked"));
            readers?;
            return Err(with_cleanup(error, cleanup));
        }
    };
    let deadline = Instant::now() + timeout;
    let mut stdout_result = None;
    let mut stderr_result = None;
    let status = loop {
        if let Err(error) = collect(&receiver, &mut stdout_result, &mut stderr_result) {
            cancel.store(true, Ordering::Release);
            let cleanup = terminate(&mut child, pidfd.as_ref());
            join(stdout_reader, stderr_reader, label)?;
            cleanup?;
            return Err(error);
        }
        if stdout_result.as_ref().is_some_and(Result::is_err)
            || stderr_result.as_ref().is_some_and(Result::is_err)
        {
            cancel.store(true, Ordering::Release);
            let cleanup = terminate(&mut child, pidfd.as_ref());
            join(stdout_reader, stderr_reader, label)?;
            cleanup?;
            return first_error(stdout_result, stderr_result);
        }
        match poll_exit(&mut child, label) {
            Ok(ExitPoll::Running) => {}
            Ok(ExitPoll::Reaped(Ok(status))) => break status,
            Ok(ExitPoll::Reaped(Err(error))) => {
                cancel.store(true, Ordering::Release);
                join(stdout_reader, stderr_reader, label)?;
                return Err(error);
            }
            Err(error) => {
                cancel.store(true, Ordering::Release);
                let cleanup = terminate(&mut child, pidfd.as_ref());
                join(stdout_reader, stderr_reader, label)?;
                cleanup?;
                return Err(error);
            }
        }
        if Instant::now() >= deadline {
            cancel.store(true, Ordering::Release);
            let cleanup = terminate(&mut child, pidfd.as_ref());
            join(stdout_reader, stderr_reader, label)?;
            cleanup?;
            return Err(format!(
                "{label} timed out after {} seconds",
                timeout.as_secs_f64()
            ));
        }
        thread::sleep(POLL_INTERVAL);
    };
    while stdout_result.is_none() || stderr_result.is_none() {
        match receiver.recv_timeout(DRAIN_TIMEOUT) {
            Ok((stream, result)) => {
                if let Err(error) =
                    set_result(stream, result, &mut stdout_result, &mut stderr_result)
                {
                    cancel.store(true, Ordering::Release);
                    join(stdout_reader, stderr_reader, label)?;
                    return Err(error);
                }
                if has_error(&stdout_result, &stderr_result) {
                    cancel.store(true, Ordering::Release);
                    join(stdout_reader, stderr_reader, label)?;
                    return first_error(stdout_result, stderr_result);
                }
            }
            Err(_) => {
                cancel.store(true, Ordering::Release);
                join(stdout_reader, stderr_reader, label)?;
                return Err(format!("{label} output did not close after process exit"));
            }
        }
    }
    join(stdout_reader, stderr_reader, label)?;
    let stdout = stdout_result.expect("stdout was collected")?;
    let stderr = stderr_result.expect("stderr was collected")?;
    if !status.success() {
        return Err(format!(
            "{label} failed with {status}: {}",
            lossy_tail(&stderr)
        ));
    }
    Ok(Output { stdout, stderr })
}

fn terminate(child: &mut Child, pidfd: Option<&std::fs::File>) -> Result<(), String> {
    let exact = match pidfd {
        Some(pidfd) => signal_pidfd(pidfd),
        None => signal_child(child),
    };
    let group = kill_process_group(child.id());
    let signalled = exact.is_ok() || group.is_ok();
    let waited = signalled
        .then(|| {
            child
                .wait()
                .map_err(|error| format!("reap terminated subprocess: {error}"))
        })
        .transpose();
    exact?;
    group?;
    waited?
        .ok_or_else(|| "subprocess could not be signalled".to_owned())
        .map(|_| ())
}

fn with_cleanup(primary: String, cleanup: Result<(), String>) -> String {
    match cleanup {
        Ok(()) => primary,
        Err(error) => format!("{primary}; cleanup failed: {error}"),
    }
}

fn take_pipe<T>(
    pipe: Option<T>,
    child: &mut Child,
    pidfd: Option<&std::fs::File>,
    missing: String,
) -> Result<T, String> {
    pipe.ok_or_else(|| with_cleanup(missing, terminate(child, pidfd)))
}

#[cfg(unix)]
fn signal_child(child: &mut Child) -> Result<(), String> {
    let pid = i32::try_from(child.id()).map_err(|_| "subprocess PID exceeds i32".to_owned())?;
    // SAFETY: the direct child is live and unreaped, so its numeric PID cannot
    // be reused during this signal attempt.
    if unsafe { libc::kill(pid, libc::SIGKILL) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(format!("terminate exact subprocess {pid}: {error}"))
    }
}

#[cfg(not(unix))]
fn signal_child(child: &mut Child) -> Result<(), String> {
    child
        .kill()
        .map_err(|error| format!("terminate exact subprocess: {error}"))
}

#[cfg(unix)]
fn kill_process_group(pid: u32) -> Result<(), String> {
    let process_group =
        i32::try_from(pid).map_err(|_| "subprocess PID exceeds process-group range".to_owned())?;
    // SAFETY: the child starts a new process group whose ID is its PID; a
    // negative PID addresses only that group while the leader is unreaped.
    if unsafe { libc::kill(-process_group, libc::SIGKILL) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(format!(
            "terminate subprocess process group {process_group}: {error}"
        ))
    }
}

#[cfg(not(unix))]
fn kill_process_group(_pid: u32) -> Result<(), String> {
    Ok(())
}

enum ExitPoll {
    Running,
    Reaped(Result<std::process::ExitStatus, String>),
}

#[cfg(target_os = "linux")]
fn poll_exit(child: &mut Child, label: &str) -> Result<ExitPoll, String> {
    let pid = i32::try_from(child.id()).map_err(|_| format!("{label} PID exceeds i32"))?;
    let mut information = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    // SAFETY: waitid initializes siginfo_t for this direct child. WNOWAIT keeps
    // the group leader unreaped until every descendant in its isolated group
    // has been swept, preventing process-group ID reuse.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            information.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(format!(
            "inspect {label} without reaping: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: successful waitid initialized the value above.
    let information = unsafe { information.assume_init() };
    // SAFETY: si_pid is valid for the SIGCHLD payload returned by waitid.
    if unsafe { information.si_pid() } == 0 {
        return Ok(ExitPoll::Running);
    }
    let group = kill_process_group(child.id());
    let waited = child
        .wait()
        .map_err(|error| format!("reap exited {label}: {error}"));
    Ok(ExitPoll::Reaped(group.and(waited)))
}

#[cfg(not(target_os = "linux"))]
fn poll_exit(child: &mut Child, label: &str) -> Result<ExitPoll, String> {
    match child
        .try_wait()
        .map_err(|error| format!("wait for {label}: {error}"))?
    {
        None => Ok(ExitPoll::Running),
        Some(status) => Ok(ExitPoll::Reaped(
            kill_process_group(child.id()).map(|()| status),
        )),
    }
}

#[cfg(target_os = "linux")]
fn open_pidfd(pid: u32) -> Result<std::fs::File, String> {
    use std::os::fd::FromRawFd;

    // SAFETY: pidfd_open takes a numeric child PID and returns a new owned fd.
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: ownership of the newly returned descriptor transfers here.
    Ok(unsafe { std::fs::File::from_raw_fd(descriptor as i32) })
}

#[cfg(not(target_os = "linux"))]
fn open_pidfd(_pid: u32) -> Result<std::fs::File, String> {
    Err("pidfd requires Linux".to_owned())
}

#[cfg(target_os = "linux")]
fn signal_pidfd(pidfd: &std::fs::File) -> Result<(), String> {
    use std::os::fd::AsRawFd;

    // SAFETY: the pidfd is live; null siginfo and flags zero request SIGKILL.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            libc::SIGKILL,
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
        Err(format!("signal subprocess through pidfd: {error}"))
    }
}

#[cfg(not(target_os = "linux"))]
fn signal_pidfd(_pidfd: &std::fs::File) -> Result<(), String> {
    Err("pidfd requires Linux".to_owned())
}

fn lossy_tail(bytes: &[u8]) -> String {
    const MAX_BYTES: usize = 1024;
    let start = bytes.len().saturating_sub(MAX_BYTES);
    String::from_utf8_lossy(&bytes[start..]).replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn captures_both_streams() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf out; printf err >&2"]);
        let output = run(command, "fixture", 16, 16, Duration::from_secs(1)).unwrap();
        assert_eq!(output.stdout, b"out");
        assert_eq!(output.stderr, b"err");
    }

    #[cfg(unix)]
    #[test]
    fn fixes_the_child_umask() {
        let mut command = Command::new("sh");
        command.args(["-c", "umask"]);
        let output = run(command, "fixture", 16, 16, Duration::from_secs(1)).unwrap();
        assert_eq!(output.stdout, b"0022\n");
    }

    #[cfg(unix)]
    #[test]
    fn bounds_and_times_out() {
        let mut oversized = Command::new("sh");
        oversized.args(["-c", "printf 12345"]);
        assert!(run(oversized, "fixture", 4, 4, Duration::from_secs(1))
            .unwrap_err()
            .contains("exceeds"));
        let mut slow = Command::new("sh");
        slow.args(["-c", "sleep 5"]);
        assert!(run(slow, "fixture", 4, 4, Duration::from_millis(50))
            .unwrap_err()
            .contains("timed out"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn sweeps_same_group_descendants_when_the_leader_exits() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & printf '%s\\n' \"$!\""]);
        let output = run(
            command,
            "descendant fixture",
            32,
            16,
            Duration::from_secs(1),
        )
        .unwrap();
        let pid: i32 = String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while process_is_running(pid) && Instant::now() < deadline {
            thread::sleep(POLL_INTERVAL);
        }
        assert!(
            !process_is_running(pid),
            "descendant {pid} survived cleanup"
        );
    }

    #[cfg(target_os = "linux")]
    fn process_is_running(pid: i32) -> bool {
        // SAFETY: signal zero only probes a numeric process identifier.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return false;
        }
        let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        state
            .split_once(") ")
            .and_then(|(_, suffix)| suffix.chars().next())
            .is_some_and(|state| state != 'Z')
    }
}
