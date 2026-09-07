use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use sf_conformance::rust_closure_receipt::{
    self, PARSER_WORKER_QUALIFICATION_INPUTS_PROFILE,
    PARSER_WORKER_QUALIFICATION_INPUTS_RECEIPT_PATH, RECEIPT_PATH,
};

const TEMP_ATTEMPTS: usize = 128;
static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Check,
    Generate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Profile {
    DefaultCliV1,
    ParserWorkerQualificationInputsV1,
}

impl Profile {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "default-cli-v1" => Ok(Self::DefaultCliV1),
            PARSER_WORKER_QUALIFICATION_INPUTS_PROFILE => {
                Ok(Self::ParserWorkerQualificationInputsV1)
            }
            _ => Err(format!("unknown closure profile {value:?}")),
        }
    }

    fn receipt_path(self) -> &'static str {
        match self {
            Self::DefaultCliV1 => RECEIPT_PATH,
            Self::ParserWorkerQualificationInputsV1 => {
                PARSER_WORKER_QUALIFICATION_INPUTS_RECEIPT_PATH
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Options {
    mode: Mode,
    profile: Profile,
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
    let Some(options) = parse_args(env::args().skip(1))? else {
        println!(
            "Usage: rust-closure-receipt (--check | --generate) [--profile default-cli-v1|parser-worker-qualification-inputs-v1]"
        );
        return Ok(());
    };
    let root = repository_root()?;
    let target = root.join(options.profile.receipt_path());
    match (options.mode, options.profile) {
        (Mode::Check, Profile::DefaultCliV1) => {
            report_default(rust_closure_receipt::check(&root, &target)?);
        }
        (Mode::Check, Profile::ParserWorkerQualificationInputsV1) => {
            let receipt =
                rust_closure_receipt::check_parser_worker_qualification_inputs(&root, &target)?;
            println!(
                "verified parser-worker qualification inputs: {} packages, {} target/host contexts and {} context edges; closure-sha256={}; qualification-inputs-sha256={}; parser-execution=not-run; qualification=not-attested; production-admission=not-attested",
                receipt.package_count(),
                receipt.context_count(),
                receipt.edge_count(),
                receipt.closure_sha256(),
                receipt.qualification_inputs_sha256(),
            );
        }
        (Mode::Generate, profile) => {
            let target = validate_generation_target(&root, &target, profile.receipt_path())?;
            let rendered = match profile {
                Profile::DefaultCliV1 => rust_closure_receipt::generate(&root)?,
                Profile::ParserWorkerQualificationInputsV1 => {
                    rust_closure_receipt::generate_parser_worker_qualification_inputs(&root)?
                }
            };
            atomic_replace(&target, rendered.as_bytes())?;
            match profile {
                Profile::DefaultCliV1 => println!(
                    "generated {} (binary/build/link/system provenance and production admission not attested)",
                    target.display()
                ),
                Profile::ParserWorkerQualificationInputsV1 => println!(
                    "generated {} (parser execution, binary/runtime/syscall provenance, qualification and production admission not attested)",
                    target.display()
                ),
            }
        }
    }
    Ok(())
}

fn report_default(receipt: rust_closure_receipt::Receipt) {
    println!(
        "verified default sf-cli package closure: {} packages, {} features and {} normal/build edges; lock-sha256={}; closure-sha256={}; artifact-provenance=not-attested; production-admission=not-attested",
        receipt.package_count(),
        receipt.feature_count(),
        receipt.edge_count(),
        receipt.cargo_lock_sha256(),
        receipt.closure_sha256(),
    );
}

fn parse_args(arguments: impl IntoIterator<Item = String>) -> Result<Option<Options>, String> {
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
    let mut profile = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--check" if mode.is_none() => mode = Some(Mode::Check),
            "--generate" if mode.is_none() => mode = Some(Mode::Generate),
            "--check" | "--generate" => {
                return Err("choose exactly one of --check or --generate".to_owned())
            }
            "--profile" if profile.is_none() => {
                index += 1;
                let value = arguments
                    .get(index)
                    .ok_or_else(|| "--profile requires one value".to_owned())?;
                profile = Some(Profile::parse(value)?);
            }
            "--profile" => return Err("--profile may be supplied only once".to_owned()),
            argument => return Err(format!("unknown argument {argument:?}")),
        }
        index += 1;
    }
    Ok(Some(Options {
        mode: mode.ok_or_else(|| "choose exactly one of --check or --generate".to_owned())?,
        profile: profile.unwrap_or(Profile::DefaultCliV1),
    }))
}

fn repository_root() -> Result<PathBuf, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::canonicalize(&root)
        .map_err(|error| format!("canonicalize repository root {}: {error}", root.display()))
}

fn validate_generation_target(
    root: &Path,
    target: &Path,
    receipt_path: &str,
) -> Result<PathBuf, String> {
    let canonical_root =
        fs::canonicalize(root).map_err(|error| format!("canonicalize repository root: {error}"))?;
    let expected = canonical_root.join(receipt_path);
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
    let target_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "atomic target file name is not UTF-8".to_owned())?;
    let (temporary, mut file) = create_temporary(parent, target_name)?;
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

fn create_temporary(parent: &Path, target_name: &str) -> Result<(PathBuf, File), String> {
    for _ in 0..TEMP_ATTEMPTS {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{target_name}.tmp-{}-{serial}",
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
            Some(Options {
                mode: Mode::Check,
                profile: Profile::DefaultCliV1,
            })
        );
        assert_eq!(
            parse_args(strings(&["--generate"])).unwrap(),
            Some(Options {
                mode: Mode::Generate,
                profile: Profile::DefaultCliV1,
            })
        );
        assert_eq!(
            parse_args(strings(&[
                "--profile",
                PARSER_WORKER_QUALIFICATION_INPUTS_PROFILE,
                "--check",
            ]))
            .unwrap(),
            Some(Options {
                mode: Mode::Check,
                profile: Profile::ParserWorkerQualificationInputsV1,
            })
        );
        assert!(parse_args(Vec::<String>::new()).is_err());
        assert!(parse_args(strings(&["--check", "--generate"])).is_err());
        assert!(parse_args(strings(&["--output", "elsewhere"])).is_err());
        assert!(parse_args(strings(&["--check", "--profile"])).is_err());
        assert!(parse_args(strings(&["--check", "--profile", "unknown"])).is_err());
        assert!(parse_args(strings(&[
            "--check",
            "--profile",
            "default-cli-v1",
            "--profile",
            "default-cli-v1",
        ]))
        .is_err());
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
