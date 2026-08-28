use std::fmt;
use std::path::Path;

use super::bounded_io;
use super::worker::ProcessIdentity;

pub const MAX_PROC_STATUS_BYTES: usize = 1_048_576;
pub const MAX_PROC_STAT_BYTES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcStatusError(pub String);

impl fmt::Display for ProcStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProcStatusError {}

/// Parse Linux `/proc/<pid>/status`'s process-lifetime resident high-water mark.
///
/// Linux labels the value `kB`; the kernel ABI defines that field in 1024-byte
/// units, so the receipt stores the converted value under unit `bytes`.
pub fn parse_vmhwm_bytes(status: &str) -> Result<u64, ProcStatusError> {
    if status.len() > MAX_PROC_STATUS_BYTES {
        return Err(ProcStatusError("proc status exceeds byte bound".into()));
    }
    let mut value = None;
    for line in status.lines() {
        let Some(body) = line.strip_prefix("VmHWM:") else {
            continue;
        };
        if value.is_some() {
            return Err(ProcStatusError("duplicate VmHWM field".into()));
        }
        let mut fields = body.split_whitespace();
        let kib = fields
            .next()
            .ok_or_else(|| ProcStatusError("missing VmHWM value".into()))?
            .parse::<u64>()
            .map_err(|_| ProcStatusError("invalid VmHWM value".into()))?;
        if fields.next() != Some("kB") || fields.next().is_some() {
            return Err(ProcStatusError("VmHWM must use the Linux kB unit".into()));
        }
        value = Some(
            kib.checked_mul(1024)
                .ok_or_else(|| ProcStatusError("VmHWM byte conversion overflow".into()))?,
        );
    }
    value.ok_or_else(|| ProcStatusError("missing VmHWM field".into()))
}

/// Parse `/proc/<pid>/stat` field 22 without assuming that the parenthesized
/// command name contains no spaces or closing parentheses.
pub fn parse_process_identity(
    stat: &str,
    expected_pid: u32,
) -> Result<ProcessIdentity, ProcStatusError> {
    if stat.len() > MAX_PROC_STAT_BYTES {
        return Err(ProcStatusError("proc stat exceeds byte bound".into()));
    }
    let (pid, rest) = stat
        .split_once(' ')
        .ok_or_else(|| ProcStatusError("invalid proc stat pid field".into()))?;
    if pid.parse::<u32>().ok() != Some(expected_pid) {
        return Err(ProcStatusError("proc stat PID mismatch".into()));
    }
    let close = rest
        .rfind(") ")
        .ok_or_else(|| ProcStatusError("invalid proc stat command field".into()))?;
    let fields: Vec<&str> = rest[close + 2..].split_whitespace().collect();
    let start_time_ticks = fields
        .get(19)
        .ok_or_else(|| ProcStatusError("proc stat is missing field 22".into()))?
        .parse::<u64>()
        .map_err(|_| ProcStatusError("invalid proc stat start time".into()))?;
    if expected_pid == 0 || start_time_ticks == 0 {
        return Err(ProcStatusError("invalid process identity".into()));
    }
    Ok(ProcessIdentity {
        pid: expected_pid,
        start_time_ticks,
    })
}

#[cfg(target_os = "linux")]
pub fn read_process_identity(pid: u32) -> Result<ProcessIdentity, ProcStatusError> {
    let path = format!("/proc/{pid}/stat");
    let bytes = bounded_io::read(Path::new(&path), MAX_PROC_STAT_BYTES)
        .map_err(|error| ProcStatusError(error.to_string()))?;
    let stat =
        String::from_utf8(bytes).map_err(|_| ProcStatusError(format!("{path} is not UTF-8")))?;
    parse_process_identity(&stat, pid)
}

#[cfg(not(target_os = "linux"))]
pub fn read_process_identity(_pid: u32) -> Result<ProcessIdentity, ProcStatusError> {
    Err(ProcStatusError(
        "process identity is available only on Linux controlled runners".into(),
    ))
}

pub fn read_self_process_identity() -> Result<ProcessIdentity, ProcStatusError> {
    read_process_identity(std::process::id())
}

#[cfg(target_os = "linux")]
pub fn read_self_vmhwm_bytes() -> Result<u64, ProcStatusError> {
    let bytes = bounded_io::read(Path::new("/proc/self/status"), MAX_PROC_STATUS_BYTES)
        .map_err(|error| ProcStatusError(error.to_string()))?;
    let status = String::from_utf8(bytes)
        .map_err(|_| ProcStatusError("/proc/self/status is not UTF-8".into()))?;
    parse_vmhwm_bytes(&status)
}

#[cfg(not(target_os = "linux"))]
pub fn read_self_vmhwm_bytes() -> Result<u64, ProcStatusError> {
    Err(ProcStatusError(
        "VmHWM is available only on Linux controlled runners".into(),
    ))
}
