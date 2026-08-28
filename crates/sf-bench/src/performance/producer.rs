use std::fmt;

use super::capture::{capture_observations, FreshProcessRssSampler};
use super::config::{parse_scenarios, MAX_SCENARIO_CONFIG_BYTES};
use super::format::{render_receipt, MAX_RECEIPT_BYTES};
use super::model::{PerformanceReceipt, ReceiptKind, RunnerBinding, SourceTree};
use super::paths::{RepositoryLayout, BASELINE_PATH, CANDIDATE_PATH, PROFILE_PATH, SCENARIOS_PATH};
use super::proc_status::read_self_process_identity;
use super::profile::{parse_profile, LinuxRunnerProbe, MAX_PROFILE_BYTES};
use super::source::inspect_source;
use super::workload_runner::{validate_fixed_m0_scenarios, SqliteGtfsExecutor};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerError(pub String);

impl fmt::Display for ProducerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProducerError {}

#[derive(Debug, Clone)]
pub struct ProducedReceipt {
    pub canonical: String,
    pub output_path: std::path::PathBuf,
}

pub fn produce(
    layout: &RepositoryLayout,
    kind: ReceiptKind,
) -> Result<ProducedReceipt, ProducerError> {
    let output_relative = match kind {
        ReceiptKind::Baseline => BASELINE_PATH,
        ReceiptKind::Candidate => CANDIDATE_PATH,
    };
    layout
        .require_fixed_absent(output_relative)
        .map_err(producer_error)?;
    let manifest = layout
        .read_fixed(SCENARIOS_PATH, MAX_SCENARIO_CONFIG_BYTES)
        .map_err(producer_error)?;
    let scenarios = parse_scenarios(&manifest).map_err(producer_error)?;
    validate_fixed_m0_scenarios(&scenarios).map_err(producer_error)?;
    let profile_bytes = layout
        .read_fixed(PROFILE_PATH, MAX_PROFILE_BYTES)
        .map_err(producer_error)?;
    let profile = parse_profile(&profile_bytes).map_err(producer_error)?;
    profile
        .validate(&LinuxRunnerProbe)
        .map_err(producer_error)?;
    let inspected = inspect_source(layout, &manifest).map_err(producer_error)?;
    validate_capture_source(kind, inspected.binding.tree)?;
    let identity = read_self_process_identity().map_err(producer_error)?;
    let run_token = format!("run-{}-{}", identity.pid, identity.start_time_ticks);
    let run_directory = layout
        .create_run_directory(&run_token)
        .map_err(producer_error)?;
    let captured = (|| {
        let mut executor =
            SqliteGtfsExecutor::new(run_directory.clone()).map_err(producer_error)?;
        let mut rss = FreshProcessRssSampler {
            executable: inspected.executable.clone(),
            repository_root: layout.root().to_owned(),
            run_directory: run_directory.clone(),
        };
        capture_observations(&scenarios, &run_token, &mut executor, &mut rss)
            .map_err(producer_error)
    })();
    let cleanup = layout
        .remove_run_directory(&run_directory)
        .map_err(producer_error);
    let observations = match (captured, cleanup) {
        (Ok(observations), Ok(())) => observations,
        (Err(error), _) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
    };
    revalidate_bindings(layout, &manifest, &profile_bytes, &inspected)?;
    let runner = RunnerBinding::new(
        &profile.profile_id,
        &profile.digest().map_err(producer_error)?,
    )
    .map_err(producer_error)?;
    let receipt = PerformanceReceipt::new(kind, runner, inspected.binding, observations)
        .map_err(producer_error)?;
    let canonical = render_receipt(&receipt).map_err(producer_error)?;
    if canonical.len() > MAX_RECEIPT_BYTES {
        return Err(ProducerError("captured receipt exceeds byte bound".into()));
    }
    let output_path = layout
        .write_new_fixed(output_relative, canonical.as_bytes())
        .map_err(producer_error)?;
    Ok(ProducedReceipt {
        canonical,
        output_path,
    })
}

fn revalidate_bindings(
    layout: &RepositoryLayout,
    manifest: &[u8],
    profile_bytes: &[u8],
    original: &super::source::InspectedSource,
) -> Result<(), ProducerError> {
    let ending_manifest = layout
        .read_fixed(SCENARIOS_PATH, MAX_SCENARIO_CONFIG_BYTES)
        .map_err(producer_error)?;
    let ending_profile = layout
        .read_fixed(PROFILE_PATH, MAX_PROFILE_BYTES)
        .map_err(producer_error)?;
    if ending_manifest != manifest || ending_profile != profile_bytes {
        return Err(ProducerError(
            "scenario manifest or runner profile changed during capture".into(),
        ));
    }
    parse_profile(&ending_profile)
        .map_err(producer_error)?
        .validate(&LinuxRunnerProbe)
        .map_err(producer_error)?;
    let ending_source = inspect_source(layout, &ending_manifest).map_err(producer_error)?;
    if ending_source.binding != original.binding || ending_source.executable != original.executable
    {
        return Err(ProducerError(
            "source or benchmark artifact changed during capture".into(),
        ));
    }
    Ok(())
}

pub fn validate_capture_source(kind: ReceiptKind, tree: SourceTree) -> Result<(), ProducerError> {
    if kind == ReceiptKind::Baseline && tree != SourceTree::Clean {
        Err(ProducerError(
            "baseline capture requires a clean committed source tree".into(),
        ))
    } else {
        Ok(())
    }
}

fn producer_error(error: impl fmt::Display) -> ProducerError {
    ProducerError(error.to_string())
}
