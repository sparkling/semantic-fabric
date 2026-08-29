//! One-shot loader observation over read-only copies sourced from sealed memfds.
//!
//! Bubblewrap is executed from a held, digest-fenced root-owned inode. It copies
//! only already sealed artifact, loader, and DSO bytes into a fresh tmpfs root,
//! then applies an exact late seccomp policy to its PID-1 reaper and loader child.
//! Matching `ld.so --list` output confirms only that this held byte set reproduces
//! candidate resolution under those fixed policies. It proves neither artifact
//! execution nor syscall trace, host-tool closure, provenance, or admission.

use std::ffi::OsString;
use std::os::fd::AsRawFd;
#[cfg(test)]
use std::os::fd::FromRawFd;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use super::HeldRuntimeInputs;
use crate::binary_artifact_receipt::process;
use crate::binary_artifact_receipt::runtime_linkage::MAX_PREPARED_SET_BYTES;

mod bindings;
mod policy;
mod receipt;
mod seccomp;
mod tool;

pub(super) use crate::binary_artifact_receipt::runtime_linkage::PREPARED_LOADER_POLICY;
pub(super) use bindings::PreparedBindingIdentity;
use bindings::ProbeBinding;
pub(super) use policy::{
    ExpectedBwrapIdentity, ExpectedPreparedSeccompPolicy, ExpectedRuntimeElfPolicy,
};
#[cfg(test)]
pub(super) use seccomp::{
    canonical_bytes_for_test as seccomp_bytes_for_test,
    policy_identity_for_test as seccomp_identity_for_test,
    validate_bytes_for_test as validate_seccomp_bytes_for_test, PREPARED_SECCOMP_POLICY,
};
const MIN_TRANSFER_FD: libc::c_int = 64;
const MAX_TRANSFER_FD: libc::c_int = 1023;

#[derive(Debug)]
pub(super) struct PreparedRuntimeProbe<'a> {
    held: HeldRuntimeInputs<'a>,
    bwrap: tool::HeldBwrap,
    expected_bwrap: ExpectedBwrapIdentity,
    expected_runtime_elf_policy: ExpectedRuntimeElfPolicy,
    expected_seccomp_policy: ExpectedPreparedSeccompPolicy,
    bindings: Vec<ProbeBinding>,
    seccomp: seccomp::PreparedSeccompPolicy,
    pub(super) arguments: Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedRuntimeObservation {
    pub(super) loader_policy: &'static str,
    pub(super) runtime_elf_policy: &'static str,
    pub(super) runtime_elf_policy_sha256: String,
    pub(super) seccomp_policy: &'static str,
    pub(super) seccomp_policy_sha256: String,
    pub(super) seccomp_policy_byte_length: u64,
    pub(super) view: super::super::RuntimeLinkageView,
    pub(super) bindings: Vec<PreparedBindingIdentity>,
    pub(super) bwrap_sha256: String,
    pub(super) bwrap_byte_length: u64,
    pub(super) bwrap_path: PathBuf,
    pub(super) bwrap_executable_policy: String,
    pub(super) stdout: Vec<u8>,
    pub(super) stdout_sha256: String,
}

pub(super) fn execute(
    held: HeldRuntimeInputs<'_>,
    expected_bwrap: ExpectedBwrapIdentity,
    expected_runtime_elf_policy: ExpectedRuntimeElfPolicy,
    expected_seccomp_policy: ExpectedPreparedSeccompPolicy,
) -> Result<PreparedRuntimeObservation, String> {
    PreparedRuntimeProbe::prepare(
        held,
        expected_bwrap,
        expected_runtime_elf_policy,
        expected_seccomp_policy,
    )?
    .execute()
}

#[cfg(test)]
pub(super) fn execute_seccomp_canary_for_test(
    expected_bwrap: ExpectedBwrapIdentity,
    expected_runtime_elf_policy: ExpectedRuntimeElfPolicy,
    expected_seccomp_policy: ExpectedPreparedSeccompPolicy,
    canary: std::fs::File,
) -> Result<process::Output, String> {
    let bwrap = tool::HeldBwrap::bind(&expected_bwrap, &expected_runtime_elf_policy)?;
    let seccomp = seccomp::PreparedSeccompPolicy::new(&expected_seccomp_policy)?;
    // Match production's descriptor topology. The original low descriptor
    // remains CLOEXEC while only this high, sealed duplicate crosses exec.
    // Otherwise a low canary FD can shift bubblewrap's PID-1 eventfd away
    // from the policy's exact write descriptor and create a false SIGSYS.
    // SAFETY: F_DUPFD_CLOEXEC returns a new independently owned descriptor.
    let canary_transfer_fd =
        unsafe { libc::fcntl(canary.as_raw_fd(), libc::F_DUPFD_CLOEXEC, MIN_TRANSFER_FD) };
    if !(MIN_TRANSFER_FD..=MAX_TRANSFER_FD).contains(&canary_transfer_fd) {
        if canary_transfer_fd >= 0 {
            // SAFETY: this branch owns the just-created descriptor.
            unsafe { libc::close(canary_transfer_fd) };
        }
        return Err("prepared seccomp canary descriptor is outside its fixed range".to_owned());
    }
    // SAFETY: ownership of the duplicated descriptor transfers to File.
    let canary_transfer = unsafe { std::fs::File::from_raw_fd(canary_transfer_fd) };
    let canary_fd = canary_transfer.as_raw_fd();
    let seccomp_fd = seccomp.raw_fd();
    if canary_fd == seccomp_fd {
        return Err("prepared seccomp canary aliases the policy descriptor".to_owned());
    }
    let arguments: Vec<OsString> = [
        "bwrap".into(),
        "--die-with-parent".into(),
        "--unshare-all".into(),
        "--unshare-user".into(),
        "--unshare-net".into(),
        "--disable-userns".into(),
        "--assert-userns-disabled".into(),
        "--clearenv".into(),
        "--size".into(),
        "1048576".into(),
        "--tmpfs".into(),
        "/".into(),
        "--cap-drop".into(),
        "ALL".into(),
        "--perms".into(),
        "0555".into(),
        "--ro-bind-data".into(),
        canary_fd.to_string().into(),
        "/canary".into(),
        "--remount-ro".into(),
        "/".into(),
        "--chdir".into(),
        "/".into(),
        "--seccomp".into(),
        seccomp_fd.to_string().into(),
        "--".into(),
        "/canary".into(),
    ]
    .to_vec();
    seccomp.rewind()?;
    let output = process::run_execveat(
        process::ExecveatRequest {
            executable_fd: bwrap.raw_fd(),
            argv: arguments,
            data_fds: vec![canary_fd, seccomp_fd],
        },
        "prepared seccomp forbidden-syscall canary",
        1,
        1024,
        super::super::LOADER_TIMEOUT,
    );
    let tool_current = bwrap.assert_current();
    let tool_policy_current =
        expected_runtime_elf_policy.assert_matches(bwrap.runtime_elf_policy());
    let seccomp_current = seccomp.assert_current(&expected_seccomp_policy);
    tool_current?;
    tool_policy_current?;
    seccomp_current?;
    output
}

impl<'a> PreparedRuntimeProbe<'a> {
    pub(super) fn prepare(
        held: HeldRuntimeInputs<'a>,
        expected_bwrap: ExpectedBwrapIdentity,
        expected_runtime_elf_policy: ExpectedRuntimeElfPolicy,
        expected_seccomp_policy: ExpectedPreparedSeccompPolicy,
    ) -> Result<Self, String> {
        expected_runtime_elf_policy.assert_matches(held.runtime_elf_policy())?;
        held.assert_current()?;
        if held.total_bytes == 0 || held.total_bytes > MAX_PREPARED_SET_BYTES {
            return Err("prepared runtime set exceeds the execution byte budget".to_owned());
        }
        if held.bwrap_path != expected_bwrap.path {
            return Err("prepared bubblewrap path differs from authorized identity".to_owned());
        }
        let bwrap = tool::HeldBwrap::bind(&expected_bwrap, &expected_runtime_elf_policy)?;
        Self::assemble(
            held,
            bwrap,
            expected_bwrap,
            expected_runtime_elf_policy,
            expected_seccomp_policy,
        )
    }

    #[cfg(test)]
    pub(super) fn prepare_with_test_tool(
        held: HeldRuntimeInputs<'a>,
        expected_bwrap: ExpectedBwrapIdentity,
        expected_runtime_elf_policy: ExpectedRuntimeElfPolicy,
        expected_seccomp_policy: ExpectedPreparedSeccompPolicy,
    ) -> Result<Self, String> {
        expected_runtime_elf_policy.assert_matches(held.runtime_elf_policy())?;
        held.assert_current()?;
        if held.total_bytes == 0 || held.total_bytes > MAX_PREPARED_SET_BYTES {
            return Err("prepared runtime set exceeds the execution byte budget".to_owned());
        }
        if held.bwrap_path != expected_bwrap.path {
            return Err("prepared bubblewrap path differs from authorized identity".to_owned());
        }
        let bwrap = tool::HeldBwrap::bind_fixture(&expected_bwrap, &expected_runtime_elf_policy)?;
        Self::assemble(
            held,
            bwrap,
            expected_bwrap,
            expected_runtime_elf_policy,
            expected_seccomp_policy,
        )
    }

    fn assemble(
        held: HeldRuntimeInputs<'a>,
        bwrap: tool::HeldBwrap,
        expected_bwrap: ExpectedBwrapIdentity,
        expected_runtime_elf_policy: ExpectedRuntimeElfPolicy,
        expected_seccomp_policy: ExpectedPreparedSeccompPolicy,
    ) -> Result<Self, String> {
        let bindings = bindings::create(&held)?;
        let seccomp = seccomp::PreparedSeccompPolicy::new(&expected_seccomp_policy)?;
        bindings::validate(&bindings, &held, seccomp.raw_fd())?;
        let arguments = bindings::arguments(
            &bindings,
            seccomp.raw_fd(),
            held.candidate.elf_interpreter(),
        );
        let probe = Self {
            held,
            bwrap,
            expected_bwrap,
            expected_runtime_elf_policy,
            expected_seccomp_policy,
            bindings,
            seccomp,
            arguments,
        };
        probe.validate()?;
        Ok(probe)
    }

    fn execute(self) -> Result<PreparedRuntimeObservation, String> {
        self.execute_with(|request| {
            process::run_execveat(
                request,
                "prepared runtime loader probe",
                super::super::MAX_LOADER_OUTPUT_BYTES as u64,
                super::super::MAX_LOADER_STDERR_BYTES,
                super::super::LOADER_TIMEOUT,
            )
        })
    }

    fn execute_with(
        self,
        runner: impl FnOnce(process::ExecveatRequest) -> Result<process::Output, String>,
    ) -> Result<PreparedRuntimeObservation, String> {
        self.validate()?;
        self.held.assert_current()?;
        self.bwrap.assert_current()?;
        self.seccomp.rewind()?;
        let request = process::ExecveatRequest {
            executable_fd: self.bwrap.raw_fd(),
            argv: self.arguments.clone(),
            data_fds: self
                .bindings
                .iter()
                .map(|binding| binding.transfer.as_raw_fd())
                .chain(std::iter::once(self.seccomp.raw_fd()))
                .collect(),
        };
        let output = runner(request);
        let tool_current = self.bwrap.assert_current();
        let inputs_current = self.held.assert_current();
        let seccomp_current = self.seccomp.assert_current(&self.expected_seccomp_policy);
        let bindings_current =
            bindings::validate(&self.bindings, &self.held, self.seccomp.raw_fd());
        tool_current?;
        inputs_current?;
        seccomp_current?;
        bindings_current?;
        let output = output?;
        self.finish(output)
    }

    #[cfg(test)]
    pub(super) fn execute_for_test(
        self,
        runner: impl FnOnce(process::ExecveatRequest) -> Result<process::Output, String>,
    ) -> Result<PreparedRuntimeObservation, String> {
        self.execute_with(runner)
    }

    fn finish(self, output: process::Output) -> Result<PreparedRuntimeObservation, String> {
        if !output.stderr.is_empty() {
            return Err("prepared runtime loader probe wrote stderr".to_owned());
        }
        let candidate = &self.held.candidate;
        let view = super::super::parse_runtime_linkage_view(
            candidate.artifact_sha256(),
            candidate.elf_interpreter(),
            candidate.direct_needed(),
            &output.stdout,
        )?;
        if view != *candidate {
            return Err("prepared runtime linkage differs from candidate discovery".to_owned());
        }
        let stdout_sha256 = format!("{:x}", Sha256::digest(&output.stdout));
        Ok(PreparedRuntimeObservation {
            loader_policy: PREPARED_LOADER_POLICY,
            runtime_elf_policy: self.held.runtime_elf_policy().id(),
            runtime_elf_policy_sha256: self.held.runtime_elf_policy().sha256().to_owned(),
            seccomp_policy: self.seccomp.identity().id(),
            seccomp_policy_sha256: self.seccomp.identity().sha256().to_owned(),
            seccomp_policy_byte_length: self.seccomp.identity().byte_length(),
            view,
            bindings: self
                .bindings
                .iter()
                .map(|binding| binding.identity.clone())
                .collect(),
            bwrap_sha256: self.bwrap.sha256().to_owned(),
            bwrap_byte_length: self.bwrap.byte_length(),
            bwrap_path: self.expected_bwrap.path,
            bwrap_executable_policy: self.expected_bwrap.executable_policy,
            stdout: output.stdout,
            stdout_sha256,
        })
    }

    #[cfg(test)]
    pub(super) fn finish_for_test(
        self,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    ) -> Result<PreparedRuntimeObservation, String> {
        self.finish(process::Output { stdout, stderr })
    }

    fn validate(&self) -> Result<(), String> {
        self.expected_runtime_elf_policy
            .assert_matches(self.held.runtime_elf_policy())?;
        self.expected_runtime_elf_policy
            .assert_matches(self.bwrap.runtime_elf_policy())?;
        self.held.assert_current()?;
        self.bwrap.assert_current()?;
        self.seccomp.assert_current(&self.expected_seccomp_policy)?;
        if self.bwrap.raw_fd() == self.seccomp.raw_fd() {
            return Err("prepared executable aliases the seccomp policy".to_owned());
        }
        bindings::validate(&self.bindings, &self.held, self.seccomp.raw_fd())?;
        if self.arguments.is_empty() || self.arguments.len() > process::MAX_EXECVEAT_ARGUMENTS {
            return Err("prepared runtime loader argument count exceeds policy".to_owned());
        }
        let argument_bytes = self.arguments.iter().try_fold(0usize, |total, value| {
            total
                .checked_add(value.as_encoded_bytes().len() + 1)
                .ok_or_else(|| "prepared runtime loader argument byte count overflow".to_owned())
        })?;
        if argument_bytes > process::MAX_EXECVEAT_ARGUMENT_BYTES {
            return Err("prepared runtime loader argument bytes exceed policy".to_owned());
        }
        if self.arguments
            != bindings::arguments(
                &self.bindings,
                self.seccomp.raw_fd(),
                self.held.candidate.elf_interpreter(),
            )
        {
            return Err("prepared runtime loader argument policy drift".to_owned());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn validate_for_test(&self) -> Result<(), String> {
        self.validate()
    }

    #[cfg(test)]
    pub(super) fn bwrap_elf_for_test(
        &self,
    ) -> &crate::binary_artifact_receipt::runtime_elf::RuntimeElfView {
        self.bwrap.elf()
    }

    #[cfg(test)]
    pub(super) fn swap_transfers_for_test(&mut self, left: usize, right: usize) {
        assert_ne!(left, right);
        let (left, right) = if left < right {
            let (before, after) = self.bindings.split_at_mut(right);
            (&mut before[left].transfer, &mut after[0].transfer)
        } else {
            let (before, after) = self.bindings.split_at_mut(left);
            (&mut after[0].transfer, &mut before[right].transfer)
        };
        std::mem::swap(left, right);
    }

    #[cfg(test)]
    pub(super) fn replace_expected_runtime_elf_policy_for_test(
        &mut self,
        expected: ExpectedRuntimeElfPolicy,
    ) {
        self.expected_runtime_elf_policy = expected;
    }

    #[cfg(test)]
    pub(super) fn replace_expected_seccomp_policy_for_test(
        &mut self,
        expected: ExpectedPreparedSeccompPolicy,
    ) {
        self.expected_seccomp_policy = expected;
    }

    #[cfg(test)]
    pub(super) fn replace_seccomp_transfer_for_test(&mut self, replacement: std::fs::File) {
        self.seccomp.replace_transfer_for_test(replacement);
    }

    #[cfg(test)]
    pub(super) fn seccomp_fd_for_test(&self) -> libc::c_int {
        self.seccomp.raw_fd()
    }
}
