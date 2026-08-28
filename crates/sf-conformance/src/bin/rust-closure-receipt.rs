use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use sf_conformance::rust_closure_receipt::{self, RECEIPT_PATH};

const TEMP_ATTEMPTS: usize = 128;
static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Check,
    Generate,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rust-closure-receipt: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let Some(mode) = parse_args(env::args().skip(1))? else {
        println!("Usage: rust-closure-receipt (--check | --generate)");
        return Ok(());
    };
    let root = repository_root()?;
    let target = root.join(RECEIPT_PATH);
    match mode {
        Mode::Check => {
            let receipt = rust_closure_receipt::check(&root, &target)?;
            println!(
                "verified default sf-cli package closure: {} packages, {} features and {} normal/build edges; lock-sha256={}; closure-sha256={}; artifact-provenance=not-attested; production-admission=not-attested",
                receipt.package_count(),
                receipt.feature_count(),
                receipt.edge_count(),
                receipt.cargo_lock_sha256(),
                receipt.closure_sha256(),
            );
        }
        Mode::Generate => {
            let target = validate_generation_target(&root, &target)?;
            let rendered = rust_closure_receipt::generate(&root)?;
            atomic_replace(&target, rendered.as_bytes())?;
            println!(
                "generated {} (binary/build/link/system provenance and production admission not attested)",
                target.display()
            );
        }
    }
    Ok(())
}

fn parse_args(arguments: impl IntoIterator<Item = String>) -> Result<Option<Mode>, String> {
    let arguments: Vec<_> = arguments.into_iter().collect();
    if matches!(arguments.as_slice(), [argument] if argument == "--help" || argument == "-h") {
        return Ok(None);
    }
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        return Err("--help cannot be combined with --check or --generate".to_owned());
    }
    let mut mode = None;
    for argument in arguments {
        match argument.as_str() {
            "--check" if mode.is_none() => mode = Some(Mode::Check),
            "--generate" if mode.is_none() => mode = Some(Mode::Generate),
            "--check" | "--generate" => {
                return Err("choose exactly one of --check or --generate".to_owned())
            }
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

fn validate_generation_target(root: &Path, target: &Path) -> Result<PathBuf, String> {
    let canonical_root =
        fs::canonicalize(root).map_err(|error| format!("canonicalize repository root: {error}"))?;
    let expected = canonical_root.join(RECEIPT_PATH);
    if target != expected {
        return Err("--generate target is not the canonical receipt path".to_owned());
    }
    let parent = target
        .parent()
        .ok_or_else(|| "receipt target has no parent".to_owned())?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| format!("canonicalize receipt parent {}: {error}", parent.display()))?;
    if canonical_parent != canonical_root.join("tests") {
        return Err("--generate target is not directly inside the tests directory".to_owned());
    }
    validate_atomic_target(target)?;
    Ok(expected)
}

fn atomic_replace(target: &Path, bytes: &[u8]) -> Result<(), String> {
    atomic_replace_with(target, bytes, |_| Ok(()))
}

fn atomic_replace_with<F>(target: &Path, bytes: &[u8], before_rename: F) -> Result<(), String>
where
    F: FnOnce(&Path) -> Result<(), String>,
{
    let parent = target
        .parent()
        .ok_or_else(|| format!("atomic target {} has no parent", target.display()))?;
    let (temporary, mut file) = create_temporary(parent)?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|error| format!("write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("sync {}: {error}", temporary.display()))?;
        drop(file);
        before_rename(&temporary)?;
        validate_atomic_target(target)?;
        fs::rename(&temporary, target).map_err(|error| {
            format!(
                "atomically replace {} from {}: {error}",
                target.display(),
                temporary.display()
            )
        })?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn create_temporary(parent: &Path) -> Result<(PathBuf, File), String> {
    for _ in 0..TEMP_ATTEMPTS {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".rust-dependency-closure.tsv.tmp-{}-{serial}",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o644);
        }
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("create {}: {error}", path.display())),
        }
    }
    Err(format!(
        "could not create an exclusive receipt temporary after {TEMP_ATTEMPTS} attempts"
    ))
}

fn validate_atomic_target(target: &Path) -> Result<(), String> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("receipt target is a symlink".to_owned())
        }
        Ok(metadata) if !metadata.is_file() => Err("receipt target is not a file".to_owned()),
        #[cfg(unix)]
        Ok(metadata)
            if {
                use std::os::unix::fs::MetadataExt;
                metadata.nlink() > 1
            } =>
        {
            Err("receipt target is a hard link".to_owned())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "inspect receipt target {}: {error}",
            target.display()
        )),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync directory {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn requires_exactly_one_fixed_mode() {
        assert_eq!(
            parse_args(strings(&["--check"])).unwrap(),
            Some(Mode::Check)
        );
        assert_eq!(
            parse_args(strings(&["--generate"])).unwrap(),
            Some(Mode::Generate)
        );
        assert!(parse_args(Vec::<String>::new()).is_err());
        assert!(parse_args(strings(&["--check", "--generate"])).is_err());
        assert!(parse_args(strings(&["--output", "elsewhere"])).is_err());
        assert!(parse_args(strings(&["--help", "--check"]))
            .unwrap_err()
            .contains("cannot be combined"));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_hard_link_targets() {
        use std::os::unix::fs::MetadataExt;

        let directory = env::temp_dir().join(format!(
            "semantic-fabric-rust-closure-cli-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let target = directory.join("target");
        let alias = directory.join("alias");
        fs::write(&target, b"old").unwrap();
        fs::hard_link(&target, &alias).unwrap();
        assert!(fs::metadata(&target).unwrap().nlink() > 1);

        let error = atomic_replace(&target, b"new").unwrap_err();

        assert!(error.contains("hard link"), "{error}");
        fs::remove_dir_all(directory).unwrap();
    }
}
