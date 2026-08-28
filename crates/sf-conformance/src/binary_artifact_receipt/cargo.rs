//! Controlled Cargo invocation and bounded JSON-message selection.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use super::{authority, process};

pub(super) const ROOT_PACKAGE: &str = "sf-cli";
pub(super) const ROOT_BINARY: &str = "semantic-fabric";
pub(super) const TARGET: &str = "x86_64-unknown-linux-gnu";
const MAX_CARGO_OUTPUT: u64 = 64 * 1024 * 1024;
const MAX_CARGO_STDERR: u64 = 16 * 1024 * 1024;
const BUILD_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const MAX_BUILD_SCRIPTS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Executable {
    pub(super) path: PathBuf,
    pub(super) sha256: String,
    pub(super) byte_length: u64,
    pub(super) version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BuildRequest<'a> {
    pub(super) repository: &'a Path,
    pub(super) cargo: &'a Path,
    pub(super) rustc: &'a Path,
    pub(super) linker: &'a Path,
    pub(super) linker_argument: &'a str,
    pub(super) cargo_home: &'a Path,
    pub(super) rustup_home: &'a Path,
    pub(super) target_dir: &'a Path,
    pub(super) link_dependency_file: &'a Path,
    pub(super) link_dependency_file_argument: &'a str,
    pub(super) temporary_dir: &'a Path,
    pub(super) expected_package_id: &'a str,
    pub(super) source_date_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BuildMessages {
    pub(super) artifact: PathBuf,
    pub(super) package_id: String,
    pub(super) build_scripts: Vec<BuildScript>,
    pub(super) cargo_stdout_sha256: String,
    pub(super) cargo_stderr_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BuildScript {
    pub(super) package_id: String,
    pub(super) out_dir: PathBuf,
}

pub(super) fn identify(path: &Path, role: &str) -> Result<Executable, String> {
    if !path.is_absolute() {
        return Err(format!("{role} path must be absolute"));
    }
    let (sha256, byte_length) = authority::digest(path, 2 * 1024 * 1024 * 1024, role)?;
    let mut command = Command::new(path);
    command.env_clear().env("LC_ALL", "C").arg("--version");
    let output = process::run(command, role, 64 * 1024, 64 * 1024, Duration::from_secs(10))?;
    if !output.stderr.is_empty() {
        return Err(format!("{role} wrote stderr while reporting its version"));
    }
    let version = String::from_utf8(output.stdout)
        .map_err(|error| format!("{role} version is not UTF-8: {error}"))?
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    if version.is_empty()
        || version.len() > 4096
        || version.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(format!("invalid {role} version"));
    }
    Ok(Executable {
        path: path.to_path_buf(),
        sha256,
        byte_length,
        version,
    })
}

pub(super) fn build(request: &BuildRequest<'_>) -> Result<BuildMessages, String> {
    validate_request(request)?;
    let before = tool_digests(request)?;
    let mut command = Command::new(request.cargo);
    command
        .current_dir(request.repository)
        .env_clear()
        .env("CARGO_HOME", request.cargo_home)
        .env("RUSTUP_HOME", request.rustup_home)
        .env("RUSTC", request.rustc)
        .env("CARGO_INCREMENTAL", "0")
        .env("CARGO_NET_OFFLINE", "true")
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .env("HOME", request.cargo_home)
        .env("TMPDIR", request.temporary_dir)
        .env("SOURCE_DATE_EPOCH", request.source_date_epoch.to_string())
        .env("PATH", controlled_path(request)?)
        .args([
            "rustc",
            "--locked",
            "--offline",
            "--release",
            "-p",
            ROOT_PACKAGE,
            "--bin",
            ROOT_BINARY,
            "--target",
            TARGET,
            "--target-dir",
        ])
        .arg(request.target_dir)
        .args([
            "--message-format=json-render-diagnostics",
            "--",
            "-C",
            &format!("linker={}", request.linker_argument),
            "-C",
            &format!(
                "link-arg=-Wl,--dependency-file={}",
                request.link_dependency_file_argument
            ),
        ]);
    let output = process::run(
        command,
        "controlled cargo build",
        MAX_CARGO_OUTPUT,
        MAX_CARGO_STDERR,
        BUILD_TIMEOUT,
    )?;
    if tool_digests(request)? != before {
        return Err("Cargo, rustc, or linker authority changed during build".to_owned());
    }
    let parsed = parse_messages(
        &output.stdout,
        request.target_dir,
        request.expected_package_id,
    )?;
    Ok(BuildMessages {
        cargo_stdout_sha256: sha256(&output.stdout),
        cargo_stderr_sha256: sha256(&output.stderr),
        ..parsed
    })
}

fn tool_digests(request: &BuildRequest<'_>) -> Result<Vec<(String, u64)>, String> {
    [
        (request.cargo, "cargo executable"),
        (request.rustc, "rustc executable"),
        (request.linker, "linker executable"),
    ]
    .into_iter()
    .map(|(path, label)| authority::digest(path, 2 * 1024 * 1024 * 1024, label))
    .collect()
}

fn validate_request(request: &BuildRequest<'_>) -> Result<(), String> {
    for (path, label) in [
        (request.repository, "repository"),
        (request.cargo_home, "synthetic CARGO_HOME"),
        (request.rustup_home, "RUSTUP_HOME"),
        (request.target_dir, "fresh target directory"),
        (request.temporary_dir, "controlled temporary directory"),
    ] {
        if !path.is_absolute() {
            return Err(format!("{label} must be absolute"));
        }
        authority::validate_directory(path, label)?;
    }
    if fs::read_dir(request.target_dir)
        .map_err(|error| format!("read fresh target directory: {error}"))?
        .next()
        .transpose()
        .map_err(|error| format!("enumerate fresh target directory: {error}"))?
        .is_some()
    {
        return Err("fresh target directory is not empty".to_owned());
    }
    if request.target_dir.starts_with(request.repository) {
        return Err("fresh target directory must be outside the repository".to_owned());
    }
    if !request.link_dependency_file.is_absolute()
        || !request.link_dependency_file.starts_with(request.target_dir)
        || request.link_dependency_file == request.target_dir
    {
        return Err("link dependency file must be inside the fresh target directory".to_owned());
    }
    if request.link_dependency_file.exists() {
        return Err("link dependency file already exists before the build".to_owned());
    }
    for (path, label) in [
        (request.cargo, "cargo executable"),
        (request.rustc, "rustc executable"),
        (request.linker, "linker executable"),
    ] {
        if !path.is_absolute() {
            return Err(format!("{label} must be absolute"));
        }
        let _ = authority::digest(path, 2 * 1024 * 1024 * 1024, label)?;
    }
    for (value, label) in [
        (request.linker_argument, "linker argument"),
        (
            request.link_dependency_file_argument,
            "link dependency file argument",
        ),
        (request.expected_package_id, "expected root package ID"),
    ] {
        if value.is_empty()
            || value.len() > 16 * 1024
            || value.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(format!("invalid {label}"));
        }
    }
    Ok(())
}

fn controlled_path(request: &BuildRequest<'_>) -> Result<std::ffi::OsString, String> {
    let mut directories = Vec::new();
    for executable in [request.cargo, request.rustc, request.linker] {
        let directory = executable
            .parent()
            .ok_or_else(|| "tool executable has no parent directory".to_owned())?;
        authority::validate_directory(directory, "tool executable parent")?;
        if !directories.iter().any(|known: &PathBuf| known == directory) {
            directories.push(directory.to_path_buf());
        }
    }
    for directory in [Path::new("/usr/bin")] {
        authority::validate_directory(directory, "controlled system PATH directory")?;
        if !directories.iter().any(|known| known == directory) {
            directories.push(directory.to_path_buf());
        }
    }
    env::join_paths(directories).map_err(|error| format!("construct controlled PATH: {error}"))
}

fn parse_messages(
    bytes: &[u8],
    target_dir: &Path,
    expected_package_id: &str,
) -> Result<BuildMessages, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("Cargo JSON stream is not UTF-8: {error}"))?;
    let mut artifact = None;
    let mut package_id = None;
    let mut build_scripts = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        let line_number = line_number + 1;
        if line.is_empty() {
            return Err(format!(
                "Cargo JSON stream contains an empty line at {line_number}"
            ));
        }
        let message: Message = serde_json::from_str(line)
            .map_err(|error| format!("parse Cargo JSON line {line_number}: {error}"))?;
        match message.reason.as_str() {
            "compiler-artifact" if message.is_root_binary(expected_package_id) => {
                let executable = message
                    .executable
                    .ok_or_else(|| "root binary compiler-artifact has no executable".to_owned())?;
                let executable = absolute_path(&executable, "root binary executable")?;
                validate_target_path(target_dir, &executable, "root binary executable")?;
                if artifact.replace(executable).is_some() {
                    return Err(
                        "Cargo JSON stream contains multiple root binary artifacts".to_owned()
                    );
                }
                package_id = Some(message.package_id);
            }
            "build-script-executed" => {
                let out_dir = message
                    .out_dir
                    .ok_or_else(|| "build-script-executed message has no out_dir".to_owned())?;
                let out_dir = absolute_path(&out_dir, "build-script out_dir")?;
                validate_target_path(target_dir, &out_dir, "build-script out_dir")?;
                if build_scripts.len() == MAX_BUILD_SCRIPTS {
                    return Err(format!(
                        "Cargo JSON stream exceeds {MAX_BUILD_SCRIPTS} build scripts"
                    ));
                }
                build_scripts.push(BuildScript {
                    package_id: message.package_id,
                    out_dir,
                });
            }
            _ => {}
        }
    }
    let artifact =
        artifact.ok_or_else(|| "Cargo JSON stream has no root binary artifact".to_owned())?;
    let package_id =
        package_id.ok_or_else(|| "Cargo JSON stream has no root package ID".to_owned())?;
    build_scripts.sort_by(|left, right| {
        left.package_id
            .cmp(&right.package_id)
            .then(left.out_dir.cmp(&right.out_dir))
    });
    if build_scripts.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("Cargo JSON stream contains duplicate build-script execution".to_owned());
    }
    Ok(BuildMessages {
        artifact,
        package_id,
        build_scripts,
        cargo_stdout_sha256: String::new(),
        cargo_stderr_sha256: String::new(),
    })
}

#[derive(Deserialize)]
struct Message {
    reason: String,
    package_id: String,
    #[serde(default)]
    target: Target,
    #[serde(default)]
    executable: Option<String>,
    #[serde(default)]
    out_dir: Option<String>,
    #[serde(default)]
    profile: Profile,
}

impl Message {
    fn is_root_binary(&self, expected_package_id: &str) -> bool {
        self.target.name == ROOT_BINARY
            && self.target.kind.iter().any(|kind| kind == "bin")
            && self.package_id == expected_package_id
            && self.profile.is_release()
    }
}

#[derive(Default, Deserialize)]
struct Target {
    #[serde(default)]
    name: String,
    #[serde(default)]
    kind: Vec<String>,
}

#[derive(Default, Deserialize)]
struct Profile {
    #[serde(default)]
    opt_level: String,
    #[serde(default)]
    debug: bool,
    #[serde(default)]
    test: bool,
}

impl Profile {
    fn is_release(&self) -> bool {
        self.opt_level == "3" && !self.debug && !self.test
    }
}

fn absolute_path(value: &str, label: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() || value.bytes().any(|byte| byte.is_ascii_control()) {
        Err(format!("invalid {label} path"))
    } else {
        Ok(path)
    }
}

fn validate_target_path(target_dir: &Path, value: &Path, label: &str) -> Result<(), String> {
    if !value.starts_with(target_dir) {
        return Err(format!("{label} escapes the fresh target directory"));
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_one_root_binary_and_sorted_build_scripts() {
        let target = Path::new("/tmp/capture-target");
        let stream = concat!(
            "{\"reason\":\"build-script-executed\",\"package_id\":\"z\",\"out_dir\":\"/tmp/capture-target/build/z/out\"}\n",
            "{\"reason\":\"compiler-artifact\",\"package_id\":\"path+file:///x#0.0.0\",\"target\":{\"name\":\"semantic-fabric\",\"kind\":[\"bin\"]},\"profile\":{\"opt_level\":\"3\",\"debug\":false,\"test\":false},\"executable\":\"/tmp/capture-target/x86_64-unknown-linux-gnu/release/semantic-fabric\"}\n",
            "{\"reason\":\"build-script-executed\",\"package_id\":\"a\",\"out_dir\":\"/tmp/capture-target/build/a/out\"}\n"
        );
        let parsed = parse_messages(stream.as_bytes(), target, "path+file:///x#0.0.0").unwrap();
        assert_eq!(parsed.package_id, "path+file:///x#0.0.0");
        assert_eq!(parsed.build_scripts[0].package_id, "a");
        assert_eq!(parsed.build_scripts[1].package_id, "z");
    }

    #[test]
    fn rejects_missing_or_escaping_root_artifact() {
        let target = Path::new("/tmp/capture-target");
        assert!(parse_messages(b"{\"reason\":\"compiler-artifact\",\"package_id\":\"x\",\"target\":{\"name\":\"other\",\"kind\":[\"bin\"]}}\n", target, "path+file:///x#0.0.0").is_err());
        let stream = "{\"reason\":\"compiler-artifact\",\"package_id\":\"path+file:///x#0.0.0\",\"target\":{\"name\":\"semantic-fabric\",\"kind\":[\"bin\"]},\"profile\":{\"opt_level\":\"3\",\"debug\":false,\"test\":false},\"executable\":\"/tmp/elsewhere/semantic-fabric\"}\n";
        assert!(parse_messages(stream.as_bytes(), target, "path+file:///x#0.0.0").is_err());
    }
}
