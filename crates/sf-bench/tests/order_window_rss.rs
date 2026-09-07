//! Fresh-process ADR-0054 RSS qualification for a fixed root ORDER window.

use std::io::Read;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use sf_bench::performance::model::ScenarioObservation;
use sf_bench::performance::order_window_rss::{
    execute_once, prepare_source, qualify_growth, remove_source, scenario_by_id, scenarios,
    source_rows,
};
use sf_bench::performance::paths::{RepositoryLayout, WORK_PATH};
use sf_bench::performance::proc_status::{read_self_process_identity, read_self_vmhwm_bytes};
use sf_bench::performance::worker::{
    collect_fresh_samples, render_worker_result, validate_run_token, ProcessWorkerLauncher,
    WorkerResult, DEFAULT_WORKER_TIMEOUT,
};

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("order-window-rss: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<String>) -> Result<String, String> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|error| format!("resolve repository root: {error}"))?;
    let layout = RepositoryLayout::new(root).map_err(|error| error.to_string())?;
    match args.as_slice() {
        [] => run_qualification(&layout),
        [command, scenario_id, run_token, sample_index, request_token]
            if command == "worker-rss" =>
        {
            run_worker(&layout, scenario_id, run_token, sample_index, request_token)
        }
        _ => Err("usage: order_window_rss [worker-rss SCENARIO RUN SAMPLE REQUEST]".into()),
    }
}

fn run_qualification(layout: &RepositoryLayout) -> Result<String, String> {
    if !cfg!(target_os = "linux") {
        return Err("fresh-process VmHWM qualification requires Linux".into());
    }
    let executable = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|error| format!("resolve qualification executable: {error}"))?;
    layout
        .validate_contained_file(&executable)
        .map_err(|error| error.to_string())?;
    let run_token = run_token()?;
    let run_directory = layout
        .create_run_directory(&run_token)
        .map_err(|error| error.to_string())?;
    let scenarios = scenarios().map_err(|error| error.to_string())?;
    let mut prepared = Vec::new();
    let measured = (|| {
        for scenario in &scenarios {
            prepare_source(&run_directory, scenario).map_err(|error| error.to_string())?;
            prepared.push(scenario.clone());
        }

        let mut observations = Vec::with_capacity(scenarios.len());
        let mut report = String::new();
        for scenario in &scenarios {
            let mut launcher = ProcessWorkerLauncher {
                executable: executable.clone(),
                repository_root: layout.root().to_owned(),
                scenario_id: scenario.id.clone(),
                run_token: run_token.clone(),
                timeout: DEFAULT_WORKER_TIMEOUT,
            };
            let samples = collect_fresh_samples(scenario, &run_token, &mut launcher)
                .map_err(|error| error.to_string())?;
            let observation = ScenarioObservation::new(scenario.clone(), samples)
                .map_err(|error| error.to_string())?;
            report.push_str(&format!(
                "bounded-order-rss\tscale\t{}\tsource-rows\t{}\tsamples\t{}\tmedian-bytes\t{}\tp95-bytes\t{}\n",
                scenario.scale,
                source_rows(scenario).map_err(|error| error.to_string())?,
                observation.raw_samples.len(),
                observation.summary.median,
                observation.summary.p95
            ));
            observations.push(observation);
        }
        let growth = qualify_growth(&observations).map_err(|error| error.to_string())?;
        report.push_str(&format!(
            "bounded-order-rss-gate\t10x-median-x2\t{}\t100x-median-x2\t{}\t10x-p95\t{}\t100x-p95\t{}\tverdict\tpass\n",
            growth.median_10x_x2,
            growth.median_100x_x2,
            growth.p95_10x,
            growth.p95_100x
        ));
        Ok::<String, String>(report)
    })();
    let cleanup = cleanup(layout, &run_directory, &prepared);
    match (measured, cleanup) {
        (Ok(report), Ok(())) => Ok(report),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn run_worker(
    layout: &RepositoryLayout,
    scenario_id: &str,
    run_token: &str,
    sample_index: &str,
    request_token: &str,
) -> Result<String, String> {
    let sample_index = sample_index
        .parse::<usize>()
        .map_err(|_| "invalid worker sample index")?;
    validate_run_token(run_token).map_err(|error| error.to_string())?;
    let scenario = scenario_by_id(scenario_id).map_err(|error| error.to_string())?;
    if sample_index >= scenario.sample_count {
        return Err("worker sample index exceeds the fixed sample count".into());
    }
    let expected_token = format!("{run_token}-s{:03}-{sample_index:04}", scenario.scale);
    if request_token != expected_token {
        return Err("worker request token does not match run, scenario, and sample".into());
    }
    let mut gate = Vec::new();
    std::io::stdin()
        .take(130)
        .read_to_end(&mut gate)
        .map_err(|error| format!("read worker gate: {error}"))?;
    if gate != format!("{request_token}\n").as_bytes() {
        return Err("worker gate token mismatch or overflow".into());
    }
    let run_directory = layout
        .fixed_path(&format!("{WORK_PATH}/{run_token}"))
        .map_err(|error| error.to_string())?;
    let identity = read_self_process_identity().map_err(|error| error.to_string())?;
    execute_once(&run_directory, &scenario).map_err(|error| error.to_string())?;
    let value = read_self_vmhwm_bytes().map_err(|error| error.to_string())?;
    render_worker_result(&WorkerResult {
        request_token: request_token.to_owned(),
        identity,
        value,
    })
    .map_err(|error| error.to_string())
}

fn cleanup(
    layout: &RepositoryLayout,
    run_directory: &std::path::Path,
    prepared: &[sf_bench::performance::model::ScenarioConfig],
) -> Result<(), String> {
    for scenario in prepared.iter().rev() {
        remove_source(run_directory, scenario).map_err(|error| error.to_string())?;
    }
    layout
        .remove_run_directory(run_directory)
        .map_err(|error| error.to_string())
}

fn run_token() -> Result<String, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("read system clock: {error}"))?
        .as_nanos();
    Ok(format!("order-window-rss-{}-{nanos}", std::process::id()))
}
