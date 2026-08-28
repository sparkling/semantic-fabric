use std::fmt;
use std::path::PathBuf;

use super::model::{MetricId, ScenarioConfig, ScenarioObservation};
use super::worker::{collect_fresh_samples, ProcessWorkerLauncher, DEFAULT_WORKER_TIMEOUT};
use super::workload_runner::{SqliteGtfsExecutor, WorkloadExecutor};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureError(pub String);

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CaptureError {}

pub trait RssSampler {
    type Error: fmt::Display;

    fn collect(
        &mut self,
        config: &ScenarioConfig,
        run_token: &str,
    ) -> Result<Vec<u64>, Self::Error>;
}

pub struct FreshProcessRssSampler {
    pub executable: PathBuf,
    pub repository_root: PathBuf,
    pub run_directory: PathBuf,
}

impl RssSampler for FreshProcessRssSampler {
    type Error = CaptureError;

    fn collect(
        &mut self,
        config: &ScenarioConfig,
        run_token: &str,
    ) -> Result<Vec<u64>, Self::Error> {
        let source = SqliteGtfsExecutor::prepare_rss_source(&self.run_directory, config)
            .map_err(|error| CaptureError(error.to_string()))?;
        let mut launcher = ProcessWorkerLauncher {
            executable: self.executable.clone(),
            repository_root: self.repository_root.clone(),
            scenario_id: config.id.clone(),
            run_token: run_token.to_owned(),
            timeout: DEFAULT_WORKER_TIMEOUT,
        };
        let captured = collect_fresh_samples(config, run_token, &mut launcher)
            .map_err(|error| CaptureError(error.to_string()));
        let cleanup = std::fs::remove_file(&source).map_err(|error| {
            CaptureError(format!(
                "remove prepared RSS source {}: {error}",
                source.display()
            ))
        });
        match (captured, cleanup) {
            (Ok(samples), Ok(())) => Ok(samples),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }
}

pub fn capture_observations<E, R>(
    scenarios: &[ScenarioConfig],
    run_token: &str,
    executor: &mut E,
    rss: &mut R,
) -> Result<Vec<ScenarioObservation>, CaptureError>
where
    E: WorkloadExecutor,
    R: RssSampler,
{
    let mut observations = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        let samples = if scenario.metric == MetricId::RssLinuxProcessPeak {
            rss.collect(scenario, run_token)
                .map_err(|error| CaptureError(format!("scenario {}: {error}", scenario.id)))?
        } else {
            capture_in_process(scenario, executor)?
        };
        observations.push(
            ScenarioObservation::new(scenario.clone(), samples)
                .map_err(|error| CaptureError(error.to_string()))?,
        );
    }
    Ok(observations)
}

fn capture_in_process<E: WorkloadExecutor>(
    scenario: &ScenarioConfig,
    executor: &mut E,
) -> Result<Vec<u64>, CaptureError> {
    executor
        .begin_scenario(scenario)
        .map_err(|error| CaptureError(format!("prepare {}: {error}", scenario.id)))?;
    let measured = (|| {
        for index in 0..usize::from(scenario.warmup_count) {
            executor.execute_once(scenario).map_err(|error| {
                CaptureError(format!("warmup {index} for {}: {error}", scenario.id))
            })?;
        }
        let mut samples = Vec::with_capacity(scenario.sample_count);
        for index in 0..scenario.sample_count {
            samples.push(executor.execute_once(scenario).map_err(|error| {
                CaptureError(format!("sample {index} for {}: {error}", scenario.id))
            })?);
        }
        Ok(samples)
    })();
    let cleanup = executor
        .finish_scenario()
        .map_err(|error| CaptureError(format!("clean up {}: {error}", scenario.id)));
    match (measured, cleanup) {
        (Ok(samples), Ok(())) => Ok(samples),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}
