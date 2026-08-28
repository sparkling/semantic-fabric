use std::path::Path;
use std::process::ExitCode;

use sf_bench::performance::compare::{compare, render_comparison};
use sf_bench::performance::config::{parse_scenarios, MAX_SCENARIO_CONFIG_BYTES};
use sf_bench::performance::format::{parse_receipt, CommittedBaseline, MAX_RECEIPT_BYTES};

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("sf-performance-receipt: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<String>) -> Result<String, String> {
    match args.as_slice() {
        [command, path] if command == "check-scenarios" => {
            let bytes = read_bounded(Path::new(path), MAX_SCENARIO_CONFIG_BYTES)?;
            let scenarios = parse_scenarios(&bytes).map_err(|error| error.to_string())?;
            Ok(format!("scenarios\t{}\n", scenarios.len()))
        }
        [command, path] if command == "check-baseline" => {
            let bytes = read_bounded(Path::new(path), MAX_RECEIPT_BYTES)?;
            let baseline = CommittedBaseline::parse(&bytes).map_err(|error| error.to_string())?;
            Ok(format!("baseline-sha256\t{}\n", baseline.sha256()))
        }
        [command, baseline_path, candidate_path] if command == "compare" => {
            let baseline_bytes = read_bounded(Path::new(baseline_path), MAX_RECEIPT_BYTES)?;
            let candidate_bytes = read_bounded(Path::new(candidate_path), MAX_RECEIPT_BYTES)?;
            let baseline =
                CommittedBaseline::parse(&baseline_bytes).map_err(|error| error.to_string())?;
            let candidate = parse_receipt(&candidate_bytes).map_err(|error| error.to_string())?;
            let comparison = compare(&baseline, &candidate).map_err(|error| error.to_string())?;
            render_comparison(&comparison).map_err(|error| error.to_string())
        }
        _ => Err(
            "usage: sf-performance-receipt check-scenarios PATH | check-baseline PATH | compare BASELINE CANDIDATE"
                .into(),
        ),
    }
}

fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, String> {
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("inspect {}: {error}", path.display()))?;
    if metadata.len() > maximum as u64 {
        return Err(format!(
            "{} exceeds the {} byte bound",
            path.display(),
            maximum
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    if bytes.len() > maximum {
        return Err(format!(
            "{} grew beyond the byte bound while reading",
            path.display()
        ));
    }
    Ok(bytes)
}
