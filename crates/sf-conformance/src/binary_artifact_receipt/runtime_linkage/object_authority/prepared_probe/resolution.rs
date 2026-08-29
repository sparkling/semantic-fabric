//! Counterfactual controlled name-resolution over the held bubblewrap identity.
//!
//! The interpreter path comes from the exact held RootPie bytes, but the loader
//! itself and every reported DSO are host path resolutions. The target is also
//! passed to the loader by path, not by the held descriptor. Pre/post fences
//! detect ordinary drift without proving time-of-use or same-principal ABA.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::tool::HeldBwrap;
use super::{ExpectedBwrapIdentity, ExpectedRuntimeElfPolicy};
use crate::binary_artifact_receipt::process;
use crate::binary_artifact_receipt::runtime_linkage::bwrap_resolution_inventory::{
    BwrapHostResolutionInventory, BwrapHostResolutionView, RecordedBwrapHostResolution,
    BWRAP_HOST_RESOLUTION_POLICY, RESOLUTION_RELATION,
};
use crate::binary_artifact_receipt::runtime_linkage::{
    parse_runtime_linkage_view, LOADER_TIMEOUT, MAX_LOADER_OUTPUT_BYTES, MAX_LOADER_STDERR_BYTES,
};

const FIXED_ENVIRONMENT: [(&str, &str); 1] = [("LC_ALL", "C")];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) struct BwrapHostResolutionPlan {
    executable: PathBuf,
    arguments: Vec<OsString>,
    direct_needed: Vec<String>,
    clear_parent_environment: bool,
    environment: [(&'static str, &'static str); 1],
    current_directory: PathBuf,
    max_stdout_bytes: u64,
    max_stderr_bytes: u64,
    timeout: Duration,
}

impl BwrapHostResolutionPlan {
    fn new(bwrap: &HeldBwrap, target: &Path) -> Result<Self, String> {
        let executable = bwrap
            .elf()
            .interpreter()
            .ok_or_else(|| "held bubblewrap RootPie has no interpreter".to_owned())?
            .into();
        let mut direct_needed = bwrap.elf().needed().to_vec();
        direct_needed.sort();
        let arguments = arguments(target);
        let plan = Self {
            executable,
            arguments,
            direct_needed,
            clear_parent_environment: true,
            environment: FIXED_ENVIRONMENT,
            current_directory: PathBuf::from("/"),
            max_stdout_bytes: MAX_LOADER_OUTPUT_BYTES as u64,
            max_stderr_bytes: MAX_LOADER_STDERR_BYTES,
            timeout: LOADER_TIMEOUT,
        };
        plan.validate(bwrap, target)?;
        Ok(plan)
    }

    pub(in super::super) fn executable(&self) -> &Path {
        &self.executable
    }

    pub(in super::super) fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    pub(in super::super) fn clears_parent_environment(&self) -> bool {
        self.clear_parent_environment
    }

    pub(in super::super) fn environment(&self) -> &[(&'static str, &'static str)] {
        &self.environment
    }

    pub(in super::super) fn current_directory(&self) -> &Path {
        &self.current_directory
    }

    pub(in super::super) fn max_stdout_bytes(&self) -> u64 {
        self.max_stdout_bytes
    }

    pub(in super::super) fn max_stderr_bytes(&self) -> u64 {
        self.max_stderr_bytes
    }

    pub(in super::super) fn timeout(&self) -> Duration {
        self.timeout
    }

    fn validate(&self, bwrap: &HeldBwrap, target: &Path) -> Result<(), String> {
        bwrap.assert_current()?;
        let interpreter = bwrap
            .elf()
            .interpreter()
            .ok_or_else(|| "held bubblewrap RootPie has no interpreter".to_owned())?;
        super::super::super::validate_runtime_file_path(interpreter, "bubblewrap interpreter")?;
        if self.executable != Path::new(interpreter)
            || self.arguments != arguments(target)
            || !self.clear_parent_environment
            || self.environment != FIXED_ENVIRONMENT
            || self.current_directory != Path::new("/")
            || self.max_stdout_bytes != MAX_LOADER_OUTPUT_BYTES as u64
            || self.max_stderr_bytes != MAX_LOADER_STDERR_BYTES
            || self.timeout != LOADER_TIMEOUT
        {
            return Err("bubblewrap host-resolution process policy drift".to_owned());
        }
        super::super::super::validate_direct_needed(&self.direct_needed)?;
        let mut expected_needed = bwrap.elf().needed().to_vec();
        expected_needed.sort();
        if self.direct_needed != expected_needed {
            return Err("bubblewrap direct DT_NEEDED set drift".to_owned());
        }
        if self.arguments.len() > process::MAX_EXECVEAT_ARGUMENTS {
            return Err("bubblewrap host-resolution argument count exceeds policy".to_owned());
        }
        let argument_bytes = self.arguments.iter().try_fold(0usize, |total, argument| {
            total
                .checked_add(argument.as_encoded_bytes().len() + 1)
                .ok_or_else(|| "bubblewrap host-resolution argument byte overflow".to_owned())
        })?;
        if argument_bytes > process::MAX_EXECVEAT_ARGUMENT_BYTES {
            return Err("bubblewrap host-resolution argument bytes exceed policy".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) struct BwrapHostResolutionObservation {
    view: BwrapHostResolutionView,
    bwrap_path: String,
    bwrap_sha256: String,
    bwrap_byte_length: u64,
    bwrap_executable_policy: String,
    runtime_elf_policy: String,
    runtime_elf_policy_sha256: String,
    stdout: Vec<u8>,
    stdout_sha256: String,
}

impl BwrapHostResolutionObservation {
    pub(in super::super) fn policy(&self) -> &'static str {
        BWRAP_HOST_RESOLUTION_POLICY
    }

    pub(in super::super) fn relation(&self) -> &'static str {
        RESOLUTION_RELATION
    }

    pub(in super::super) fn to_non_authority_inventory(
        &self,
    ) -> Result<BwrapHostResolutionInventory, String> {
        BwrapHostResolutionInventory::from_recorded(RecordedBwrapHostResolution {
            bwrap_path: self.bwrap_path.clone(),
            bwrap_sha256: self.bwrap_sha256.clone(),
            bwrap_byte_length: self.bwrap_byte_length,
            bwrap_executable_policy: self.bwrap_executable_policy.clone(),
            runtime_elf_policy: self.runtime_elf_policy.clone(),
            runtime_elf_policy_sha256: self.runtime_elf_policy_sha256.clone(),
            view: self.view.clone(),
            stdout: self.stdout.clone(),
            stdout_sha256: self.stdout_sha256.clone(),
        })
    }
}

pub(in super::super) fn observe_bwrap_host_resolution(
    expected_bwrap: ExpectedBwrapIdentity,
    expected_runtime_elf_policy: ExpectedRuntimeElfPolicy,
) -> Result<BwrapHostResolutionObservation, String> {
    let held = HeldBwrap::bind(&expected_bwrap, &expected_runtime_elf_policy)?;
    observe_held(held, expected_bwrap, expected_runtime_elf_policy, run_plan)
}

#[cfg(test)]
pub(in super::super) fn observe_bwrap_host_resolution_with_test_tool(
    expected_bwrap: ExpectedBwrapIdentity,
    expected_runtime_elf_policy: ExpectedRuntimeElfPolicy,
    runner: impl FnOnce(&BwrapHostResolutionPlan) -> Result<process::Output, String>,
) -> Result<BwrapHostResolutionObservation, String> {
    let held = HeldBwrap::bind_fixture(&expected_bwrap, &expected_runtime_elf_policy)?;
    observe_held(held, expected_bwrap, expected_runtime_elf_policy, runner)
}

fn observe_held(
    held: HeldBwrap,
    expected_bwrap: ExpectedBwrapIdentity,
    expected_runtime_elf_policy: ExpectedRuntimeElfPolicy,
    runner: impl FnOnce(&BwrapHostResolutionPlan) -> Result<process::Output, String>,
) -> Result<BwrapHostResolutionObservation, String> {
    expected_runtime_elf_policy.assert_matches(held.runtime_elf_policy())?;
    let plan = BwrapHostResolutionPlan::new(&held, &expected_bwrap.path)?;
    held.assert_current()?;
    plan.validate(&held, &expected_bwrap.path)?;
    let output = runner(&plan);
    let tool_current = held.assert_current();
    let policy_current = expected_runtime_elf_policy.assert_matches(held.runtime_elf_policy());
    let plan_current = plan.validate(&held, &expected_bwrap.path);
    tool_current?;
    policy_current?;
    plan_current?;
    finish(held, expected_bwrap, plan, output?)
}

fn run_plan(plan: &BwrapHostResolutionPlan) -> Result<process::Output, String> {
    let mut command = Command::new(plan.executable());
    if plan.clears_parent_environment() {
        command.env_clear();
    }
    for (key, value) in plan.environment() {
        command.env(key, value);
    }
    command
        .current_dir(plan.current_directory())
        .args(plan.arguments());
    process::run(
        command,
        "counterfactual bubblewrap host name-resolution inventory",
        plan.max_stdout_bytes(),
        plan.max_stderr_bytes(),
        plan.timeout(),
    )
}

fn finish(
    held: HeldBwrap,
    expected: ExpectedBwrapIdentity,
    plan: BwrapHostResolutionPlan,
    output: process::Output,
) -> Result<BwrapHostResolutionObservation, String> {
    if !output.stderr.is_empty() {
        return Err("bubblewrap host-resolution inventory wrote stderr".to_owned());
    }
    let interpreter = plan
        .executable
        .to_str()
        .ok_or_else(|| "bubblewrap interpreter path is not UTF-8".to_owned())?;
    let view = parse_runtime_linkage_view(
        held.sha256(),
        interpreter,
        &plan.direct_needed,
        &output.stdout,
    )?;
    let stdout_sha256 = format!("{:x}", Sha256::digest(&output.stdout));
    Ok(BwrapHostResolutionObservation {
        view: BwrapHostResolutionView::from_runtime(view),
        bwrap_path: expected
            .path
            .to_str()
            .ok_or_else(|| "bubblewrap path is not UTF-8".to_owned())?
            .to_owned(),
        bwrap_sha256: held.sha256().to_owned(),
        bwrap_byte_length: held.byte_length(),
        bwrap_executable_policy: expected.executable_policy,
        runtime_elf_policy: held.runtime_elf_policy().id().to_owned(),
        runtime_elf_policy_sha256: held.runtime_elf_policy().sha256().to_owned(),
        stdout: output.stdout,
        stdout_sha256,
    })
}

fn arguments(target: &Path) -> Vec<OsString> {
    vec![
        "--inhibit-cache".into(),
        "--glibc-hwcaps-mask".into(),
        "".into(),
        "--list".into(),
        target.as_os_str().to_owned(),
    ]
}
