use std::collections::BTreeMap;
use std::env;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use sf_conformance::binary_artifact_receipt::{
    capture, load_external, render, write_new_external, CaptureRequest, Receipt,
};

const USAGE: &str = "Usage:
  current-sf-cli-artifact-observation --capture \
    --repository <ABSOLUTE_PATH> --git <ABSOLUTE_PATH> \
    --bwrap <ABSOLUTE_PATH> --toolchain-root <ABSOLUTE_PATH> \
    --cargo-home <ABSOLUTE_PATH> --readelf <ABSOLUTE_PATH> \
    --scratch-root <ABSOLUTE_PATH> --output <ABSOLUTE_PATH>
  current-sf-cli-artifact-observation --verify \
    --repository <ABSOLUTE_PATH> --input <ABSOLUTE_PATH>
  current-sf-cli-artifact-observation --help

Capture is explicit, creates a new external receipt, and never overwrites it.
Verify performs canonical structural verification only.";

const CAPTURE_ARGUMENTS: [&str; 8] = [
    "--repository",
    "--git",
    "--bwrap",
    "--toolchain-root",
    "--cargo-home",
    "--readelf",
    "--scratch-root",
    "--output",
];
const VERIFY_ARGUMENTS: [&str; 2] = ["--repository", "--input"];

#[derive(Debug, PartialEq, Eq)]
struct CaptureOptions {
    repository: PathBuf,
    git: PathBuf,
    bwrap: PathBuf,
    toolchain_root: PathBuf,
    cargo_home: PathBuf,
    readelf: PathBuf,
    scratch_root: PathBuf,
    output: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
struct VerifyOptions {
    repository: PathBuf,
    input: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
enum Operation {
    Help,
    Capture(CaptureOptions),
    Verify(VerifyOptions),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Capture,
    Verify,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("current-sf-cli-artifact-observation: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    match parse_args(env::args().skip(1))? {
        Operation::Help => println!("{USAGE}"),
        Operation::Capture(options) => capture_receipt(&options)?,
        Operation::Verify(options) => verify_receipt(&options)?,
    }
    Ok(())
}

fn capture_receipt(options: &CaptureOptions) -> Result<(), String> {
    let receipt = capture(&CaptureRequest {
        repository: &options.repository,
        git: &options.git,
        bwrap: &options.bwrap,
        toolchain_root: &options.toolchain_root,
        cargo_home: &options.cargo_home,
        readelf: &options.readelf,
        scratch_root: &options.scratch_root,
    })?;
    write_new_external(&options.repository, &options.output, &receipt)?;
    print_summary("captured", &receipt)
}

fn verify_receipt(options: &VerifyOptions) -> Result<(), String> {
    let receipt = load_external(&options.repository, &options.input)?;
    print_summary("verified", &receipt)
}

fn print_summary(action: &str, receipt: &Receipt) -> Result<(), String> {
    let canonical = render(receipt)?;
    let scope = metadata_value(&canonical, "attestation-scope")
        .ok_or_else(|| "canonical receipt has no attestation scope".to_owned())?;
    let nonclaims: Vec<_> = canonical
        .lines()
        .filter_map(|line| line.strip_prefix("meta\t")?.strip_suffix("\tnot-attested"))
        .collect();
    if nonclaims.is_empty() {
        return Err("canonical receipt has no explicit nonclaims".to_owned());
    }
    println!("{action} current sf-cli artifact observation");
    println!("scope={scope}");
    println!(
        "portable-authority-sha256={}",
        receipt.portable_authority_sha256()
    );
    println!(
        "host-observation-sha256={}",
        receipt.host_observation_sha256()
    );
    println!("receipt-sha256={}", receipt.receipt_sha256());
    println!("nonclaims={}", nonclaims.join(","));
    Ok(())
}

fn metadata_value<'a>(canonical: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("meta\t{key}\t");
    canonical
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
}

fn parse_args(arguments: impl IntoIterator<Item = String>) -> Result<Operation, String> {
    let arguments: Vec<_> = arguments.into_iter().collect();
    if matches!(arguments.as_slice(), [argument] if argument == "--help" || argument == "-h") {
        return Ok(Operation::Help);
    }
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        return Err("--help cannot be combined with a mode or path argument".to_owned());
    }
    let capture_count = arguments
        .iter()
        .filter(|argument| argument.as_str() == "--capture")
        .count();
    let verify_count = arguments
        .iter()
        .filter(|argument| argument.as_str() == "--verify")
        .count();
    let mode = match (capture_count, verify_count) {
        (1, 0) => Mode::Capture,
        (0, 1) => Mode::Verify,
        _ => return Err("choose exactly one of --capture or --verify".to_owned()),
    };
    let allowed = match mode {
        Mode::Capture => CAPTURE_ARGUMENTS.as_slice(),
        Mode::Verify => VERIFY_ARGUMENTS.as_slice(),
    };
    let paths = parse_paths(&arguments, mode, allowed)?;
    match mode {
        Mode::Capture => Ok(Operation::Capture(CaptureOptions {
            repository: required(&paths, "--repository")?,
            git: required(&paths, "--git")?,
            bwrap: required(&paths, "--bwrap")?,
            toolchain_root: required(&paths, "--toolchain-root")?,
            cargo_home: required(&paths, "--cargo-home")?,
            readelf: required(&paths, "--readelf")?,
            scratch_root: required(&paths, "--scratch-root")?,
            output: required(&paths, "--output")?,
        })),
        Mode::Verify => Ok(Operation::Verify(VerifyOptions {
            repository: required(&paths, "--repository")?,
            input: required(&paths, "--input")?,
        })),
    }
}

fn parse_paths(
    arguments: &[String],
    mode: Mode,
    allowed: &[&str],
) -> Result<BTreeMap<String, PathBuf>, String> {
    let mode_flag = match mode {
        Mode::Capture => "--capture",
        Mode::Verify => "--verify",
    };
    let mut paths = BTreeMap::new();
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        if argument == mode_flag {
            index += 1;
            continue;
        }
        if !allowed.contains(&argument.as_str()) {
            return Err(format!("unknown argument {argument:?} for {mode_flag}"));
        }
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| format!("{argument} requires an absolute path value"))?;
        if value.starts_with("--") {
            return Err(format!("{argument} requires an absolute path value"));
        }
        let path = validate_absolute_path(value, argument)?;
        if paths.insert(argument.clone(), path).is_some() {
            return Err(format!("duplicate argument {argument}"));
        }
        index += 2;
    }
    Ok(paths)
}

fn required(paths: &BTreeMap<String, PathBuf>, name: &str) -> Result<PathBuf, String> {
    paths
        .get(name)
        .cloned()
        .ok_or_else(|| format!("missing required argument {name}"))
}

fn validate_absolute_path(value: &str, argument: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if !path.is_absolute()
        || value.contains("//")
        || value.contains('\\')
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(format!(
            "{argument} must be an absolute, normalized Unix path"
        ));
    }
    Ok(path.to_path_buf())
}
