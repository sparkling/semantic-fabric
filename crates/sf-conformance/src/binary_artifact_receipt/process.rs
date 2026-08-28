//! Bounded subprocess execution for the capture lane.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

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
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn {label}: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{label} stdout is unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{label} stderr is unavailable"))?;
    let (sender, receiver) = mpsc::sync_channel(2);
    let stdout_reader = read_limited(
        stdout,
        max_stdout,
        label.to_owned(),
        "stdout",
        sender.clone(),
    );
    let stderr_reader = read_limited(stderr, max_stderr, label.to_owned(), "stderr", sender);
    let deadline = Instant::now() + timeout;
    let mut stdout_result = None;
    let mut stderr_result = None;
    let status = loop {
        if let Err(error) = collect(&receiver, &mut stdout_result, &mut stderr_result) {
            terminate(&mut child);
            join(stdout_reader, stderr_reader, label)?;
            return Err(error);
        }
        if stdout_result.as_ref().is_some_and(Result::is_err)
            || stderr_result.as_ref().is_some_and(Result::is_err)
        {
            terminate(&mut child);
            join(stdout_reader, stderr_reader, label)?;
            return first_error(stdout_result, stderr_result);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("wait for {label}: {error}"))?
        {
            kill_process_group(child.id());
            break status;
        }
        if Instant::now() >= deadline {
            terminate(&mut child);
            join(stdout_reader, stderr_reader, label)?;
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
                set_result(stream, result, &mut stdout_result, &mut stderr_result)?
            }
            Err(_) => {
                terminate(&mut child);
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

type ResultMessage = (Stream, Result<Vec<u8>, String>);

#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

fn read_limited<R: Read + Send + 'static>(
    reader: R,
    limit: u64,
    label: String,
    stream: &'static str,
    sender: mpsc::SyncSender<ResultMessage>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = reader
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read {label} {stream}: {error}"))
            .and_then(|_| {
                if bytes.len() as u64 > limit {
                    Err(format!("{label} {stream} exceeds {limit} bytes"))
                } else {
                    Ok(bytes)
                }
            });
        let kind = if stream == "stdout" {
            Stream::Stdout
        } else {
            Stream::Stderr
        };
        let _ = sender.send((kind, result));
    })
}

fn collect(
    receiver: &mpsc::Receiver<ResultMessage>,
    stdout: &mut Option<Result<Vec<u8>, String>>,
    stderr: &mut Option<Result<Vec<u8>, String>>,
) -> Result<(), String> {
    loop {
        match receiver.try_recv() {
            Ok((stream, result)) => set_result(stream, result, stdout, stderr)?,
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) if stdout.is_some() && stderr.is_some() => {
                return Ok(())
            }
            Err(TryRecvError::Disconnected) => {
                return Err("subprocess output reader stopped unexpectedly".to_owned());
            }
        }
    }
}

fn set_result(
    stream: Stream,
    result: Result<Vec<u8>, String>,
    stdout: &mut Option<Result<Vec<u8>, String>>,
    stderr: &mut Option<Result<Vec<u8>, String>>,
) -> Result<(), String> {
    let slot = match stream {
        Stream::Stdout => stdout,
        Stream::Stderr => stderr,
    };
    if slot.replace(result).is_some() {
        return Err("subprocess produced duplicate output stream result".to_owned());
    }
    Ok(())
}

fn first_error(
    stdout: Option<Result<Vec<u8>, String>>,
    stderr: Option<Result<Vec<u8>, String>>,
) -> Result<Output, String> {
    for result in [stdout, stderr].into_iter().flatten() {
        result?;
    }
    Err("subprocess output capture failed without an error".to_owned())
}

fn join(
    stdout: thread::JoinHandle<()>,
    stderr: thread::JoinHandle<()>,
    label: &str,
) -> Result<(), String> {
    stdout
        .join()
        .map_err(|_| format!("{label} stdout reader panicked"))?;
    stderr
        .join()
        .map_err(|_| format!("{label} stderr reader panicked"))?;
    Ok(())
}

fn terminate(child: &mut Child) {
    kill_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

fn kill_process_group(pid: u32) {
    #[cfg(unix)]
    if let Ok(process_group) = i32::try_from(pid) {
        // SAFETY: the child starts a new process group whose ID is its PID.
        unsafe { libc::kill(-process_group, libc::SIGKILL) };
    }
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
}
