use std::collections::BTreeSet;
use std::fmt;

use super::model::ScenarioConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerRequest {
    pub sample_index: usize,
    pub request_token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProcessIdentity {
    pub pid: u32,
    /// Linux `/proc/<pid>/stat` field 22, paired with PID so PID reuse remains valid.
    pub start_time_ticks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerResult {
    pub request_token: String,
    pub identity: ProcessIdentity,
    pub value: u64,
}

/// External-process boundary. Implementations must spawn and reap one process
/// inside each call; the collector verifies the echoed request and identity.
pub trait WorkerLauncher {
    type Error: fmt::Display;

    fn launch_once(&mut self, request: &WorkerRequest) -> Result<WorkerResult, Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerError(pub String);

impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WorkerError {}

pub fn collect_fresh_samples<L: WorkerLauncher>(
    config: &ScenarioConfig,
    run_token: &str,
    launcher: &mut L,
) -> Result<Vec<u64>, WorkerError> {
    validate_run_token(run_token)?;
    let mut identities = BTreeSet::new();
    let mut samples = Vec::with_capacity(config.sample_count);
    for sample_index in 0..config.sample_count {
        let request = WorkerRequest {
            sample_index,
            request_token: format!("{run_token}-{sample_index:04}"),
        };
        let result = launcher
            .launch_once(&request)
            .map_err(|error| WorkerError(format!("worker {sample_index} failed: {error}")))?;
        if result.request_token != request.request_token {
            return Err(WorkerError(format!(
                "worker {sample_index} echoed the wrong request token"
            )));
        }
        if result.identity.pid == 0 || result.identity.start_time_ticks == 0 {
            return Err(WorkerError(format!(
                "worker {sample_index} returned an invalid process identity"
            )));
        }
        if !identities.insert(result.identity) {
            return Err(WorkerError(format!(
                "worker {sample_index} reused process identity {:?}",
                result.identity
            )));
        }
        samples.push(result.value);
    }
    Ok(samples)
}

fn validate_run_token(value: &str) -> Result<(), WorkerError> {
    if value.is_empty()
        || value.len() > 96
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(WorkerError("invalid run token".into()));
    }
    Ok(())
}
