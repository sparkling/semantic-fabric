//! One-shot loader observation over read-only copies sourced from sealed memfds.
//!
//! Bubblewrap is executed from a held, digest-fenced root-owned inode. It copies
//! only the already sealed artifact, loader, and DSO bytes into a fresh tmpfs
//! root. Matching `ld.so --list` output confirms only that this held byte set
//! reproduces the candidate resolution under the fixed policy; it does not prove
//! artifact execution, relocations, a complete closure, direct memfd consumption,
//! or receipt, replay, admission, provenance, minimality, or release authority.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{HeldRuntimeInputs, HeldRuntimeObject, RuntimeObjectIdentity};
use crate::binary_artifact_receipt::{process, runtime_elf::RuntimeElfRole};

mod tool;

pub(super) const PREPARED_LOADER_POLICY: &str =
    "glibc-ldso-list-sealed-source-copies-bwrap-inode-execveat-v1";

const ARTIFACT_DESTINATION: &str = "/artifact";
const FILE_MODE: u32 = 0o444;
const EXECUTABLE_MODE: u32 = 0o555;
const MAX_PREPARED_SET_BYTES: u64 = 64 * 1024 * 1024;
const ROOT_TMPFS_BYTES_TEXT: &str = "134217728";
const MIN_TRANSFER_FD: libc::c_int = 64;
const MAX_TRANSFER_FD: libc::c_int = 1023;
const MAX_EXECUTABLE_POLICY_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExpectedBwrapIdentity {
    path: PathBuf,
    sha256: String,
    byte_length: u64,
    executable_policy: String,
}

impl ExpectedBwrapIdentity {
    pub(super) fn new(
        path: &Path,
        sha256: &str,
        byte_length: u64,
        executable_policy: &str,
    ) -> Result<Self, String> {
        super::super::validate_absolute_path(path, "expected bubblewrap executable")?;
        super::super::validate_sha256(sha256)?;
        if byte_length == 0
            || executable_policy.is_empty()
            || executable_policy.len() > MAX_EXECUTABLE_POLICY_BYTES
            || !executable_policy.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-._".contains(&byte)
            })
        {
            return Err("expected bubblewrap identity is outside policy".to_owned());
        }
        Ok(Self {
            path: path.to_path_buf(),
            sha256: sha256.to_owned(),
            byte_length,
            executable_policy: executable_policy.to_owned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedBindingIdentity {
    pub(super) object: RuntimeObjectIdentity,
    pub(super) destination: String,
    pub(super) mode: u32,
}

#[derive(Debug)]
pub(super) struct ProbeBinding {
    identity: PreparedBindingIdentity,
    transfer: File,
}

#[derive(Debug)]
pub(super) struct PreparedRuntimeProbe<'a> {
    held: HeldRuntimeInputs<'a>,
    bwrap: tool::HeldBwrap,
    expected_bwrap: ExpectedBwrapIdentity,
    bindings: Vec<ProbeBinding>,
    pub(super) arguments: Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedRuntimeObservation {
    pub(super) loader_policy: &'static str,
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
) -> Result<PreparedRuntimeObservation, String> {
    PreparedRuntimeProbe::prepare(held, expected_bwrap)?.execute()
}

impl<'a> PreparedRuntimeProbe<'a> {
    pub(super) fn prepare(
        held: HeldRuntimeInputs<'a>,
        expected_bwrap: ExpectedBwrapIdentity,
    ) -> Result<Self, String> {
        held.assert_current()?;
        if held.total_bytes == 0 || held.total_bytes > MAX_PREPARED_SET_BYTES {
            return Err("prepared runtime set exceeds the execution byte budget".to_owned());
        }
        if held.bwrap_path != expected_bwrap.path {
            return Err("prepared bubblewrap path differs from authorized identity".to_owned());
        }
        let bwrap = tool::HeldBwrap::bind(&expected_bwrap)?;
        Self::assemble(held, bwrap, expected_bwrap)
    }

    #[cfg(test)]
    pub(super) fn prepare_with_test_tool(
        held: HeldRuntimeInputs<'a>,
        expected_bwrap: ExpectedBwrapIdentity,
    ) -> Result<Self, String> {
        held.assert_current()?;
        if held.total_bytes == 0 || held.total_bytes > MAX_PREPARED_SET_BYTES {
            return Err("prepared runtime set exceeds the execution byte budget".to_owned());
        }
        if held.bwrap_path != expected_bwrap.path {
            return Err("prepared bubblewrap path differs from authorized identity".to_owned());
        }
        let bwrap = tool::HeldBwrap::bind_fixture(&expected_bwrap)?;
        Self::assemble(held, bwrap, expected_bwrap)
    }

    fn assemble(
        held: HeldRuntimeInputs<'a>,
        bwrap: tool::HeldBwrap,
        expected_bwrap: ExpectedBwrapIdentity,
    ) -> Result<Self, String> {
        let mut bindings = Vec::with_capacity(held.objects.len() + 2);
        bindings.push(binding(&held.artifact, ARTIFACT_DESTINATION, FILE_MODE)?);
        bindings.push(binding(
            &held.loader,
            &held.loader.identity.logical_path,
            EXECUTABLE_MODE,
        )?);
        for object in &held.objects {
            bindings.push(binding(object, &object.identity.logical_path, FILE_MODE)?);
        }
        bindings.sort_by(|left, right| left.identity.destination.cmp(&right.identity.destination));
        validate_bindings(&bindings, &held)?;
        let arguments = arguments(&bindings, held.candidate.elf_interpreter());
        let probe = Self {
            held,
            bwrap,
            expected_bwrap,
            bindings,
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
        let request = process::ExecveatRequest {
            executable_fd: self.bwrap.raw_fd(),
            argv: self.arguments.clone(),
            data_fds: self
                .bindings
                .iter()
                .map(|binding| binding.transfer.as_raw_fd())
                .collect(),
        };
        let output = runner(request);
        let tool_current = self.bwrap.assert_current();
        let inputs_current = self.held.assert_current();
        let bindings_current = validate_bindings(&self.bindings, &self.held);
        tool_current?;
        inputs_current?;
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
        self.held.assert_current()?;
        self.bwrap.assert_current()?;
        validate_bindings(&self.bindings, &self.held)?;
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
        if self.arguments != arguments(&self.bindings, self.held.candidate.elf_interpreter()) {
            return Err("prepared runtime loader argument policy drift".to_owned());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn validate_for_test(&self) -> Result<(), String> {
        self.validate()
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
}

fn binding(
    object: &HeldRuntimeObject,
    destination: &str,
    mode: u32,
) -> Result<ProbeBinding, String> {
    super::linux::assert_sealed_current(&object.sealed)?;
    // SAFETY: the sealed source descriptor is live; F_DUPFD_CLOEXEC returns a
    // new independently owned descriptor that shares only the read offset.
    let descriptor = unsafe {
        libc::fcntl(
            object.sealed.file.as_raw_fd(),
            libc::F_DUPFD_CLOEXEC,
            MIN_TRANSFER_FD,
        )
    };
    if !(MIN_TRANSFER_FD..=MAX_TRANSFER_FD).contains(&descriptor) {
        if descriptor >= 0 {
            // SAFETY: this branch owns the just-created descriptor.
            unsafe { libc::close(descriptor) };
        }
        return Err("prepared transfer descriptor is outside its fixed range".to_owned());
    }
    // SAFETY: ownership of the new descriptor transfers to File.
    let transfer = unsafe { File::from_raw_fd(descriptor) };
    Ok(ProbeBinding {
        identity: PreparedBindingIdentity {
            object: object.identity.clone(),
            destination: destination.to_owned(),
            mode,
        },
        transfer,
    })
}

fn validate_bindings(
    bindings: &[ProbeBinding],
    held: &HeldRuntimeInputs<'_>,
) -> Result<(), String> {
    if bindings.len() != held.identities.len() || bindings.len() < 3 {
        return Err("prepared runtime binding count differs from held inputs".to_owned());
    }
    let mut destinations = BTreeSet::new();
    let mut descriptors = BTreeSet::new();
    let mut artifact = 0;
    let mut loader = 0;
    for binding in bindings {
        let destination = binding.identity.destination.as_str();
        match binding.identity.object.role {
            RuntimeElfRole::RootPie => {
                artifact += 1;
                if destination != ARTIFACT_DESTINATION || binding.identity.mode != FILE_MODE {
                    return Err("prepared artifact binding is outside policy".to_owned());
                }
            }
            RuntimeElfRole::Loader => {
                loader += 1;
                super::super::validate_runtime_file_path(destination, "prepared loader")?;
                if destination != held.candidate.elf_interpreter()
                    || binding.identity.mode != EXECUTABLE_MODE
                {
                    return Err("prepared loader binding is outside policy".to_owned());
                }
            }
            RuntimeElfRole::Libc | RuntimeElfRole::SharedObject => {
                super::super::validate_runtime_file_path(destination, "prepared runtime object")?;
                if binding.identity.mode != FILE_MODE {
                    return Err("prepared runtime object mode is outside policy".to_owned());
                }
            }
        }
        let descriptor = binding.transfer.as_raw_fd();
        if !destinations.insert(destination)
            || !descriptors.insert(descriptor)
            || !(MIN_TRANSFER_FD..=MAX_TRANSFER_FD).contains(&descriptor)
        {
            return Err("prepared runtime binding is ambiguous".to_owned());
        }
        let sealed = object_for(held, &binding.identity.object)?;
        super::linux::assert_sealed_current(sealed)?;
        super::linux::assert_sealed_duplicate_current(&binding.transfer, sealed)?;
    }
    if artifact != 1 || loader != 1 {
        return Err("prepared runtime binding roles are incomplete".to_owned());
    }
    let bound: Vec<_> = bindings
        .iter()
        .map(|binding| binding.identity.object.clone())
        .collect();
    let mut expected = held.identities.clone();
    expected.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    let mut bound_sorted = bound;
    bound_sorted.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    if bound_sorted != expected {
        return Err("prepared runtime bindings differ from held identities".to_owned());
    }
    Ok(())
}

fn object_for<'a>(
    held: &'a HeldRuntimeInputs<'_>,
    identity: &RuntimeObjectIdentity,
) -> Result<&'a super::linux::SealedBytes, String> {
    std::iter::once(&held.artifact)
        .chain(std::iter::once(&held.loader))
        .chain(held.objects.iter())
        .find(|object| object.identity == *identity)
        .map(|object| &object.sealed)
        .ok_or_else(|| "prepared binding has no held sealed source".to_owned())
}

fn arguments(bindings: &[ProbeBinding], interpreter: &str) -> Vec<OsString> {
    // `--new-session` is deliberately absent: stdio is non-TTY and process-group
    // teardown depends on bubblewrap remaining in the controller-created group.
    let mut values = strings(&[
        "bwrap",
        "--die-with-parent",
        "--unshare-all",
        "--unshare-user",
        "--unshare-net",
        "--disable-userns",
        "--assert-userns-disabled",
        "--clearenv",
        "--size",
        ROOT_TMPFS_BYTES_TEXT,
        "--tmpfs",
        "/",
        "--cap-drop",
        "ALL",
    ]);
    for parent in binding_parents(bindings) {
        values.extend(["--dir".into(), parent.into_os_string()]);
    }
    for binding in bindings {
        values.extend([
            "--perms".into(),
            format!("{:04o}", binding.identity.mode).into(),
            "--ro-bind-data".into(),
            binding.transfer.as_raw_fd().to_string().into(),
            binding.identity.destination.as_str().into(),
        ]);
    }
    values.extend(strings(&[
        "--remount-ro",
        "/",
        "--setenv",
        "LC_ALL",
        "C",
        "--chdir",
        "/",
        "--",
        interpreter,
        "--inhibit-cache",
        "--glibc-hwcaps-mask",
        "",
        "--list",
        ARTIFACT_DESTINATION,
    ]));
    values
}

fn binding_parents(bindings: &[ProbeBinding]) -> BTreeSet<PathBuf> {
    bindings
        .iter()
        .flat_map(|binding| Path::new(&binding.identity.destination).ancestors().skip(1))
        .filter(|parent| *parent != Path::new("/"))
        .map(Path::to_path_buf)
        .collect()
}

fn strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
