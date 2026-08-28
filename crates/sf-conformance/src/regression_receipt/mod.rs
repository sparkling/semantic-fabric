mod atomic;
mod environment;
mod format;
mod model;
mod path;
mod runner;

#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::fs::{File, Metadata};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use model::{BaselineRow, Disposition, ExpectedBaseline, Profile, Surface};
use runner::{ProcessRunner, SuiteRunner};

const MAX_INPUT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_INPUT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
struct W3cInventorySpec {
    suite_relative: &'static str,
    inventory_relative: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct ProfileSpec {
    profile_id: &'static str,
    surface: Surface,
    inventory_relative: &'static str,
    baseline_relative: &'static str,
    baseline_name: &'static str,
    w3c: Option<W3cInventorySpec>,
}

pub const QUERY_SPEC: ProfileSpec = ProfileSpec {
    profile_id: "sqlite-query-regression-v2",
    surface: Surface::SparqlQuery,
    inventory_relative: "tests/sparql/query/inventory.tsv",
    baseline_relative: "tests/sparql/query/sqlite-expected-regression-baseline.tsv",
    baseline_name: "sqlite-expected-regression-baseline.tsv",
    w3c: Some(W3cInventorySpec {
        suite_relative: "tests/w3c/rdb2rdf",
        inventory_relative: "tests/w3c/rdb2rdf/inventory.tsv",
    }),
};

pub const PROTOCOL_SPEC: ProfileSpec = ProfileSpec {
    profile_id: "sqlite-protocol-regression-v2",
    surface: Surface::SparqlProtocol,
    inventory_relative: "tests/sparql/protocol/inventory.tsv",
    baseline_relative: "tests/sparql/protocol/sqlite-expected-regression-baseline.tsv",
    baseline_name: "sqlite-expected-regression-baseline.tsv",
    w3c: None,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Check,
    Generate,
}

pub fn main_for(spec: ProfileSpec) -> ExitCode {
    match parse_mode(std::env::args_os().skip(1)).and_then(|mode| {
        let Some(mode) = mode else {
            println!("Usage: {} (--check | --generate)", spec.profile_id);
            return Ok(None);
        };
        run(spec, mode, &ProcessRunner).map(Some)
    }) {
        Ok(Some(baseline)) => {
            println!(
                "verified expected {} SQLite regression baseline: suites={}; required-tests={}; excluded={}; inventory-sha256={}; outcomes-sha256={}; runtime-provenance=not-attested; W3C-query/protocol-conformance=not-attested; backend-admission=not-attested",
                baseline.surface.name(),
                baseline.suite_count,
                baseline.rows.len(),
                baseline.excluded_count,
                baseline.inventory_sha256,
                baseline.outcomes_sha256
            );
            ExitCode::SUCCESS
        }
        Ok(None) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}: {error}", spec.profile_id);
            ExitCode::FAILURE
        }
    }
}

fn parse_mode(arguments: impl IntoIterator<Item = OsString>) -> Result<Option<Mode>, String> {
    let mut mode = None;
    let mut help = false;
    for argument in arguments {
        match argument.to_str() {
            Some("--check") => set_mode(&mut mode, Mode::Check)?,
            Some("--generate") => set_mode(&mut mode, Mode::Generate)?,
            Some("--help" | "-h") if !help => help = true,
            Some("--help" | "-h") => return Err("help flag may be supplied only once".to_owned()),
            Some(other) => return Err(format!("unknown argument {other:?}")),
            None => return Err("arguments must be UTF-8".to_owned()),
        }
    }
    if help && mode.is_some() {
        return Err("--help cannot be combined with --check or --generate".to_owned());
    }
    if help {
        Ok(None)
    } else {
        mode.map(Some)
            .ok_or_else(|| "choose exactly one of --check or --generate".to_owned())
    }
}

fn set_mode(target: &mut Option<Mode>, candidate: Mode) -> Result<(), String> {
    if target.replace(candidate).is_some() {
        Err("choose exactly one of --check or --generate".to_owned())
    } else {
        Ok(())
    }
}

fn run<R: SuiteRunner>(
    spec: ProfileSpec,
    mode: Mode,
    runner: &R,
) -> Result<ExpectedBaseline, String> {
    run_at_root(spec, mode, runner, &repository_root()?)
}

fn run_at_root<R: SuiteRunner>(
    spec: ProfileSpec,
    mode: Mode,
    runner: &R,
    root: &Path,
) -> Result<ExpectedBaseline, String> {
    let root = path::canonical_root(root)?;
    let (inventory_bytes, profile) = load_profile(&root, spec)?;
    let inventory_sha256 = format::sha256(&inventory_bytes);
    validate_input_bindings(&root, &profile)?;
    validate_w3c_inventory(&root, spec)?;
    match mode {
        Mode::Generate => {
            let rows = execute(&profile, &root, runner)?;
            validate_unchanged(&root, spec, &inventory_bytes, &profile)?;
            let baseline = build_baseline(spec, &profile, &inventory_sha256, rows);
            let rendered = format::render_baseline(&baseline);
            atomic::replace_fixed(
                &root,
                spec.baseline_relative,
                spec.baseline_name,
                rendered.as_bytes(),
            )?;
            let written = read_fixed(&root, spec.baseline_relative, format::MAX_BASELINE_BYTES)?;
            if written != rendered.as_bytes() {
                return Err("generated expected baseline read-back mismatch".to_owned());
            }
            Ok(baseline)
        }
        Mode::Check => {
            let baseline_bytes =
                read_fixed(&root, spec.baseline_relative, format::MAX_BASELINE_BYTES)?;
            let expected_text = std::str::from_utf8(&baseline_bytes)
                .map_err(|error| format!("expected baseline is not UTF-8: {error}"))?;
            let expected = format::parse_baseline(expected_text)?;
            if format::render_baseline(&expected) != expected_text {
                return Err("expected regression baseline is valid but not canonical".to_owned());
            }
            let shape = build_baseline(spec, &profile, &inventory_sha256, required_rows(&profile));
            if expected != shape {
                return Err("expected baseline does not match the bound profile".to_owned());
            }
            let observed_rows = execute(&profile, &root, runner)?;
            validate_unchanged(&root, spec, &inventory_bytes, &profile)?;
            let current_baseline =
                read_fixed(&root, spec.baseline_relative, format::MAX_BASELINE_BYTES)?;
            if current_baseline != baseline_bytes {
                return Err("expected baseline changed during read-only replay".to_owned());
            }
            let observed = build_baseline(spec, &profile, &inventory_sha256, observed_rows);
            if observed != expected {
                return Err("parsed per-test outcomes differ from expected baseline".to_owned());
            }
            Ok(expected)
        }
    }
}

fn execute<R: SuiteRunner>(
    profile: &Profile,
    root: &Path,
    runner: &R,
) -> Result<Vec<BaselineRow>, String> {
    runner::execute_profile(profile, root, runner).map_err(|failure| failure.message)
}

fn build_baseline(
    spec: ProfileSpec,
    profile: &Profile,
    inventory_sha256: &str,
    rows: Vec<BaselineRow>,
) -> ExpectedBaseline {
    ExpectedBaseline {
        profile_id: profile.profile_id.clone(),
        surface: profile.surface,
        backend: profile.backend.clone(),
        inventory_path: spec.inventory_relative.to_owned(),
        inventory_sha256: inventory_sha256.to_owned(),
        outcomes_sha256: format::outcomes_digest(&rows),
        suite_count: profile.suites.len(),
        excluded_count: profile
            .tests
            .iter()
            .filter(|test| test.disposition == Disposition::Exclude)
            .count(),
        rows,
    }
}

fn required_rows(profile: &Profile) -> Vec<BaselineRow> {
    profile
        .tests
        .iter()
        .filter(|test| test.disposition == Disposition::Include)
        .map(|test| BaselineRow {
            suite_id: test.suite_id.clone(),
            test_name: test.name.clone(),
        })
        .collect()
}

fn load_profile(root: &Path, spec: ProfileSpec) -> Result<(Vec<u8>, Profile), String> {
    let bytes = read_fixed(root, spec.inventory_relative, format::MAX_INVENTORY_BYTES)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| format!("profile inventory is not UTF-8: {error}"))?;
    let profile = format::parse_profile(text)?;
    if format::render_profile(&profile) != text {
        return Err("profile inventory is valid but not canonical".to_owned());
    }
    if profile.profile_id != spec.profile_id
        || profile.surface != spec.surface
        || profile.backend != "sqlite"
    {
        return Err("profile metadata does not match its fixed baseline binary".to_owned());
    }
    Ok((bytes, profile))
}

fn validate_input_bindings(root: &Path, profile: &Profile) -> Result<(), String> {
    let mut total = 0_u64;
    for input in &profile.inputs {
        let bytes = read_fixed(root, &input.path, MAX_INPUT_BYTES)?;
        total = total
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| "bound input size overflow".to_owned())?;
        if total > MAX_TOTAL_INPUT_BYTES {
            return Err(format!("bound inputs exceed {MAX_TOTAL_INPUT_BYTES} bytes"));
        }
        if bytes.len() as u64 != input.byte_length || format::sha256(&bytes) != input.sha256 {
            return Err(format!(
                "bound input digest/length mismatch for {}",
                input.path
            ));
        }
    }
    Ok(())
}

fn validate_unchanged(
    root: &Path,
    spec: ProfileSpec,
    inventory_before: &[u8],
    profile: &Profile,
) -> Result<(), String> {
    if read_fixed(root, spec.inventory_relative, format::MAX_INVENTORY_BYTES)? != inventory_before {
        return Err("profile inventory changed during execution".to_owned());
    }
    validate_input_bindings(root, profile)
        .map_err(|error| format!("bound input changed during execution: {error}"))?;
    validate_w3c_inventory(root, spec)
        .map_err(|error| format!("W3C inventory changed during execution: {error}"))
}

fn validate_w3c_inventory(root: &Path, spec: ProfileSpec) -> Result<(), String> {
    let Some(w3c) = spec.w3c else {
        return Ok(());
    };
    let suite = path::existing_directory(root, w3c.suite_relative)?;
    let inventory_path = path::existing_file(root, w3c.inventory_relative)?;
    crate::inventory::check(&suite, &inventory_path)
        .map(|_| ())
        .map_err(|error| format!("W3C RDB2RDF case-tree inventory: {error}"))
}

fn read_fixed(root: &Path, relative: &str, limit: u64) -> Result<Vec<u8>, String> {
    let path = path::existing_file(root, relative)?;
    read_authority(&path, limit)
}

fn read_authority(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let path_metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("inspect {}: {error}", path.display()))?;
    validate_authority_metadata(path, &path_metadata)?;
    if path_metadata.len() > limit {
        return Err(format!(
            "authority {} exceeds {limit} bytes",
            path.display()
        ));
    }
    let mut file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("inspect opened authority {}: {error}", path.display()))?;
    validate_authority_metadata(path, &opened_metadata)?;
    if !same_authority(&path_metadata, &opened_metadata) {
        return Err(format!(
            "authority {} changed while opening",
            path.display()
        ));
    }
    validate_authority_identity(path, &opened_metadata)?;
    let mut bytes = Vec::with_capacity(path_metadata.len() as usize);
    file.by_ref()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    if bytes.len() as u64 > limit {
        return Err(format!(
            "authority {} grew beyond {limit} bytes",
            path.display()
        ));
    }
    validate_authority_identity(path, &opened_metadata)?;
    Ok(bytes)
}

fn validate_authority_metadata(path: &Path, metadata: &Metadata) -> Result<(), String> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "authority {} is not a regular non-symlink file",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(format!("authority {} is a hard link", path.display()));
        }
    }
    Ok(())
}

fn validate_authority_identity(path: &Path, opened: &Metadata) -> Result<(), String> {
    let current = std::fs::symlink_metadata(path)
        .map_err(|error| format!("reinspect authority {}: {error}", path.display()))?;
    validate_authority_metadata(path, &current)?;
    if !same_authority(&current, opened) {
        return Err(format!("authority {} changed during read", path.display()));
    }
    Ok(())
}

#[cfg(unix)]
fn same_authority(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_authority(left: &Metadata, right: &Metadata) -> bool {
    left.len() == right.len()
}

fn repository_root() -> Result<PathBuf, String> {
    path::canonical_root(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}
