use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::proc_status::read_process_identity;
use super::worker::ProcessIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubprocessError(pub String);

impl fmt::Display for SubprocessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SubprocessError {}

#[derive(Debug, Clone)]
pub struct BoundedCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub current_dir: PathBuf,
    pub stdin: Vec<u8>,
    pub timeout: Duration,
    pub maximum_output: usize,
    pub observe_linux_identity: bool,
}

#[derive(Debug)]
pub struct BoundedOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub identity: Option<ProcessIdentity>,
}

impl BoundedCommand {
    /// Spawn with an empty environment: model/provider credentials and routing
    /// variables are never inherited by benchmark workers or git inspection.
    pub fn run(&self) -> Result<BoundedOutput, SubprocessError> {
        validate_spec(self)?;
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .current_dir(&self.current_dir)
            .env_clear()
            .stdin(if self.stdin.is_empty() {
                Stdio::null()
            } else {
                Stdio::piped()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|error| {
            SubprocessError(format!("spawn {}: {error}", self.program.display()))
        })?;
        let identity = if self.observe_linux_identity {
            match read_process_identity(child.id()) {
                Ok(identity) => Some(identity),
                Err(error) => {
                    reap(&mut child);
                    return Err(SubprocessError(format!(
                        "identify child {}: {error}",
                        child.id()
                    )));
                }
            }
        } else {
            None
        };
        let overflow = Arc::new(AtomicBool::new(false));
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SubprocessError("child stdout pipe is missing".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| SubprocessError("child stderr pipe is missing".into()))?;
        let stdout_reader = drain(stdout, self.maximum_output, Arc::clone(&overflow));
        let stderr_reader = drain(stderr, self.maximum_output, Arc::clone(&overflow));

        if !self.stdin.is_empty() {
            let write_result = child
                .stdin
                .take()
                .ok_or_else(|| SubprocessError("child stdin pipe is missing".into()))
                .and_then(|mut input| {
                    input
                        .write_all(&self.stdin)
                        .map_err(|error| SubprocessError(format!("write child stdin: {error}")))
                });
            if let Err(error) = write_result {
                reap(&mut child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(error);
            }
        }

        let started = Instant::now();
        let status = loop {
            if overflow.load(Ordering::Relaxed) {
                reap(&mut child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(SubprocessError(format!(
                    "{} exceeded the {} byte output bound",
                    self.program.display(),
                    self.maximum_output
                )));
            }
            if started.elapsed() >= self.timeout {
                reap(&mut child);
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(SubprocessError(format!(
                    "{} exceeded the {} ms timeout",
                    self.program.display(),
                    self.timeout.as_millis()
                )));
            }
            match exited_status(&mut child) {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(error) => {
                    reap(&mut child);
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return Err(error);
                }
            }
        };
        let stdout = join_reader(stdout_reader)?;
        let stderr = join_reader(stderr_reader)?;
        if overflow.load(Ordering::Relaxed) {
            return Err(SubprocessError(format!(
                "{} exceeded the {} byte output bound",
                self.program.display(),
                self.maximum_output
            )));
        }
        Ok(BoundedOutput {
            status,
            stdout,
            stderr,
            identity,
        })
    }
}

fn validate_spec(spec: &BoundedCommand) -> Result<(), SubprocessError> {
    if !spec.program.is_absolute() || !spec.current_dir.is_absolute() {
        return Err(SubprocessError(
            "subprocess program and current directory must be absolute".into(),
        ));
    }
    let metadata = std::fs::symlink_metadata(&spec.program)
        .map_err(|error| SubprocessError(format!("inspect {}: {error}", spec.program.display())))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SubprocessError(
            "subprocess program must be a non-symlink regular file".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(SubprocessError(
                "subprocess program must not be a hard link".into(),
            ));
        }
    }
    let directory = std::fs::symlink_metadata(&spec.current_dir).map_err(|error| {
        SubprocessError(format!(
            "inspect current directory {}: {error}",
            spec.current_dir.display()
        ))
    })?;
    if directory.file_type().is_symlink()
        || !directory.is_dir()
        || spec.current_dir.canonicalize().ok().as_ref() != Some(&spec.current_dir)
    {
        return Err(SubprocessError(
            "subprocess current directory must be canonical and non-symlink".into(),
        ));
    }
    if spec.timeout.is_zero()
        || spec.timeout > Duration::from_secs(3_600)
        || spec.maximum_output == 0
        || spec.maximum_output > 1_048_576
        || spec.stdin.len() > 4_096
    {
        return Err(SubprocessError("invalid subprocess resource bound".into()));
    }
    Ok(())
}

fn drain<R: Read + Send + 'static>(
    mut reader: R,
    maximum: usize,
    overflow: Arc<AtomicBool>,
) -> std::thread::JoinHandle<std::io::Result<Vec<u8>>> {
    std::thread::spawn(move || {
        let mut stored = Vec::new();
        let mut buffer = [0u8; 8_192];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            if stored.len().saturating_add(count) > maximum {
                overflow.store(true, Ordering::Relaxed);
            } else {
                stored.extend_from_slice(&buffer[..count]);
            }
        }
        Ok(stored)
    })
}

fn join_reader(
    reader: std::thread::JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<Vec<u8>, SubprocessError> {
    reader
        .join()
        .map_err(|_| SubprocessError("subprocess output reader panicked".into()))?
        .map_err(|error| SubprocessError(format!("read subprocess output: {error}")))
}

fn reap(child: &mut std::process::Child) {
    terminate_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
fn exited_status(child: &mut std::process::Child) -> Result<Option<ExitStatus>, SubprocessError> {
    let pid =
        i32::try_from(child.id()).map_err(|_| SubprocessError("child PID exceeds i32".into()))?;
    let mut information = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
    // SAFETY: waitid initializes siginfo_t for this direct child. WNOWAIT keeps
    // an exited group leader unreaped until descendants have been swept, so its
    // process-group ID cannot be recycled in between observation and kill.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            information.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        return Err(SubprocessError(format!(
            "inspect child without reaping: {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: successful waitid initialized the value above.
    let information = unsafe { information.assume_init() };
    // SAFETY: si_pid is valid for the SIGCHLD payload returned by waitid.
    if unsafe { information.si_pid() } == 0 {
        return Ok(None);
    }
    terminate_process_group(child.id());
    child
        .wait()
        .map(Some)
        .map_err(|error| SubprocessError(format!("reap exited child: {error}")))
}

#[cfg(not(target_os = "linux"))]
fn exited_status(child: &mut std::process::Child) -> Result<Option<ExitStatus>, SubprocessError> {
    let status = child
        .try_wait()
        .map_err(|error| SubprocessError(format!("wait for child: {error}")))?;
    if status.is_some() {
        terminate_process_group(child.id());
    }
    Ok(status)
}

fn terminate_process_group(pid: u32) {
    #[cfg(unix)]
    if let Ok(process_group) = i32::try_from(pid) {
        // SAFETY: the child was placed in a fresh process group whose ID is its
        // PID; a negative PID addresses that group only.
        unsafe {
            libc::kill(-process_group, libc::SIGKILL);
        }
    }
}

pub fn require_success(output: &BoundedOutput, label: &str) -> Result<(), SubprocessError> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(SubprocessError(format!(
        "{label} exited with {}: {}",
        output.status,
        stderr.trim()
    )))
}

pub fn canonical_program(path: &Path) -> Result<PathBuf, SubprocessError> {
    path.canonicalize()
        .map_err(|error| SubprocessError(format!("resolve {}: {error}", path.display())))
}
