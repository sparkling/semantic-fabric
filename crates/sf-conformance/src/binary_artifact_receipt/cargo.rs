//! Bound Cargo/Rustc identities and sandbox-logical Cargo JSON parsing.

use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use super::{authority, process};

pub(super) const ROOT_BINARY: &str = "semantic-fabric";
pub(super) const TARGET: &str = "x86_64-unknown-linux-gnu";
pub(super) const LOGICAL_TARGET_DIR: &str = "/target";
pub(super) const LOGICAL_ARTIFACT: &str =
    "/target/x86_64-unknown-linux-gnu/release/semantic-fabric";

const MAX_SANDBOX_STDOUT: usize = 64 * 1024 * 1024;
const MAX_JSON_LINE: usize = 1024 * 1024;
const MAX_BUILD_SCRIPTS: usize = 2_000;
const MAX_TOOL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_TOOL_OUTPUT: u64 = 64 * 1024;
const TOOL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Executable {
    pub(super) path: PathBuf,
    pub(super) sha256: String,
    pub(super) byte_length: u64,
    pub(super) version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RustcIdentity {
    pub(super) executable: Executable,
    pub(super) host: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BuildMessages {
    /// Sandbox-logical and fixed at [`LOGICAL_ARTIFACT`].
    pub(super) logical_artifact: PathBuf,
    pub(super) package_id: String,
    pub(super) build_scripts: Vec<BuildScript>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BuildScript {
    pub(super) package_id: String,
    /// Sandbox-logical OUT_DIR below `/target`.
    pub(super) logical_out_dir: PathBuf,
}

/// Binds an absolute tool executable and its single-line `--version` result.
pub(super) fn identify(path: &Path, role: &str) -> Result<Executable, String> {
    let (sha256, byte_length) = digest_tool(path, role)?;
    let version = first_line(&invoke(path, &["--version"], role)?, role)?;
    let (sha256_after, byte_length_after) = digest_tool(path, role)?;
    if sha256 != sha256_after || byte_length != byte_length_after {
        return Err(format!("{role} changed while identifying it"));
    }
    Ok(Executable {
        path: path.to_path_buf(),
        sha256,
        byte_length,
        version,
    })
}

/// Binds Rustc and rejects a `rustc -vV` host other than the fixed target.
pub(super) fn identify_rustc(path: &Path) -> Result<RustcIdentity, String> {
    let (sha256, byte_length) = digest_tool(path, "rustc executable")?;
    let verbose = utf8(&invoke(path, &["-vV"], "rustc executable")?, "rustc -vV")?;
    let version = first_line(verbose.as_bytes(), "rustc -vV")?;
    let host = required_host(&verbose)?;
    if host != TARGET {
        return Err(format!("rustc -vV host must be exactly {TARGET}"));
    }
    let (sha256_after, byte_length_after) = digest_tool(path, "rustc executable")?;
    if sha256 != sha256_after || byte_length != byte_length_after {
        return Err("rustc executable changed while identifying it".to_owned());
    }
    Ok(RustcIdentity {
        executable: Executable {
            path: path.to_path_buf(),
            sha256,
            byte_length,
            version,
        },
        host,
    })
}

/// Parses bounded stdout from the sandboxed exact Cargo command. Host paths are
/// never accepted: each selected path must be sandbox-logical under `/target`.
pub(super) fn parse_sandbox_stdout(
    bytes: &[u8],
    expected_package_id: &str,
) -> Result<BuildMessages, String> {
    if bytes.is_empty() || bytes.len() > MAX_SANDBOX_STDOUT {
        return Err("sandbox Cargo stdout is empty or exceeds its bound".to_owned());
    }
    validate_package_id(expected_package_id)?;
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("sandbox Cargo stdout is not UTF-8: {error}"))?;
    let mut artifact = None;
    let mut package_id = None;
    let mut build_scripts = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line.is_empty() || line.len() > MAX_JSON_LINE {
            return Err(format!("invalid sandbox Cargo JSON line {line_number}"));
        }
        let message: Message = serde_json::from_str(line)
            .map_err(|error| format!("parse sandbox Cargo JSON line {line_number}: {error}"))?;
        match message.reason.as_str() {
            "compiler-artifact" if message.is_root_binary(expected_package_id) => {
                let executable = message
                    .executable
                    .ok_or_else(|| "root binary compiler-artifact has no executable".to_owned())?;
                let executable = logical_path(&executable, "root binary executable")?;
                if executable != Path::new(LOGICAL_ARTIFACT) {
                    return Err(format!(
                        "root binary executable must be exactly {LOGICAL_ARTIFACT}"
                    ));
                }
                if artifact.replace(executable).is_some() {
                    return Err(
                        "sandbox Cargo stdout has multiple root binary artifacts".to_owned()
                    );
                }
                package_id = Some(message.package_id);
            }
            "build-script-executed" => {
                validate_build_script_package_id(&message.package_id)?;
                let out_dir = message
                    .out_dir
                    .ok_or_else(|| "build-script-executed message has no out_dir".to_owned())?;
                let logical_out_dir = logical_path(&out_dir, "build-script OUT_DIR")?;
                if !logical_out_dir.starts_with(LOGICAL_TARGET_DIR)
                    || logical_out_dir == Path::new(LOGICAL_TARGET_DIR)
                {
                    return Err("build-script OUT_DIR escapes logical /target".to_owned());
                }
                if build_scripts.len() == MAX_BUILD_SCRIPTS {
                    return Err(format!(
                        "sandbox Cargo stdout exceeds {MAX_BUILD_SCRIPTS} build scripts"
                    ));
                }
                build_scripts.push(BuildScript {
                    package_id: message.package_id,
                    logical_out_dir,
                });
            }
            _ => {}
        }
    }
    let logical_artifact =
        artifact.ok_or_else(|| "sandbox Cargo stdout has no root binary artifact".to_owned())?;
    let package_id =
        package_id.ok_or_else(|| "sandbox Cargo stdout has no root package ID".to_owned())?;
    build_scripts.sort_by(|left, right| {
        left.package_id
            .cmp(&right.package_id)
            .then(left.logical_out_dir.cmp(&right.logical_out_dir))
    });
    if build_scripts.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("sandbox Cargo stdout has duplicate build-script execution".to_owned());
    }
    Ok(BuildMessages {
        logical_artifact,
        package_id,
        build_scripts,
    })
}

fn digest_tool(path: &Path, role: &str) -> Result<(String, u64), String> {
    if !path.is_absolute() {
        return Err(format!("{role} path must be absolute"));
    }
    authority::digest(path, MAX_TOOL_BYTES, role)
}

fn invoke(path: &Path, arguments: &[&str], role: &str) -> Result<Vec<u8>, String> {
    let mut command = Command::new(path);
    command.env_clear().env("LC_ALL", "C").args(arguments);
    let output = process::run(
        command,
        role,
        MAX_TOOL_OUTPUT,
        MAX_TOOL_OUTPUT,
        TOOL_TIMEOUT,
    )?;
    if !output.stderr.is_empty() {
        return Err(format!("{role} wrote stderr while reporting identity"));
    }
    Ok(output.stdout)
}

fn utf8(bytes: &[u8], label: &str) -> Result<String, String> {
    String::from_utf8(bytes.to_vec()).map_err(|error| format!("{label} is not UTF-8: {error}"))
}

fn first_line(bytes: &[u8], label: &str) -> Result<String, String> {
    let value = utf8(bytes, label)?;
    let line = value.lines().next().unwrap_or_default().trim();
    if line.is_empty() || line.len() > 4096 || line.bytes().any(|byte| byte.is_ascii_control()) {
        Err(format!("invalid {label} version"))
    } else {
        Ok(line.to_owned())
    }
}

fn required_host(output: &str) -> Result<String, String> {
    let hosts: Vec<_> = output
        .lines()
        .filter_map(|line| line.strip_prefix("host: "))
        .collect();
    match hosts.as_slice() {
        [host]
            if !host.is_empty()
                && host.len() <= 128
                && host
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)) =>
        {
            Ok((*host).to_owned())
        }
        _ => Err("rustc -vV has no unique valid host field".to_owned()),
    }
}

fn logical_path(value: &str, label: &str) -> Result<PathBuf, String> {
    if value.is_empty()
        || value.len() > 16 * 1024
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(format!("invalid {label} path"));
    }
    let path = PathBuf::from(value);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
        || value.contains("//")
        || value.contains('\\')
    {
        Err(format!("invalid {label} path"))
    } else {
        Ok(path)
    }
}

fn validate_package_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 16 * 1024
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        Err("invalid Cargo package ID".to_owned())
    } else {
        Ok(())
    }
}

fn validate_build_script_package_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 512
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || !byte.is_ascii())
    {
        Err("invalid Cargo build-script package ID".to_owned())
    } else {
        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const PACKAGE: &str = "path+file:///workspace/crates/sf-cli#sf-cli@0.0.0";

    #[test]
    fn selects_exact_logical_artifact_and_sorts_build_scripts() {
        let stream = concat!(
            "{\"reason\":\"build-script-executed\",\"package_id\":\"z\",\"out_dir\":\"/target/build/z/out\"}\n",
            "{\"reason\":\"compiler-artifact\",\"package_id\":\"path+file:///workspace/crates/sf-cli#sf-cli@0.0.0\",\"target\":{\"name\":\"semantic-fabric\",\"kind\":[\"bin\"]},\"profile\":{\"opt_level\":\"3\",\"debug\":false,\"test\":false},\"executable\":\"/target/x86_64-unknown-linux-gnu/release/semantic-fabric\"}\n",
            "{\"reason\":\"build-script-executed\",\"package_id\":\"a\",\"out_dir\":\"/target/build/a/out\"}\n"
        );
        let parsed = parse_sandbox_stdout(stream.as_bytes(), PACKAGE).unwrap();
        assert_eq!(parsed.logical_artifact, Path::new(LOGICAL_ARTIFACT));
        assert_eq!(parsed.build_scripts[0].package_id, "a");
        assert_eq!(parsed.build_scripts[1].package_id, "z");
    }

    #[test]
    fn rejects_host_paths_wrong_output_and_bad_hosts() {
        let wrong_output = format!(
            "{{\"reason\":\"compiler-artifact\",\"package_id\":\"{PACKAGE}\",\"target\":{{\"name\":\"semantic-fabric\",\"kind\":[\"bin\"]}},\"profile\":{{\"opt_level\":\"3\",\"debug\":false,\"test\":false}},\"executable\":\"/target/release/semantic-fabric\"}}\n"
        );
        assert!(parse_sandbox_stdout(wrong_output.as_bytes(), PACKAGE).is_err());
        assert!(logical_path("/target/../escape", "test").is_err());
        assert!(required_host("host: x86_64-unknown-linux-gnu\nhost: other\n").is_err());
    }

    #[test]
    fn bounds_build_script_package_ids_to_receipt_schema() {
        assert!(validate_build_script_package_id(&"a".repeat(512)).is_ok());
        assert!(validate_build_script_package_id(&"a".repeat(513)).is_err());
    }
}
