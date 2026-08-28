use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

pub(super) fn output(
    mut command: Command,
    label: &str,
    max_bytes: u64,
    timeout: Duration,
) -> Result<String, String> {
    let label = label.to_owned();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn {label}: {error}"))?;
    let Some(stdout) = child.stdout.take() else {
        terminate(&mut child);
        return Err(format!("{label} stdout is unavailable"));
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader_label = label.clone();
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(max_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("read {reader_label}: {error}"))
            .and_then(|_| {
                if bytes.len() as u64 > max_bytes {
                    Err(format!("{reader_label} exceeds {max_bytes} bytes"))
                } else {
                    Ok(bytes)
                }
            });
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + timeout;
    let mut captured = None;
    let status = loop {
        if captured.is_none() {
            match receiver.try_recv() {
                Ok(result) => captured = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    terminate(&mut child);
                    let _ = reader.join();
                    return Err(format!("{label} output reader stopped unexpectedly"));
                }
            }
        }
        if matches!(captured, Some(Err(_))) {
            terminate(&mut child);
            let _ = reader.join();
            return captured
                .expect("captured result was checked")
                .map(|_| String::new());
        }
        let status = match exited_status(&mut child) {
            Ok(status) => status,
            Err(error) => {
                terminate(&mut child);
                let _ = reader.join();
                return Err(format!("wait for {label}: {error}"));
            }
        };
        if let Some(status) = status {
            break status;
        }
        if Instant::now() >= deadline {
            terminate(&mut child);
            let _ = reader.join();
            return Err(format!(
                "{label} timed out after {} seconds",
                timeout.as_secs_f64()
            ));
        }
        thread::sleep(POLL_INTERVAL);
    };
    if captured.is_none() {
        match receiver.recv_timeout(DRAIN_TIMEOUT) {
            Ok(result) => captured = Some(result),
            Err(_) => {
                terminate(&mut child);
                let _ = reader.join();
                return Err(format!("{label} stdout did not close after process exit"));
            }
        }
    }
    reader
        .join()
        .map_err(|_| format!("{label} output reader panicked"))?;
    if !status.success() {
        return Err(format!("{label} failed with {status}"));
    }
    let bytes = captured.expect("captured output was populated")?;
    String::from_utf8(bytes).map_err(|error| format!("{label} is not UTF-8: {error}"))
}

fn terminate(child: &mut Child) {
    kill_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

fn kill_process_group(pid: u32) {
    #[cfg(unix)]
    if let Ok(process_group) = i32::try_from(pid) {
        // SAFETY: the child was placed in a new process group whose id equals
        // its pid; a negative PID addresses only that group.
        unsafe {
            libc::kill(-process_group, libc::SIGKILL);
        }
    }
}

#[cfg(target_os = "linux")]
fn exited_status(child: &mut Child) -> Result<Option<std::process::ExitStatus>, std::io::Error> {
    let pid =
        i32::try_from(child.id()).map_err(|_| std::io::Error::other("child PID exceeds i32"))?;
    let mut information = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    // SAFETY: waitid initializes siginfo_t for this direct child. WNOWAIT keeps
    // the exited leader unreaped while terminate() sweeps its process group.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            information.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: successful waitid initialized the value above.
    let information = unsafe { information.assume_init() };
    // SAFETY: si_pid is valid for the SIGCHLD payload returned by waitid.
    if unsafe { information.si_pid() } == 0 {
        return Ok(None);
    }
    kill_process_group(child.id());
    child.wait().map(Some)
}

#[cfg(not(target_os = "linux"))]
fn exited_status(child: &mut Child) -> Result<Option<std::process::ExitStatus>, std::io::Error> {
    let status = child.try_wait()?;
    if status.is_some() {
        terminate(child);
    }
    Ok(status)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn bounds_stdout() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf 123456789"]);
        let error = output(command, "fixture", 4, Duration::from_secs(1)).unwrap_err();
        assert!(error.contains("exceeds"), "{error}");
    }

    #[test]
    fn times_out_and_reaps_a_process_group() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5"]);
        let started = Instant::now();
        let error = output(command, "fixture", 16, Duration::from_millis(50)).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
