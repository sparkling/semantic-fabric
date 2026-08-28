//! Controlled verification of the tracked closure receipt for artifact capture.

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use super::{metadata, platform, process, Receipt};

const MAX_LOCK_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOOLCHAIN_BYTES: u64 = 1024 * 1024;
const MAX_EXECUTABLE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Exact filesystem and process authority for a controlled closure check.
#[derive(Debug, Clone, Copy)]
pub struct ControlledCheckRequest<'a> {
    pub materialized_source: &'a Path,
    pub cargo: &'a Path,
    pub rustc: &'a Path,
    pub toolchain_root: &'a Path,
    pub cargo_home: &'a Path,
    pub temporary_dir: &'a Path,
    pub source_date_epoch: u64,
}

/// Verifies the tracked receipt using only explicitly bound tools and state.
pub fn check_with_tools(request: &ControlledCheckRequest<'_>) -> Result<Receipt, String> {
    let context = Context::new(request)?;
    let receipt_path = context.source.join(super::RECEIPT_PATH);
    let (expected, receipt_bytes) = super::load(&receipt_path)?;
    let inputs = Inputs::read(&context.source)?;
    inputs.bind(&expected)?;
    let tools_before = context.tool_fingerprints()?;
    let observed = capture(&context, &inputs)?;
    inputs.ensure_unchanged(&context.source)?;
    super::authority::ensure_unchanged(
        &receipt_path,
        &receipt_bytes,
        super::format::MAX_RECEIPT_BYTES,
    )?;
    if context.tool_fingerprints()? != tools_before {
        return Err("controlled Cargo or rustc changed during closure verification".to_owned());
    }
    if expected != observed {
        return Err(format!(
            "controlled Rust closure drift: recorded lock={} closure={}, actual lock={} closure={}",
            expected.cargo_lock_sha256,
            expected.closure_sha256,
            observed.cargo_lock_sha256,
            observed.closure_sha256,
        ));
    }
    Ok(expected)
}

struct Context {
    source: PathBuf,
    cargo: PathBuf,
    rustc: PathBuf,
    toolchain: PathBuf,
    cargo_home: PathBuf,
    temporary: PathBuf,
    path: OsString,
    source_date_epoch: u64,
}

impl Context {
    fn new(request: &ControlledCheckRequest<'_>) -> Result<Self, String> {
        if request.source_date_epoch == 0 || request.source_date_epoch > i64::MAX as u64 {
            return Err("SOURCE_DATE_EPOCH is outside supported Unix timestamp bounds".to_owned());
        }
        let source = canonical_directory(request.materialized_source, "materialized source")?;
        let toolchain = canonical_directory(request.toolchain_root, "toolchain root")?;
        let cargo_home = canonical_directory(request.cargo_home, "controlled Cargo home")?;
        let temporary = canonical_directory(request.temporary_dir, "controlled temporary dir")?;
        for (index, left) in [&source, &toolchain, &cargo_home, &temporary]
            .iter()
            .enumerate()
        {
            for right in [&source, &toolchain, &cargo_home, &temporary]
                .iter()
                .skip(index + 1)
            {
                if left.starts_with(right) || right.starts_with(left) {
                    return Err("controlled closure roots overlap".to_owned());
                }
            }
        }
        let cargo = canonical_executable(request.cargo, "Cargo executable")?;
        let rustc = canonical_executable(request.rustc, "rustc executable")?;
        if cargo != toolchain.join("bin/cargo") || rustc != toolchain.join("bin/rustc") {
            return Err("Cargo and rustc must be the canonical toolchain-root binaries".to_owned());
        }
        reject_active_cargo_configs(&source, &cargo_home)?;
        let path = env::join_paths([toolchain.join("bin")])
            .map_err(|error| format!("construct controlled PATH: {error}"))?;
        Ok(Self {
            source,
            cargo,
            rustc,
            toolchain,
            cargo_home,
            temporary,
            path,
            source_date_epoch: request.source_date_epoch,
        })
    }

    fn environment(&self) -> BTreeMap<OsString, OsString> {
        [
            ("CARGO_HOME", self.cargo_home.as_os_str().to_owned()),
            ("CARGO_INCREMENTAL", "0".into()),
            ("CARGO_NET_OFFLINE", "true".into()),
            ("HOME", self.cargo_home.as_os_str().to_owned()),
            ("LC_ALL", "C".into()),
            ("PATH", self.path.clone()),
            ("RUSTC", self.rustc.as_os_str().to_owned()),
            ("RUSTUP_HOME", self.toolchain.as_os_str().to_owned()),
            (
                "SOURCE_DATE_EPOCH",
                self.source_date_epoch.to_string().into(),
            ),
            ("TMPDIR", self.temporary.as_os_str().to_owned()),
            ("TZ", "UTC".into()),
        ]
        .into_iter()
        .map(|(name, value)| (name.into(), value))
        .collect()
    }

    fn command(&self, program: &Path, arguments: Vec<OsString>) -> CommandSpec {
        CommandSpec {
            program: program.to_path_buf(),
            current_dir: self.source.clone(),
            arguments,
            environment: self.environment(),
            clear_environment: true,
        }
    }

    fn cargo_command(&self, arguments: &[&str]) -> CommandSpec {
        self.command(&self.cargo, os_strings(arguments))
    }

    fn rustc_command(&self, arguments: &[&str]) -> CommandSpec {
        self.command(&self.rustc, os_strings(arguments))
    }

    fn metadata_command(&self) -> CommandSpec {
        let mut arguments = os_strings(&[
            "metadata",
            "--locked",
            "--offline",
            "--format-version",
            "1",
            "--manifest-path",
        ]);
        arguments.push(self.source.join(super::ROOT_MANIFEST).into_os_string());
        arguments.extend(os_strings(&["--filter-platform", super::TARGET]));
        self.command(&self.cargo, arguments)
    }

    fn tree_command(&self) -> CommandSpec {
        self.cargo_command(&[
            "tree",
            "--locked",
            "--offline",
            "-p",
            super::ROOT_PACKAGE,
            "-e",
            "normal,build",
            "--target",
            super::TARGET,
            "--prefix",
            "none",
            "--format",
            "{p}\t{f}",
        ])
    }

    fn tool_fingerprints(&self) -> Result<[ToolFingerprint; 2], String> {
        Ok([
            ToolFingerprint::read(&self.cargo, "Cargo executable")?,
            ToolFingerprint::read(&self.rustc, "rustc executable")?,
        ])
    }
}

struct CommandSpec {
    program: PathBuf,
    current_dir: PathBuf,
    arguments: Vec<OsString>,
    environment: BTreeMap<OsString, OsString>,
    clear_environment: bool,
}

impl CommandSpec {
    fn output(
        &self,
        label: &str,
        max_bytes: u64,
        timeout: std::time::Duration,
    ) -> Result<String, String> {
        let mut command = Command::new(&self.program);
        command.current_dir(&self.current_dir);
        if self.clear_environment {
            command.env_clear();
        }
        command.envs(&self.environment).args(&self.arguments);
        process::output(command, label, max_bytes, timeout)
    }
}

struct Inputs {
    lock: Vec<u8>,
    toolchain: Vec<u8>,
}

impl Inputs {
    fn read(source: &Path) -> Result<Self, String> {
        Ok(Self {
            lock: super::authority::read(&source.join("Cargo.lock"), MAX_LOCK_BYTES)?,
            toolchain: super::authority::read(
                &source.join("rust-toolchain.toml"),
                MAX_TOOLCHAIN_BYTES,
            )?,
        })
    }

    fn bind(&self, expected: &Receipt) -> Result<(), String> {
        if super::sha256(&self.lock) != expected.cargo_lock_sha256 {
            return Err("tracked closure receipt does not bind the actual Cargo.lock".to_owned());
        }
        if super::sha256(&self.toolchain) != expected.rust_toolchain_sha256 {
            return Err(
                "tracked closure receipt does not bind the actual rust-toolchain.toml".to_owned(),
            );
        }
        Ok(())
    }

    fn ensure_unchanged(&self, source: &Path) -> Result<(), String> {
        let current = Self::read(source)?;
        if self.lock != current.lock || self.toolchain != current.toolchain {
            Err("closure lock or toolchain input changed during verification".to_owned())
        } else {
            Ok(())
        }
    }
}

#[derive(PartialEq, Eq)]
struct ToolFingerprint {
    sha256: String,
    byte_length: u64,
}

impl ToolFingerprint {
    fn read(path: &Path, label: &str) -> Result<Self, String> {
        let mut file = File::open(path)
            .map_err(|error| format!("open {label} {}: {error}", path.display()))?;
        let metadata = file
            .metadata()
            .map_err(|error| format!("inspect {label} {}: {error}", path.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_EXECUTABLE_BYTES {
            return Err(format!("{label} is not a bounded regular file"));
        }
        let mut digest = Sha256::new();
        let mut byte_length = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("read {label}: {error}"))?;
            if read == 0 {
                break;
            }
            byte_length = byte_length
                .checked_add(read as u64)
                .ok_or_else(|| format!("{label} size overflow"))?;
            if byte_length > MAX_EXECUTABLE_BYTES {
                return Err(format!("{label} exceeds its byte bound"));
            }
            digest.update(&buffer[..read]);
        }
        if byte_length != metadata.len() {
            return Err(format!("{label} changed during read"));
        }
        Ok(Self {
            sha256: format!("{:x}", digest.finalize()),
            byte_length,
        })
    }
}

fn capture(context: &Context, inputs: &Inputs) -> Result<Receipt, String> {
    let cargo = version_and_host(&context.cargo_command(&["-Vv"]), "Cargo verbose version")?;
    let rustc = version_and_host(&context.rustc_command(&["-vV"]), "rustc verbose version")?;
    if cargo.host != rustc.host || cargo.host != super::TARGET {
        return Err(format!(
            "controlled closure requires native host/target {}; cargo={}, rustc={}",
            super::TARGET,
            cargo.host,
            rustc.host
        ));
    }
    let raw_metadata = context.metadata_command().output(
        "controlled cargo metadata",
        super::MAX_METADATA_BYTES,
        super::CARGO_TIMEOUT,
    )?;
    let raw_tree = context.tree_command().output(
        "controlled cargo tree",
        super::MAX_TREE_BYTES,
        super::CARGO_TIMEOUT,
    )?;
    let raw_cfg = context
        .rustc_command(&["--print", "cfg", "--target", super::TARGET])
        .output(
            "controlled rustc target cfg",
            super::MAX_CFG_BYTES,
            super::TOOL_TIMEOUT,
        )?;
    let target = platform::TargetContext::parse(super::TARGET, &raw_cfg)?;
    let packages = metadata::parse(&raw_metadata, &raw_tree, &context.source, &target)?;
    Receipt::from_parts(
        super::sha256(&inputs.lock),
        super::sha256(&inputs.toolchain),
        &cargo.version,
        &rustc.version,
        &cargo.host,
        packages,
    )
}

struct VersionHost {
    version: String,
    host: String,
}

fn version_and_host(command: &CommandSpec, label: &str) -> Result<VersionHost, String> {
    let output = command.output(label, super::MAX_TOOL_OUTPUT_BYTES, super::TOOL_TIMEOUT)?;
    let version = output.lines().next().unwrap_or_default().trim();
    super::validate_text(label, version)?;
    let hosts: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix("host: "))
        .collect();
    let [host] = hosts.as_slice() else {
        return Err(format!("{label} has no unique host"));
    };
    super::validate_text("tool host", host)?;
    Ok(VersionHost {
        version: version.to_owned(),
        host: (*host).to_owned(),
    })
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    validate_absolute_normal(path, label)?;
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))?;
    if canonical != path {
        return Err(format!("{label} is not canonical"));
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("inspect {label}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{label} is not a non-symlink directory"));
    }
    Ok(canonical)
}

fn canonical_executable(path: &Path, label: &str) -> Result<PathBuf, String> {
    validate_absolute_normal(path, label)?;
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))?;
    if canonical != path {
        return Err(format!("{label} is not canonical"));
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("inspect {label}: {error}"))?;
    validate_executable(&metadata, label)?;
    Ok(canonical)
}

fn validate_executable(metadata: &Metadata, label: &str) -> Result<(), String> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{label} is not a non-symlink regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o111 == 0 {
            return Err(format!("{label} is not executable"));
        }
    }
    Ok(())
}

fn validate_absolute_normal(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        Err(format!("{label} must be absolute and normalized"))
    } else {
        Ok(())
    }
}

fn reject_active_cargo_configs(source: &Path, cargo_home: &Path) -> Result<(), String> {
    let mut candidates = Vec::new();
    for directory in source.ancestors() {
        candidates.push(directory.join(".cargo/config"));
        candidates.push(directory.join(".cargo/config.toml"));
    }
    for name in ["config", "config.toml", "credentials", "credentials.toml"] {
        candidates.push(cargo_home.join(name));
    }
    for candidate in candidates {
        match fs::symlink_metadata(&candidate) {
            Ok(_) => {
                return Err(format!(
                    "ambient Cargo configuration or credentials are forbidden: {}",
                    candidate.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "inspect Cargo authority {}: {error}",
                    candidate.display()
                ));
            }
        }
    }
    Ok(())
}

fn os_strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[cfg(test)]
#[path = "controlled_tests.rs"]
mod tests;
