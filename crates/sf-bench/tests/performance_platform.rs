use std::convert::Infallible;

use sf_bench::performance::model::{BoundaryId, MetricId, ScenarioConfig, Unit, M0_SAMPLE_COUNT};
use sf_bench::performance::proc_status::parse_vmhwm_bytes;
use sf_bench::performance::worker::{
    collect_fresh_samples, ProcessIdentity, WorkerLauncher, WorkerRequest, WorkerResult,
};

#[test]
fn should_parse_linux_vmhwm_kibibytes_as_bytes() {
    let status = "Name:\tworker\nVmPeak:\t99999 kB\nVmHWM:\t12345 kB\nVmRSS:\t12000 kB\n";

    let bytes = parse_vmhwm_bytes(status).expect("valid Linux proc status");

    assert_eq!(bytes, 12_641_280);
}

#[test]
fn should_reject_duplicate_vmhwm_fields() {
    let status = "VmHWM:\t10 kB\nVmHWM:\t11 kB\n";

    let result = parse_vmhwm_bytes(status);

    assert!(result.is_err());
}

#[test]
fn should_reject_vmhwm_with_non_linux_unit() {
    let status = "VmHWM:\t10 MB\n";

    let result = parse_vmhwm_bytes(status);

    assert!(result.is_err());
}

struct RecordingLauncher {
    requests: Vec<WorkerRequest>,
    reuse_identity: bool,
}

impl WorkerLauncher for RecordingLauncher {
    type Error = Infallible;

    fn launch_once(&mut self, request: &WorkerRequest) -> Result<WorkerResult, Self::Error> {
        self.requests.push(request.clone());
        let sequence = if self.reuse_identity {
            0
        } else {
            request.sample_index as u64
        };
        Ok(WorkerResult {
            request_token: request.request_token.clone(),
            identity: ProcessIdentity {
                pid: 1_000 + sequence as u32,
                start_time_ticks: 50_000 + sequence,
            },
            value: sequence,
        })
    }
}

fn rss_config() -> ScenarioConfig {
    ScenarioConfig::new(
        "gtfs.sqlite.construct.rss.scale1",
        1,
        MetricId::RssLinuxProcessPeak,
        BoundaryId::LinuxFreshProcessLifetime,
        Unit::Bytes,
        0,
        M0_SAMPLE_COUNT,
    )
    .expect("valid RSS scenario")
}

#[test]
fn should_launch_one_fresh_worker_for_every_raw_sample() {
    let mut launcher = RecordingLauncher {
        requests: Vec::new(),
        reuse_identity: false,
    };

    let samples =
        collect_fresh_samples(&rss_config(), "run-0001", &mut launcher).expect("fresh workers");

    assert_eq!(samples.len(), M0_SAMPLE_COUNT);
    assert_eq!(launcher.requests.len(), M0_SAMPLE_COUNT);
}

#[test]
fn should_reject_reused_process_identity_between_samples() {
    let mut launcher = RecordingLauncher {
        requests: Vec::new(),
        reuse_identity: true,
    };

    let result = collect_fresh_samples(&rss_config(), "run-0001", &mut launcher);

    assert!(result.is_err());
}
