use std::collections::BTreeMap;
use std::convert::Infallible;

use sf_bench::performance::capture::{capture_observations, RssSampler};
use sf_bench::performance::config::parse_scenarios;
use sf_bench::performance::model::{MetricId, ReceiptKind, ScenarioConfig, SourceTree};
use sf_bench::performance::producer::validate_capture_source;
use sf_bench::performance::workload_runner::{
    validate_fixed_m0_scenarios, workload_sha256, SqliteGtfsExecutor, WorkloadExecutor,
};

#[derive(Default)]
struct FakeExecutor {
    calls: BTreeMap<String, usize>,
    begun: Vec<String>,
    finished: usize,
}

impl WorkloadExecutor for FakeExecutor {
    type Error = Infallible;

    fn begin_scenario(&mut self, config: &ScenarioConfig) -> Result<(), Self::Error> {
        self.begun.push(config.id.clone());
        Ok(())
    }

    fn execute_once(&mut self, config: &ScenarioConfig) -> Result<u64, Self::Error> {
        let calls = self.calls.entry(config.id.clone()).or_default();
        *calls += 1;
        Ok(*calls as u64)
    }

    fn finish_scenario(&mut self) -> Result<(), Self::Error> {
        self.finished += 1;
        Ok(())
    }
}

#[derive(Default)]
struct FakeRss {
    calls: Vec<String>,
}

impl RssSampler for FakeRss {
    type Error = Infallible;

    fn collect(
        &mut self,
        config: &ScenarioConfig,
        _run_token: &str,
    ) -> Result<Vec<u64>, Self::Error> {
        self.calls.push(config.id.clone());
        Ok(vec![8_192; config.sample_count])
    }
}

fn fixed_scenarios() -> Vec<ScenarioConfig> {
    parse_scenarios(include_bytes!("../config/performance-scenarios-v1.tsv")).unwrap()
}

#[test]
fn should_execute_exact_warmups_and_samples_without_a_live_capture() {
    let scenarios = fixed_scenarios();
    let mut executor = FakeExecutor::default();
    let mut rss = FakeRss::default();

    let observations =
        capture_observations(&scenarios, "run-1-2", &mut executor, &mut rss).unwrap();

    assert_eq!(observations.len(), 17);
    assert!(observations
        .iter()
        .all(|observation| observation.raw_samples.len() == 50));
    assert_eq!(executor.begun.len(), 14);
    assert_eq!(executor.finished, 14);
    assert_eq!(rss.calls.len(), 3);
    for scenario in scenarios
        .iter()
        .filter(|scenario| scenario.metric != MetricId::RssLinuxProcessPeak)
    {
        assert_eq!(
            executor.calls[&scenario.id],
            usize::from(scenario.warmup_count) + scenario.sample_count
        );
    }
}

#[test]
fn should_bind_manifest_mapping_and_queries_into_one_workload_digest() {
    let manifest = include_bytes!("../config/performance-scenarios-v1.tsv");

    let first = workload_sha256(manifest).unwrap();
    let second = workload_sha256(manifest).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.len(), 64);
}

#[test]
fn should_reject_a_noncanonical_fixed_workload() {
    let mut scenarios = fixed_scenarios();
    scenarios.pop();

    assert!(validate_fixed_m0_scenarios(&scenarios).is_err());
}

#[test]
fn should_require_clean_source_only_for_baseline_generation() {
    assert!(validate_capture_source(ReceiptKind::Baseline, SourceTree::Dirty).is_err());
    assert!(validate_capture_source(ReceiptKind::Baseline, SourceTree::Clean).is_ok());
    assert!(validate_capture_source(ReceiptKind::Candidate, SourceTree::Dirty).is_ok());
}

#[test]
fn should_exclude_source_generation_from_the_typed_rss_worker_path() {
    let directory = tempfile::tempdir().unwrap();
    let scenario = fixed_scenarios()
        .into_iter()
        .find(|scenario| scenario.metric == MetricId::RssLinuxProcessPeak && scenario.scale == 1)
        .unwrap();
    let source = SqliteGtfsExecutor::prepare_rss_source(directory.path(), &scenario).unwrap();
    let mut executor = SqliteGtfsExecutor::new(directory.path().to_owned()).unwrap();
    executor.begin_rss_scenario(&scenario).unwrap();

    assert!(executor.execute_once(&scenario).is_err());
    executor.execute_rss_once(&scenario).unwrap();
    executor.finish_scenario().unwrap();

    assert!(
        source.is_file(),
        "worker teardown must retain the prepared source"
    );
    std::fs::remove_file(source).unwrap();
}
