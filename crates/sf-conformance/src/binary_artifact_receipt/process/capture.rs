//! Cancellable bounded stdout/stderr capture for subprocesses.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::Output;

const READ_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(super) type ResultMessage = (Stream, Result<Vec<u8>, String>);

#[derive(Clone, Copy)]
pub(super) enum Stream {
    Stdout,
    Stderr,
}

#[cfg(unix)]
pub(super) fn read_limited<R: Read + std::os::fd::AsRawFd + Send + 'static>(
    mut reader: R,
    limit: u64,
    label: String,
    stream: &'static str,
    sender: mpsc::SyncSender<ResultMessage>,
    cancel: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, String> {
    let spawn_label = format!("{label} {stream}");
    thread::Builder::new()
        .name(format!("sf-capture-{stream}"))
        .spawn(move || {
            let result = read_nonblocking(&mut reader, limit, &label, stream, &cancel);
            let kind = if stream == "stdout" {
                Stream::Stdout
            } else {
                Stream::Stderr
            };
            let _ = sender.send((kind, result));
        })
        .map_err(|error| format!("spawn {spawn_label} reader: {error}"))
}

#[cfg(unix)]
fn read_nonblocking<R: Read + std::os::fd::AsRawFd>(
    reader: &mut R,
    limit: u64,
    label: &str,
    stream: &str,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    let descriptor = reader.as_raw_fd();
    // SAFETY: fcntl reads and updates flags on the live pipe descriptor.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0
    {
        return Err(format!(
            "configure {label} {stream}: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(format!("{label} {stream} capture cancelled"));
        }
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(bytes),
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.len() as u64 > limit {
                    return Err(format!("{label} {stream} exceeds {limit} bytes"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(READ_POLL_INTERVAL);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(format!("read {label} {stream}: {error}")),
        }
    }
}

#[cfg(not(unix))]
pub(super) fn read_limited<R: Read + Send + 'static>(
    reader: R,
    limit: u64,
    label: String,
    stream: &'static str,
    sender: mpsc::SyncSender<ResultMessage>,
    _cancel: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, String> {
    let spawn_label = format!("{label} {stream}");
    thread::Builder::new()
        .name(format!("sf-capture-{stream}"))
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = reader
                .take(limit.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|error| format!("read {label} {stream}: {error}"))
                .and_then(|_| {
                    (bytes.len() as u64 <= limit)
                        .then_some(bytes)
                        .ok_or_else(|| format!("{label} {stream} exceeds {limit} bytes"))
                });
            let kind = if stream == "stdout" {
                Stream::Stdout
            } else {
                Stream::Stderr
            };
            let _ = sender.send((kind, result));
        })
        .map_err(|error| format!("spawn {spawn_label} reader: {error}"))
}

pub(super) fn collect(
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

pub(super) fn set_result(
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

pub(super) fn first_error(
    stdout: Option<Result<Vec<u8>, String>>,
    stderr: Option<Result<Vec<u8>, String>>,
) -> Result<Output, String> {
    for result in [stdout, stderr].into_iter().flatten() {
        result?;
    }
    Err("subprocess output capture failed without an error".to_owned())
}

pub(super) fn has_error(
    stdout: &Option<Result<Vec<u8>, String>>,
    stderr: &Option<Result<Vec<u8>, String>>,
) -> bool {
    stdout.as_ref().is_some_and(Result::is_err) || stderr.as_ref().is_some_and(Result::is_err)
}

pub(super) fn join(
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
