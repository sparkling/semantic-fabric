//! Exact sealed-source-copy bindings and bubblewrap argument policy.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::path::{Path, PathBuf};

use super::super::{HeldRuntimeInputs, HeldRuntimeObject, RuntimeObjectIdentity};
use super::{MAX_TRANSFER_FD, MIN_TRANSFER_FD};
use crate::binary_artifact_receipt::runtime_elf::RuntimeElfRole;

const ARTIFACT_DESTINATION: &str = "/artifact";
const FILE_MODE: u32 = 0o444;
const EXECUTABLE_MODE: u32 = 0o555;
const ROOT_TMPFS_BYTES_TEXT: &str = "134217728";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) struct PreparedBindingIdentity {
    pub(in super::super) object: RuntimeObjectIdentity,
    pub(in super::super) destination: String,
    pub(in super::super) mode: u32,
}

#[derive(Debug)]
pub(super) struct ProbeBinding {
    pub(super) identity: PreparedBindingIdentity,
    pub(super) transfer: File,
}

pub(super) fn create(held: &HeldRuntimeInputs<'_>) -> Result<Vec<ProbeBinding>, String> {
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
    Ok(bindings)
}

fn binding(
    object: &HeldRuntimeObject,
    destination: &str,
    mode: u32,
) -> Result<ProbeBinding, String> {
    super::super::linux::assert_sealed_current(&object.sealed)?;
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

pub(super) fn validate(
    bindings: &[ProbeBinding],
    held: &HeldRuntimeInputs<'_>,
    seccomp_fd: RawFd,
) -> Result<(), String> {
    if bindings.len() != held.identities.len() || bindings.len() < 3 {
        return Err("prepared runtime binding count differs from held inputs".to_owned());
    }
    if !(MIN_TRANSFER_FD..=MAX_TRANSFER_FD).contains(&seccomp_fd) {
        return Err("prepared seccomp descriptor is outside binding policy".to_owned());
    }
    let mut destinations = BTreeSet::new();
    let mut descriptors = BTreeSet::from([seccomp_fd]);
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
                super::super::super::validate_runtime_file_path(destination, "prepared loader")?;
                if destination != held.candidate.elf_interpreter()
                    || binding.identity.mode != EXECUTABLE_MODE
                {
                    return Err("prepared loader binding is outside policy".to_owned());
                }
            }
            RuntimeElfRole::Libc | RuntimeElfRole::SharedObject => {
                super::super::super::validate_runtime_file_path(
                    destination,
                    "prepared runtime object",
                )?;
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
        super::super::linux::assert_sealed_current(sealed)?;
        super::super::linux::assert_sealed_duplicate_current(&binding.transfer, sealed)?;
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
) -> Result<&'a super::super::linux::SealedBytes, String> {
    std::iter::once(&held.artifact)
        .chain(std::iter::once(&held.loader))
        .chain(held.objects.iter())
        .find(|object| object.identity == *identity)
        .map(|object| &object.sealed)
        .ok_or_else(|| "prepared binding has no held sealed source".to_owned())
}

pub(super) fn arguments(
    bindings: &[ProbeBinding],
    seccomp_fd: RawFd,
    interpreter: &str,
) -> Vec<OsString> {
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
        "--seccomp",
    ]));
    values.push(seccomp_fd.to_string().into());
    values.extend(strings(&[
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
