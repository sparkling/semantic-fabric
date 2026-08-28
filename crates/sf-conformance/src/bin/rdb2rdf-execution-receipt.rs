use std::env;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use sf_conformance::execution_receipt;

const RECEIPT_NAME: &str = "sqlite-execution-receipt.tsv";
const TEMP_ATTEMPTS: usize = 128;
static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Check,
    Generate,
}

#[derive(Debug)]
struct Options {
    mode: Mode,
    suite: PathBuf,
    receipt: PathBuf,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rdb2rdf-execution-receipt: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let Some(options) = parse_args(env::args().skip(1))? else {
        println!(
            "Usage: rdb2rdf-execution-receipt (--check | --generate) \
             [--suite PATH] [--receipt PATH]"
        );
        return Ok(());
    };
    match options.mode {
        Mode::Check => {
            let receipt = execution_receipt::check(&options.suite, &options.receipt)?;
            println!(
                "verified {} SQLite outcome-baseline records; inventory-sha256={}; \
                 outcomes-sha256={}; runner-provenance=not-attested",
                receipt.cases().len(),
                receipt.inventory_sha256(),
                receipt.outcomes_sha256()
            );
        }
        Mode::Generate => {
            let target = validate_generation_target(&options.suite, &options.receipt)?;
            let generated = execution_receipt::generate(&options.suite)?;
            atomic_replace(&target, generated.as_bytes())?;
            println!(
                "generated SQLite outcome baseline {} (runner provenance not attested)",
                target.display()
            );
        }
    }
    Ok(())
}

fn parse_args(arguments: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
    let suite_default = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf");
    let mut suite = suite_default;
    let mut receipt = None;
    let mut mode = None;
    let mut args = arguments.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--check" => set_mode(&mut mode, Mode::Check)?,
            "--generate" => set_mode(&mut mode, Mode::Generate)?,
            "--suite" => {
                suite = PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--suite requires a path".to_owned())?,
                );
            }
            "--receipt" => {
                receipt = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--receipt requires a path".to_owned())?,
                ));
            }
            "--help" | "-h" => return Ok(None),
            _ => return Err(format!("unknown argument {argument:?}")),
        }
    }
    let mode = mode.ok_or_else(|| "choose exactly one of --check or --generate".to_owned())?;
    let receipt = receipt.unwrap_or_else(|| suite.join(RECEIPT_NAME));
    Ok(Some(Options {
        mode,
        suite,
        receipt,
    }))
}

fn set_mode(target: &mut Option<Mode>, candidate: Mode) -> Result<(), String> {
    if target.is_some() {
        return Err("choose exactly one of --check or --generate".to_owned());
    }
    *target = Some(candidate);
    Ok(())
}

/// Restrict the only write path to a non-symlink file with the canonical name
/// directly inside the canonical suite root.
fn validate_generation_target(suite: &Path, receipt: &Path) -> Result<PathBuf, String> {
    let canonical_suite = fs::canonicalize(suite)
        .map_err(|error| format!("canonicalize suite {}: {error}", suite.display()))?;
    if !canonical_suite.is_dir() {
        return Err(format!("suite {} is not a directory", suite.display()));
    }
    if receipt.file_name() != Some(OsStr::new(RECEIPT_NAME)) {
        return Err(format!(
            "--generate target must be named {RECEIPT_NAME} inside the suite root"
        ));
    }
    let parent = receipt
        .parent()
        .ok_or_else(|| "--generate target has no parent directory".to_owned())?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| format!("canonicalize receipt parent {}: {error}", parent.display()))?;
    if canonical_parent != canonical_suite {
        return Err("--generate target must be directly inside the suite root".to_owned());
    }
    match fs::symlink_metadata(receipt) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("--generate refuses a symlink receipt target".to_owned())
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err("--generate receipt target exists but is not a file".to_owned())
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "inspect receipt target {}: {error}",
                receipt.display()
            ))
        }
    }
    Ok(canonical_suite.join(RECEIPT_NAME))
}

/// Durably replace the canonical receipt without truncating an existing inode.
/// The final rename replaces a raced link entry rather than following it; the
/// last metadata check also rejects links installed before the rename boundary.
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
            ".{RECEIPT_NAME}.tmp-{}-{serial}",
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
            Err("atomic receipt target changed to a symlink".to_owned())
        }
        Ok(metadata) if !metadata.is_file() => {
            Err("atomic receipt target changed to a non-file".to_owned())
        }
        #[cfg(unix)]
        Ok(metadata)
            if {
                use std::os::unix::fs::MetadataExt;
                metadata.nlink() > 1
            } =>
        {
            Err("atomic receipt target changed to a hard link".to_owned())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "inspect atomic receipt target {}: {error}",
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

    static NEXT_TEST_TEMP: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            loop {
                let serial = NEXT_TEST_TEMP.fetch_add(1, Ordering::Relaxed);
                let path = env::temp_dir().join(format!(
                    "semantic-fabric-receipt-cli-{}-{serial}",
                    std::process::id()
                ));
                let mut builder = fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                match builder.create(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create temporary directory: {error}"),
                }
            }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove temporary directory");
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn rejects_conflicting_modes() {
        let error = parse_args(strings(&["--check", "--generate"])).expect_err("conflict fails");
        assert!(error.contains("exactly one"), "{error}");
    }

    #[test]
    fn rejects_missing_option_values_and_unknown_arguments() {
        assert!(parse_args(strings(&["--suite"]))
            .unwrap_err()
            .contains("--suite requires"));
        assert!(parse_args(strings(&["--receipt"]))
            .unwrap_err()
            .contains("--receipt requires"));
        assert!(parse_args(strings(&["--unknown"]))
            .unwrap_err()
            .contains("unknown argument"));
    }

    #[test]
    fn accepts_only_the_canonical_generation_target() {
        let root = TempDir::new();
        let suite = root.0.join("suite");
        let outside = root.0.join("outside");
        fs::create_dir(&suite).expect("create suite");
        fs::create_dir(&outside).expect("create outside directory");
        let intended = suite.join(RECEIPT_NAME);
        assert_eq!(
            validate_generation_target(&suite, &intended).expect("intended target"),
            fs::canonicalize(&suite).unwrap().join(RECEIPT_NAME)
        );
        let error = validate_generation_target(&suite, &outside.join(RECEIPT_NAME)).unwrap_err();
        assert!(error.contains("directly inside"), "{error}");
        let error = validate_generation_target(&suite, &suite.join("other.tsv")).unwrap_err();
        assert!(error.contains("must be named"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_generation_target() {
        use std::os::unix::fs::symlink;

        let root = TempDir::new();
        let suite = root.0.join("suite");
        fs::create_dir(&suite).expect("create suite");
        let destination = root.0.join("destination.tsv");
        fs::write(&destination, b"do not overwrite\n").expect("write destination");
        let receipt = suite.join(RECEIPT_NAME);
        symlink(&destination, &receipt).expect("create receipt symlink");
        let error = validate_generation_target(&suite, &receipt).unwrap_err();
        assert!(error.contains("refuses a symlink"), "{error}");
        assert_eq!(
            fs::read(destination).expect("read destination"),
            b"do not overwrite\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_replace_rejects_a_raced_symlink_without_touching_its_victim() {
        use std::os::unix::fs::symlink;

        let root = TempDir::new();
        let target = root.0.join(RECEIPT_NAME);
        let victim = root.0.join("victim.tsv");
        fs::write(&victim, b"victim stays\n").unwrap();
        let error = atomic_replace_with(&target, b"new receipt\n", |_| {
            symlink(&victim, &target).map_err(|error| error.to_string())
        })
        .unwrap_err();

        assert!(error.contains("changed to a symlink"), "{error}");
        assert_eq!(fs::read(&victim).unwrap(), b"victim stays\n");
        assert!(fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_no_temporary(&root.0);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_replace_rejects_a_raced_hard_link_without_touching_its_victim() {
        let root = TempDir::new();
        let target = root.0.join(RECEIPT_NAME);
        let victim = root.0.join("victim.tsv");
        fs::write(&victim, b"victim stays\n").unwrap();
        let error = atomic_replace_with(&target, b"new receipt\n", |_| {
            fs::hard_link(&victim, &target).map_err(|error| error.to_string())
        })
        .unwrap_err();

        assert!(error.contains("changed to a hard link"), "{error}");
        assert_eq!(fs::read(&victim).unwrap(), b"victim stays\n");
        assert_eq!(fs::read(&target).unwrap(), b"victim stays\n");
        assert_no_temporary(&root.0);
    }

    #[test]
    fn atomic_replace_failure_preserves_the_previous_receipt() {
        let root = TempDir::new();
        let target = root.0.join(RECEIPT_NAME);
        fs::write(&target, b"previous receipt\n").unwrap();
        let error = atomic_replace_with(&target, b"new receipt\n", |_| {
            Err("injected pre-rename failure".to_owned())
        })
        .unwrap_err();

        assert_eq!(error, "injected pre-rename failure");
        assert_eq!(fs::read(&target).unwrap(), b"previous receipt\n");
        assert_no_temporary(&root.0);
    }

    #[test]
    fn atomic_replace_success_is_exact_and_leaves_no_temporary() {
        let root = TempDir::new();
        let target = root.0.join(RECEIPT_NAME);
        fs::write(&target, b"previous receipt\n").unwrap();

        atomic_replace(&target, b"new receipt\n").expect("atomic replacement");

        assert_eq!(fs::read(&target).unwrap(), b"new receipt\n");
        assert_no_temporary(&root.0);
    }

    fn assert_no_temporary(directory: &Path) {
        let names: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(
            names.iter().all(|name| !name
                .to_string_lossy()
                .starts_with(&format!(".{RECEIPT_NAME}.tmp-"))),
            "temporary receipt leaked: {names:?}"
        );
    }
}
