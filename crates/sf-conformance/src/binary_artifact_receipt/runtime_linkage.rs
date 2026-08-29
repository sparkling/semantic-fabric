//! Non-authorizing runtime-linkage contract for the current artifact observer.
//!
//! This module only describes and parses one capability-minimal loader probe. It
//! neither executes that probe nor writes, imports, or upgrades a receipt.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use super::sandbox;

mod loader_output;
// This is a staged capability boundary: tests exercise it now, while a later
// prepared-probe slice will become its sole production caller.
#[cfg(target_os = "linux")]
#[allow(
    dead_code,
    reason = "sealed runtime authority is not wired to execution yet"
)]
mod object_authority;
#[cfg(test)]
mod tests;

pub const LOADER_POLICY: &str =
    "glibc-ldso-list-env-clear-inhibit-cache-empty-hwcaps-host-inputs-read-only-v1";
pub const MAX_LOADER_OUTPUT_BYTES: usize = 512 * 1024;
pub const MAX_RESOLVED_OBJECTS: usize = 256;
pub const MAX_LOADER_STDERR_BYTES: u64 = 16 * 1024;
pub const LOADER_TIMEOUT: Duration = Duration::from_secs(10);

const ARTIFACT_DESTINATION: &str = "/artifact";
const RUNTIME_DESTINATIONS: [&str; 4] = ["/lib", "/lib64", "/usr/lib", "/usr/lib64"];

/// One filesystem object named by the controlled loader's semantic output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResolvedRuntimeObject {
    soname: String,
    resolved_path: String,
}

impl ResolvedRuntimeObject {
    pub fn soname(&self) -> &str {
        &self.soname
    }

    pub fn resolved_path(&self) -> &str {
        &self.resolved_path
    }
}

/// One loader-reported virtual object for which this contract records no bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtualRuntimeObject {
    name: String,
}

impl VirtualRuntimeObject {
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// A semantic view over bounded loader output, not an attestation or receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLinkageView {
    artifact_sha256: String,
    elf_interpreter: String,
    direct_needed: Vec<String>,
    loader_path: String,
    resolved_objects: Vec<ResolvedRuntimeObject>,
    virtual_objects: Vec<VirtualRuntimeObject>,
}

impl RuntimeLinkageView {
    pub fn artifact_sha256(&self) -> &str {
        &self.artifact_sha256
    }

    pub fn elf_interpreter(&self) -> &str {
        &self.elf_interpreter
    }

    pub fn loader_policy(&self) -> &'static str {
        LOADER_POLICY
    }

    pub fn direct_needed(&self) -> &[String] {
        &self.direct_needed
    }

    pub fn loader_path(&self) -> &str {
        &self.loader_path
    }

    pub fn resolved_objects(&self) -> &[ResolvedRuntimeObject] {
        &self.resolved_objects
    }

    pub fn virtual_objects(&self) -> &[VirtualRuntimeObject] {
        &self.virtual_objects
    }
}

/// Parses one exact loader output into a deterministic, non-authorizing view.
pub fn parse_runtime_linkage_view(
    artifact_sha256: &str,
    elf_interpreter: &str,
    direct_needed: &[String],
    output: &[u8],
) -> Result<RuntimeLinkageView, String> {
    validate_sha256(artifact_sha256)?;
    validate_runtime_file_path(elf_interpreter, "ELF interpreter")?;
    validate_direct_needed(direct_needed)?;
    let parsed = loader_output::parse(output)?;
    if parsed.loader_path != elf_interpreter {
        return Err("loader output does not name the exact ELF interpreter".to_owned());
    }

    let loader_name = Path::new(elf_interpreter)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "ELF interpreter has no UTF-8 file name".to_owned())?;
    for needed in direct_needed {
        let resolution_count = parsed
            .resolved_objects
            .iter()
            .filter(|object| object.soname.as_str() == needed.as_str())
            .count()
            + usize::from(loader_name == needed.as_str());
        if resolution_count != 1 {
            return Err(format!(
                "direct DT_NEEDED object does not have one resolution: {needed}"
            ));
        }
    }
    Ok(RuntimeLinkageView {
        artifact_sha256: artifact_sha256.to_owned(),
        elf_interpreter: elf_interpreter.to_owned(),
        direct_needed: direct_needed.to_vec(),
        loader_path: parsed.loader_path,
        resolved_objects: parsed.resolved_objects,
        virtual_objects: parsed.virtual_objects,
    })
}

/// Exact bubblewrap command data with read-only host inputs. No method executes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLoaderPlan {
    executable: PathBuf,
    artifact: PathBuf,
    interpreter: String,
    mounts: Vec<RuntimeReadOnlyMount>,
    arguments: Vec<OsString>,
    clear_parent_environment: bool,
    max_stdout_bytes: u64,
    max_stderr_bytes: u64,
    timeout: Duration,
}

impl RuntimeLoaderPlan {
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    pub fn clears_parent_environment(&self) -> bool {
        self.clear_parent_environment
    }

    pub fn max_stdout_bytes(&self) -> u64 {
        self.max_stdout_bytes
    }

    pub fn max_stderr_bytes(&self) -> u64 {
        self.max_stderr_bytes
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RuntimeReadOnlyMount {
    source: PathBuf,
    destination: String,
}

/// Builds command data for the exact controlled loader policy without running it.
///
/// This is not execution authorization. A later collector must independently
/// bind artifact bytes and reject loader-active dynamic tags before invocation.
pub fn plan_runtime_linkage(
    bwrap: &Path,
    artifact: &Path,
    elf_interpreter: &str,
) -> Result<RuntimeLoaderPlan, String> {
    let mounts = sandbox::system_mounts()?
        .into_iter()
        .filter(|mount| RUNTIME_DESTINATIONS.contains(&mount.destination))
        .map(|mount| RuntimeReadOnlyMount {
            source: mount.source,
            destination: mount.destination.to_owned(),
        })
        .collect();
    build_plan(bwrap, artifact, elf_interpreter, mounts)
}

fn build_plan(
    bwrap: &Path,
    artifact: &Path,
    elf_interpreter: &str,
    mut mounts: Vec<RuntimeReadOnlyMount>,
) -> Result<RuntimeLoaderPlan, String> {
    validate_plan_inputs(bwrap, artifact, elf_interpreter, &mounts)?;
    mounts.sort_by(|left, right| {
        (&left.destination, &left.source).cmp(&(&right.destination, &right.source))
    });
    let arguments = plan_arguments(artifact, elf_interpreter, &mounts);
    let plan = RuntimeLoaderPlan {
        executable: bwrap.to_path_buf(),
        artifact: artifact.to_path_buf(),
        interpreter: elf_interpreter.to_owned(),
        mounts,
        arguments,
        clear_parent_environment: true,
        max_stdout_bytes: MAX_LOADER_OUTPUT_BYTES as u64,
        max_stderr_bytes: MAX_LOADER_STDERR_BYTES,
        timeout: LOADER_TIMEOUT,
    };
    validate_plan(&plan)?;
    Ok(plan)
}

fn validate_plan(plan: &RuntimeLoaderPlan) -> Result<(), String> {
    validate_plan_inputs(
        &plan.executable,
        &plan.artifact,
        &plan.interpreter,
        &plan.mounts,
    )?;
    if !plan.clear_parent_environment
        || plan.max_stdout_bytes != MAX_LOADER_OUTPUT_BYTES as u64
        || plan.max_stderr_bytes != MAX_LOADER_STDERR_BYTES
        || plan.timeout != LOADER_TIMEOUT
    {
        return Err("runtime loader process policy drift".to_owned());
    }
    if plan.arguments != plan_arguments(&plan.artifact, &plan.interpreter, &plan.mounts) {
        return Err("runtime loader argument policy drift".to_owned());
    }
    Ok(())
}

fn validate_plan_inputs(
    bwrap: &Path,
    artifact: &Path,
    elf_interpreter: &str,
    mounts: &[RuntimeReadOnlyMount],
) -> Result<(), String> {
    validate_absolute_path(bwrap, "bubblewrap executable")?;
    validate_absolute_path(artifact, "artifact")?;
    validate_runtime_file_path(elf_interpreter, "ELF interpreter")?;
    if mounts.is_empty() {
        return Err("runtime loader plan has no system library mounts".to_owned());
    }
    let mut destinations = BTreeSet::new();
    for mount in mounts {
        validate_absolute_path(&mount.source, "runtime mount source")?;
        validate_runtime_path(&mount.destination, "runtime mount destination")?;
        if !RUNTIME_DESTINATIONS.contains(&mount.destination.as_str())
            || !destinations.insert(mount.destination.as_str())
            || artifact.starts_with(&mount.source)
            || mount.source.starts_with(artifact)
        {
            return Err("runtime loader mount is outside the exact allowlist".to_owned());
        }
    }
    if !destinations.contains("/usr/lib")
        || !mounts
            .iter()
            .any(|mount| Path::new(elf_interpreter).starts_with(&mount.destination))
    {
        return Err("runtime loader mounts do not cover the fixed library policy".to_owned());
    }
    Ok(())
}

fn plan_arguments(
    artifact: &Path,
    elf_interpreter: &str,
    mounts: &[RuntimeReadOnlyMount],
) -> Vec<OsString> {
    let mut arguments = strings(&[
        "--die-with-parent",
        "--new-session",
        "--unshare-all",
        "--unshare-net",
        "--clearenv",
        "--tmpfs",
        "/",
        "--cap-drop",
        "ALL",
    ]);
    let parents: BTreeSet<_> = mounts
        .iter()
        .filter_map(|mount| Path::new(&mount.destination).parent())
        .filter(|parent| *parent != Path::new("/"))
        .collect();
    for parent in parents {
        arguments.extend(["--dir".into(), parent.as_os_str().to_owned()]);
    }
    arguments.extend([
        "--ro-bind".into(),
        artifact.as_os_str().to_owned(),
        ARTIFACT_DESTINATION.into(),
    ]);
    for mount in mounts {
        arguments.extend([
            "--ro-bind".into(),
            mount.source.as_os_str().to_owned(),
            mount.destination.as_str().into(),
        ]);
    }
    arguments.extend(strings(&[
        "--setenv",
        "LC_ALL",
        "C",
        "--chdir",
        "/",
        "--",
        elf_interpreter,
        "--inhibit-cache",
        "--glibc-hwcaps-mask",
        "",
        "--list",
        ARTIFACT_DESTINATION,
    ]));
    arguments
}

fn strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn validate_direct_needed(values: &[String]) -> Result<(), String> {
    if values.is_empty() || values.len() > MAX_RESOLVED_OBJECTS {
        return Err("direct DT_NEEDED count is outside bounds".to_owned());
    }
    if !values.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err("direct DT_NEEDED names are not strictly ordered".to_owned());
    }
    for value in values {
        validate_soname(value)?;
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err("artifact SHA-256 is not canonical lowercase hexadecimal".to_owned())
    }
}

fn validate_soname(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    {
        Err("invalid runtime object name".to_owned())
    } else {
        Ok(())
    }
}

fn validate_runtime_path(value: &str, label: &str) -> Result<(), String> {
    let path = Path::new(value);
    validate_absolute_path(path, label)?;
    if value.len() > 4096
        || value.contains("//")
        || value.contains('\\')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._+-".contains(&byte))
        || !RUNTIME_DESTINATIONS
            .iter()
            .any(|root| path.starts_with(root))
    {
        Err(format!(
            "{label} is outside normalized runtime library roots"
        ))
    } else {
        Ok(())
    }
}

fn validate_runtime_file_path(value: &str, label: &str) -> Result<(), String> {
    validate_runtime_path(value, label)?;
    if RUNTIME_DESTINATIONS.contains(&value) {
        Err(format!("{label} cannot equal a runtime library root"))
    } else {
        Ok(())
    }
}

fn validate_absolute_path(path: &Path, label: &str) -> Result<(), String> {
    let text = path
        .to_str()
        .ok_or_else(|| format!("{label} is not UTF-8"))?;
    if !path.is_absolute()
        || text.is_empty()
        || text.len() > 4096
        || text.contains("//")
        || text.ends_with('/')
        || text
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        || text.contains('\\')
        || text.bytes().any(|byte| byte.is_ascii_control())
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        Err(format!("{label} must be an absolute normalized path"))
    } else {
        Ok(())
    }
}
