use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use super::model::{MetricId, ScenarioConfig};
use super::subprocess::{require_success, BoundedCommand};

const WORKER_MAGIC: &str = "sf-performance-worker-v1";
pub const MAX_WORKER_OUTPUT_BYTES: usize = 4_096;
pub const DEFAULT_WORKER_TIMEOUT: Duration = Duration::from_secs(300);

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

#[derive(Debug, Clone)]
pub struct ProcessWorkerLauncher {
    pub executable: PathBuf,
    pub repository_root: PathBuf,
    pub scenario_id: String,
    pub run_token: String,
    pub timeout: Duration,
}

impl WorkerLauncher for ProcessWorkerLauncher {
    type Error = WorkerError;

    fn launch_once(&mut self, request: &WorkerRequest) -> Result<WorkerResult, Self::Error> {
        let output = BoundedCommand {
            program: self.executable.clone(),
            args: vec![
                "worker-rss".into(),
                self.scenario_id.clone(),
                self.run_token.clone(),
                request.sample_index.to_string(),
                request.request_token.clone(),
            ],
            current_dir: self.repository_root.clone(),
            stdin: format!("{}\n", request.request_token).into_bytes(),
            timeout: self.timeout,
            maximum_output: MAX_WORKER_OUTPUT_BYTES,
            observe_linux_identity: true,
        }
        .run()
        .map_err(|error| WorkerError(error.to_string()))?;
        require_success(&output, "RSS worker").map_err(|error| WorkerError(error.to_string()))?;
        let result = parse_worker_result(&output.stdout)?;
        let observed = output
            .identity
            .ok_or_else(|| WorkerError("parent did not observe worker identity".into()))?;
        if result.identity != observed {
            return Err(WorkerError(format!(
                "worker identity mismatch: parent observed {:?}, child echoed {:?}",
                observed, result.identity
            )));
        }
        Ok(result)
    }
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
    if config.metric != MetricId::RssLinuxProcessPeak || config.warmup_count != 0 {
        return Err(WorkerError(
            "fresh-process collector requires a zero-warmup RSS scenario".into(),
        ));
    }
    let mut identities = BTreeSet::new();
    let mut samples = Vec::with_capacity(config.sample_count);
    for sample_index in 0..config.sample_count {
        let request = WorkerRequest {
            sample_index,
            request_token: format!("{run_token}-s{:03}-{sample_index:04}", config.scale),
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

pub fn render_worker_result(result: &WorkerResult) -> Result<String, WorkerError> {
    validate_request_token(&result.request_token)?;
    if result.identity.pid == 0 || result.identity.start_time_ticks == 0 {
        return Err(WorkerError("invalid worker process identity".into()));
    }
    Ok(format!(
        "{WORKER_MAGIC}\nrequest-token\t{}\npid\t{}\nstart-time-ticks\t{}\nvalue\t{}\n",
        result.request_token, result.identity.pid, result.identity.start_time_ticks, result.value
    ))
}

pub fn parse_worker_result(bytes: &[u8]) -> Result<WorkerResult, WorkerError> {
    if bytes.len() > MAX_WORKER_OUTPUT_BYTES {
        return Err(WorkerError("worker output exceeds byte bound".into()));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| WorkerError("worker output is not UTF-8".into()))?;
    if !text.ends_with('\n') || text.contains('\r') {
        return Err(WorkerError("worker output is not canonical LF text".into()));
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() != 5 || lines[0] != WORKER_MAGIC {
        return Err(WorkerError("invalid worker output header".into()));
    }
    let request_token = field(&lines, 1, "request-token")?.to_owned();
    validate_request_token(&request_token)?;
    let pid = number_field(&lines, 2, "pid")?;
    let start_time_ticks = number_field(&lines, 3, "start-time-ticks")?;
    let value = number_field(&lines, 4, "value")?;
    let result = WorkerResult {
        request_token,
        identity: ProcessIdentity {
            pid,
            start_time_ticks,
        },
        value,
    };
    if render_worker_result(&result)? != text {
        return Err(WorkerError("worker output is not canonical".into()));
    }
    Ok(result)
}

pub fn validate_run_token(value: &str) -> Result<(), WorkerError> {
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

fn validate_request_token(value: &str) -> Result<(), WorkerError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(WorkerError("invalid worker request token".into()));
    }
    Ok(())
}

fn field<'a>(lines: &'a [&str], index: usize, key: &str) -> Result<&'a str, WorkerError> {
    let mut parts = lines[index].split('\t');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(actual), Some(value), None) if actual == key => Ok(value),
        _ => Err(WorkerError(format!("invalid worker {key} field"))),
    }
}

fn number_field<T: std::str::FromStr>(
    lines: &[&str],
    index: usize,
    key: &str,
) -> Result<T, WorkerError> {
    field(lines, index, key)?
        .parse()
        .map_err(|_| WorkerError(format!("invalid worker {key}")))
}
