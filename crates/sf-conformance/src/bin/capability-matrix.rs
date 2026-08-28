use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use sf_conformance::capability_catalog;
use sf_conformance::capability_render::{self, GeneratedArtifacts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Check,
    Generate,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("capability-matrix: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let Some(mode) = parse_args(env::args().skip(1))? else {
        println!("Usage: capability-matrix (--check | --generate)");
        return Ok(());
    };
    let root = repository_root()?;
    let readme_path = root.join(capability_render::README_PATH);
    let readme = fs::read_to_string(&readme_path)
        .map_err(|error| format!("read {}: {error}", readme_path.display()))?;
    let loaded = capability_catalog::load(&root)?;
    let artifacts = capability_render::render(&loaded, &readme)?;
    match mode {
        Mode::Check => check_artifacts(&root, &artifacts)?,
        Mode::Generate => write_artifacts(&root, &artifacts)?,
    }
    println!(
        "{} capability matrix: {} cells; catalog-sha256={}; schema-sha256={}",
        match mode {
            Mode::Check => "verified",
            Mode::Generate => "generated",
        },
        loaded.catalog.cells.len(),
        loaded.catalog_sha256,
        loaded.schema_sha256
    );
    Ok(())
}

fn parse_args(arguments: impl IntoIterator<Item = String>) -> Result<Option<Mode>, String> {
    let mut mode = None;
    for argument in arguments {
        match argument.as_str() {
            "--check" if mode.is_none() => mode = Some(Mode::Check),
            "--generate" if mode.is_none() => mode = Some(Mode::Generate),
            "--check" | "--generate" => {
                return Err("choose exactly one of --check or --generate".to_owned())
            }
            "--help" | "-h" if mode.is_none() => return Ok(None),
            _ => return Err(format!("unknown argument {argument:?}")),
        }
    }
    mode.map(Some)
        .ok_or_else(|| "choose exactly one of --check or --generate".to_owned())
}

fn repository_root() -> Result<PathBuf, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::canonicalize(&root)
        .map_err(|error| format!("canonicalize repository root {}: {error}", root.display()))
}

fn check_artifacts(root: &Path, artifacts: &GeneratedArtifacts) -> Result<(), String> {
    check_file(
        &root.join(capability_render::GENERATED_JSON_PATH),
        artifacts.json.as_bytes(),
    )?;
    check_file(
        &root.join(capability_render::GENERATED_MARKDOWN_PATH),
        artifacts.markdown.as_bytes(),
    )?;
    check_file(
        &root.join(capability_render::README_PATH),
        artifacts.readme.as_bytes(),
    )
}

fn check_file(path: &Path, expected: &[u8]) -> Result<(), String> {
    let observed = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    if observed == expected {
        Ok(())
    } else {
        Err(format!(
            "{} is stale; run capability-matrix --generate explicitly",
            path.display()
        ))
    }
}

fn write_artifacts(root: &Path, artifacts: &GeneratedArtifacts) -> Result<(), String> {
    atomic_write(
        &root.join(capability_render::GENERATED_JSON_PATH),
        artifacts.json.as_bytes(),
    )?;
    atomic_write(
        &root.join(capability_render::GENERATED_MARKDOWN_PATH),
        artifacts.markdown.as_bytes(),
    )?;
    atomic_write(
        &root.join(capability_render::README_PATH),
        artifacts.readme.as_bytes(),
    )
}

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", target.display()))?;
    if !parent.is_dir() {
        return Err(format!("{} is not a directory", parent.display()));
    }
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{} has no UTF-8 file name", target.display()))?;
    let temporary = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("create {}: {error}", temporary.display()))?;
        file.write_all(bytes)
            .map_err(|error| format!("write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("sync {}: {error}", temporary.display()))?;
        drop(file);
        fs::rename(&temporary, target).map_err(|error| {
            format!(
                "replace {} from {}: {error}",
                target.display(),
                temporary.display()
            )
        })?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("sync directory {}: {error}", parent.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn requires_exactly_one_mode_and_rejects_write_path_options() {
        assert_eq!(
            parse_args(strings(&["--check"])).unwrap(),
            Some(Mode::Check)
        );
        assert_eq!(
            parse_args(strings(&["--generate"])).unwrap(),
            Some(Mode::Generate)
        );
        assert!(parse_args(Vec::<String>::new())
            .unwrap_err()
            .contains("exactly one"));
        assert!(parse_args(strings(&["--check", "--generate"]))
            .unwrap_err()
            .contains("exactly one"));
        assert!(parse_args(strings(&["--output", "elsewhere"]))
            .unwrap_err()
            .contains("unknown argument"));
    }
}
