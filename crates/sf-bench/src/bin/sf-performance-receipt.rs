use std::io::Read;
use std::process::ExitCode;

use sf_bench::performance::compare::{compare, render_comparison, ComparisonVerdict};
use sf_bench::performance::config::{parse_scenarios, MAX_SCENARIO_CONFIG_BYTES};
use sf_bench::performance::format::{
    parse_receipt, receipt_sha256, CommittedBaseline, MAX_RECEIPT_BYTES,
};
use sf_bench::performance::model::{MetricId, PerformanceReceipt, ReceiptKind, ScenarioConfig};
use sf_bench::performance::paths::{
    RepositoryLayout, BASELINE_PATH, CANDIDATE_PATH, PROFILE_PATH, SCENARIOS_PATH, WORK_PATH,
};
use sf_bench::performance::proc_status::read_self_process_identity;
use sf_bench::performance::producer::produce;
use sf_bench::performance::profile::{
    parse_profile, render_uncontrolled_template, LinuxRunnerProbe, RunnerProbe, MAX_PROFILE_BYTES,
};
use sf_bench::performance::worker::{render_worker_result, validate_run_token, WorkerResult};
use sf_bench::performance::workload_runner::{
    validate_fixed_m0_scenarios, workload_sha256, SqliteGtfsExecutor, WorkloadExecutor,
};

#[global_allocator]
static GLOBAL: sf_bench::mem::Tracking = sf_bench::mem::Tracking;

struct CliOutput {
    text: String,
    code: u8,
}

struct FixedAuthorities {
    scenarios: Vec<ScenarioConfig>,
    profile_id: String,
    profile_sha256: String,
    workload_sha256: String,
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(output) => {
            print!("{}", output.text);
            ExitCode::from(output.code)
        }
        Err(error) => {
            eprintln!("sf-performance-receipt: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<String>) -> Result<CliOutput, String> {
    let layout = RepositoryLayout::discover().map_err(|error| error.to_string())?;
    match args.as_slice() {
        [command] if command == "probe-profile" => {
            let snapshot = LinuxRunnerProbe
                .probe()
                .map_err(|error| error.to_string())?;
            success(render_uncontrolled_template(&snapshot).map_err(|error| error.to_string())?)
        }
        [command] if command == "check-scenarios" => {
            let bytes = layout
                .read_fixed(SCENARIOS_PATH, MAX_SCENARIO_CONFIG_BYTES)
                .map_err(|error| error.to_string())?;
            let scenarios = parse_scenarios(&bytes).map_err(|error| error.to_string())?;
            validate_fixed_m0_scenarios(&scenarios).map_err(|error| error.to_string())?;
            success(format!("scenarios\t{}\n", scenarios.len()))
        }
        [command] if command == "check-profile" => {
            let bytes = layout
                .read_fixed(PROFILE_PATH, MAX_PROFILE_BYTES)
                .map_err(|error| error.to_string())?;
            let profile = parse_profile(&bytes).map_err(|error| error.to_string())?;
            profile
                .validate(&LinuxRunnerProbe)
                .map_err(|error| error.to_string())?;
            success(format!(
                "runner-profile-sha256\t{}\n",
                profile.digest().map_err(|error| error.to_string())?
            ))
        }
        [command] if command == "check-baseline" => {
            let authorities = load_fixed_authorities(&layout)?;
            let bytes = layout
                .read_fixed(BASELINE_PATH, MAX_RECEIPT_BYTES)
                .map_err(|error| error.to_string())?;
            let baseline = CommittedBaseline::parse(&bytes).map_err(|error| error.to_string())?;
            validate_fixed_receipt(baseline.receipt(), &authorities)?;
            success(format!("baseline-sha256\t{}\n", baseline.sha256()))
        }
        [command] if command == "check-candidate" => {
            let authorities = load_fixed_authorities(&layout)?;
            let bytes = layout
                .read_fixed(CANDIDATE_PATH, MAX_RECEIPT_BYTES)
                .map_err(|error| error.to_string())?;
            let candidate = parse_receipt(&bytes).map_err(|error| error.to_string())?;
            if candidate.kind != ReceiptKind::Candidate {
                return Err("fixed candidate has the wrong receipt kind".into());
            }
            validate_fixed_receipt(&candidate, &authorities)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| "candidate is not UTF-8")?;
            success(format!("candidate-sha256\t{}\n", receipt_sha256(text)))
        }
        [command] if command == "capture-candidate" => capture(&layout, ReceiptKind::Candidate),
        [command] if command == "capture-baseline" => capture(&layout, ReceiptKind::Baseline),
        [command] if command == "compare" => compare_fixed(&layout),
        [command, scenario_id, run_token, sample_index, request_token]
            if command == "worker-rss" =>
        {
            run_rss_worker(
                &layout,
                scenario_id,
                run_token,
                sample_index,
                request_token,
            )
        }
        _ => Err("usage: sf-performance-receipt probe-profile | check-scenarios | check-profile | check-baseline | check-candidate | capture-candidate | capture-baseline | compare".into()),
    }
}

fn capture(layout: &RepositoryLayout, kind: ReceiptKind) -> Result<CliOutput, String> {
    let produced = produce(layout, kind).map_err(|error| error.to_string())?;
    let relative = match kind {
        ReceiptKind::Baseline => BASELINE_PATH,
        ReceiptKind::Candidate => CANDIDATE_PATH,
    };
    success(format!(
        "kind\t{}\noutput\t{}\nreceipt-sha256\t{}\n",
        kind,
        relative,
        receipt_sha256(&produced.canonical)
    ))
}

fn compare_fixed(layout: &RepositoryLayout) -> Result<CliOutput, String> {
    let authorities = load_fixed_authorities(layout)?;
    let baseline_bytes = layout
        .read_fixed(BASELINE_PATH, MAX_RECEIPT_BYTES)
        .map_err(|error| error.to_string())?;
    let candidate_bytes = layout
        .read_fixed(CANDIDATE_PATH, MAX_RECEIPT_BYTES)
        .map_err(|error| error.to_string())?;
    let baseline = CommittedBaseline::parse(&baseline_bytes).map_err(|error| error.to_string())?;
    let candidate = parse_receipt(&candidate_bytes).map_err(|error| error.to_string())?;
    if candidate.kind != ReceiptKind::Candidate {
        return Err("fixed candidate has the wrong receipt kind".into());
    }
    validate_fixed_receipt(baseline.receipt(), &authorities)?;
    validate_fixed_receipt(&candidate, &authorities)?;
    let comparison = compare(&baseline, &candidate).map_err(|error| error.to_string())?;
    let code = if comparison.verdict == ComparisonVerdict::Pass {
        0
    } else {
        1
    };
    Ok(CliOutput {
        text: render_comparison(&comparison).map_err(|error| error.to_string())?,
        code,
    })
}

fn load_fixed_authorities(layout: &RepositoryLayout) -> Result<FixedAuthorities, String> {
    let manifest = layout
        .read_fixed(SCENARIOS_PATH, MAX_SCENARIO_CONFIG_BYTES)
        .map_err(|error| error.to_string())?;
    let scenarios = parse_scenarios(&manifest).map_err(|error| error.to_string())?;
    validate_fixed_m0_scenarios(&scenarios).map_err(|error| error.to_string())?;
    let profile_bytes = layout
        .read_fixed(PROFILE_PATH, MAX_PROFILE_BYTES)
        .map_err(|error| error.to_string())?;
    let profile = parse_profile(&profile_bytes).map_err(|error| error.to_string())?;
    Ok(FixedAuthorities {
        scenarios,
        profile_id: profile.profile_id.clone(),
        profile_sha256: profile.digest().map_err(|error| error.to_string())?,
        workload_sha256: workload_sha256(&manifest).map_err(|error| error.to_string())?,
    })
}

fn validate_fixed_receipt(
    receipt: &PerformanceReceipt,
    authorities: &FixedAuthorities,
) -> Result<(), String> {
    if receipt.runner.profile_id != authorities.profile_id
        || receipt.runner.profile_sha256 != authorities.profile_sha256
    {
        return Err("receipt runner binding does not match the fixed profile authority".into());
    }
    if receipt.source.workload_sha256 != authorities.workload_sha256 {
        return Err("receipt workload binding does not match the current fixed workload".into());
    }
    if receipt.observations.len() != authorities.scenarios.len()
        || receipt
            .observations
            .iter()
            .zip(&authorities.scenarios)
            .any(|(observation, scenario)| observation.config != *scenario)
    {
        return Err("receipt scenarios do not match the current fixed M0 manifest".into());
    }
    Ok(())
}

fn run_rss_worker(
    layout: &RepositoryLayout,
    scenario_id: &str,
    run_token: &str,
    sample_index: &str,
    request_token: &str,
) -> Result<CliOutput, String> {
    let sample_index = sample_index
        .parse::<usize>()
        .map_err(|_| "invalid worker sample index")?;
    validate_run_token(run_token).map_err(|error| error.to_string())?;
    let mut gate = Vec::new();
    std::io::stdin()
        .take(130)
        .read_to_end(&mut gate)
        .map_err(|error| format!("read worker gate: {error}"))?;
    if gate != format!("{request_token}\n").as_bytes() {
        return Err("worker gate token mismatch or overflow".into());
    }
    let manifest = layout
        .read_fixed(SCENARIOS_PATH, MAX_SCENARIO_CONFIG_BYTES)
        .map_err(|error| error.to_string())?;
    let scenarios = parse_scenarios(&manifest).map_err(|error| error.to_string())?;
    validate_fixed_m0_scenarios(&scenarios).map_err(|error| error.to_string())?;
    let scenario = scenarios
        .iter()
        .find(|scenario| scenario.id == scenario_id)
        .ok_or_else(|| "worker scenario is not in the fixed manifest".to_owned())?;
    if scenario.metric != MetricId::RssLinuxProcessPeak || sample_index >= scenario.sample_count {
        return Err("worker invocation is not a valid RSS sample".into());
    }
    let expected_token = format!("{run_token}-s{:03}-{sample_index:04}", scenario.scale);
    if request_token != expected_token {
        return Err("worker request token does not match run, scenario, and sample".into());
    }
    let run_directory = layout
        .fixed_path(&format!("{WORK_PATH}/{run_token}"))
        .map_err(|error| error.to_string())?;
    let identity = read_self_process_identity().map_err(|error| error.to_string())?;
    let mut executor = SqliteGtfsExecutor::new(run_directory).map_err(|error| error.to_string())?;
    executor
        .begin_rss_scenario(scenario)
        .map_err(|error| error.to_string())?;
    let measured = executor
        .execute_rss_once(scenario)
        .map_err(|error| error.to_string());
    let cleanup = executor
        .finish_scenario()
        .map_err(|error| error.to_string());
    match (measured, cleanup) {
        (Ok(_), Ok(())) => {}
        (Err(error), _) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
    }
    let value = sf_bench::performance::proc_status::read_self_vmhwm_bytes()
        .map_err(|error| error.to_string())?;
    success(
        render_worker_result(&WorkerResult {
            request_token: request_token.to_owned(),
            identity,
            value,
        })
        .map_err(|error| error.to_string())?,
    )
}

fn success(text: String) -> Result<CliOutput, String> {
    Ok(CliOutput { text, code: 0 })
}
