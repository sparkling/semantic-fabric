use std::fmt;

pub const MAX_PROC_STATUS_BYTES: usize = 1_048_576;

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

#[cfg(target_os = "linux")]
pub fn read_self_vmhwm_bytes() -> Result<u64, ProcStatusError> {
    let status = std::fs::read_to_string("/proc/self/status")
        .map_err(|error| ProcStatusError(format!("read /proc/self/status: {error}")))?;
    parse_vmhwm_bytes(&status)
}

#[cfg(not(target_os = "linux"))]
pub fn read_self_vmhwm_bytes() -> Result<u64, ProcStatusError> {
    Err(ProcStatusError(
        "VmHWM is available only on Linux controlled runners".into(),
    ))
}
