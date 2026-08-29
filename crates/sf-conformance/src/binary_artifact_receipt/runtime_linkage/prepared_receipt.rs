//! Provider-free offline verification of one private Linux loader observation.
//!
//! The record is deliberately distinct from the live Linux probe event. Its
//! semantic replay only reparses embedded bounded stdout and checks internal
//! consistency; it performs no process, filesystem, network, model, or provider
//! call and grants no execution, admission, provenance, or release authority.

use std::collections::BTreeSet;
use std::fmt;

use sha2::{Digest, Sha256};

use super::{
    parse_runtime_linkage_view, RuntimeLinkageView, LOADER_POLICY, MAX_BWRAP_BYTES,
    MAX_LOADER_OUTPUT_BYTES, MAX_PREPARED_SET_BYTES, MAX_RESOLVED_OBJECTS,
    MAX_RUNTIME_OBJECT_BYTES, PREPARED_LOADER_POLICY,
};

mod format;
#[cfg(test)]
mod tests;

pub(super) fn parse(input: &str) -> Result<PreparedRuntimeReceipt, String> {
    format::parse(input)
}

pub(super) fn render(receipt: &PreparedRuntimeReceipt) -> Result<String, String> {
    format::render(receipt)
}

pub(super) const HEADER: &str = "semantic-fabric-prepared-runtime-observation-non-admission-v1";
pub(super) const AUTHORITY: &str = "none";
pub(super) const ADMISSION_RESULT: &str = "not-evaluated";
pub(super) const RECORD_DISPOSITION: &str = "non-admission-only";
pub(super) const NOT_ATTESTED: &str = "not-attested";
pub(super) const STDOUT_CHUNK_BYTES: usize = 1024;
pub(super) const RECEIPT_DOMAIN: &[u8] = b"semantic-fabric:prepared-runtime-observation-receipt:v1";
pub(super) const OBSERVATION_DOMAIN: &[u8] =
    b"semantic-fabric:prepared-runtime-observation-record:v1";

pub(super) const NONCLAIM_KEYS: [&str; 34] = [
    "candidate-discovery-authority",
    "artifact-main-execution",
    "relocation-completeness",
    "symbol-version-resolution",
    "initializer-execution",
    "dlopen-closure",
    "nss-closure",
    "vdso-byte-closure",
    "runtime-elf-policy-replay",
    "runtime-closure-completeness",
    "dynamic-runtime-portability",
    "direct-sealed-inode-consumption",
    "bubblewrap-host-runtime-closure",
    "final-child-fd-inventory",
    "target-seccomp-or-syscall-trace",
    "aggregate-cgroup-containment",
    "same-principal-race-resistance",
    "root-race-resistance",
    "rollback-resistance",
    "hostile-kernel-resistance",
    "hostile-filesystem-resistance",
    "prepared-execution-provenance",
    "loader-output-origin",
    "external-witness",
    "signing",
    "provenance",
    "sbom",
    "reproducibility",
    "minimality",
    "backend-admission",
    "production-admission",
    "performance-authority",
    "release-readiness",
    "replay-execution",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecordedRuntimeRole {
    RootPie,
    Loader,
    Libc,
    SharedObject,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct RecordedObjectIdentity {
    pub(super) logical_path: String,
    pub(super) role: RecordedRuntimeRole,
    pub(super) soname: Option<String>,
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) byte_length: u64,
    pub(super) sha256: String,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct RecordedBindingIdentity {
    pub(super) object: RecordedObjectIdentity,
    pub(super) destination: String,
    pub(super) mode: u32,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct RecordedPreparedRuntimeObservation {
    pub(super) view: RuntimeLinkageView,
    pub(super) bindings: Vec<RecordedBindingIdentity>,
    pub(super) bwrap_sha256: String,
    pub(super) bwrap_byte_length: u64,
    pub(super) bwrap_path: String,
    pub(super) bwrap_executable_policy: String,
    pub(super) stdout: Vec<u8>,
    pub(super) stdout_sha256: String,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct PreparedRuntimeReceipt {
    record: RecordedPreparedRuntimeObservation,
}

impl fmt::Debug for PreparedRuntimeReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedRuntimeReceipt")
            .field("authority", &AUTHORITY)
            .field("admission_result", &ADMISSION_RESULT)
            .field("artifact_sha256", &self.record.view.artifact_sha256())
            .field("binding_count", &self.record.bindings.len())
            .field("stdout_sha256", &self.record.stdout_sha256)
            .finish_non_exhaustive()
    }
}

impl PreparedRuntimeReceipt {
    pub(super) fn from_recorded(
        record: RecordedPreparedRuntimeObservation,
    ) -> Result<Self, String> {
        let receipt = Self { record };
        receipt.validate()?;
        format::render(&receipt)?;
        Ok(receipt)
    }

    pub(super) fn record_sha256(&self) -> String {
        domain_sha256(
            OBSERVATION_DOMAIN,
            format::observation_records(&self.record).as_bytes(),
        )
    }

    pub(super) fn receipt_sha256(&self) -> Result<String, String> {
        self.validate()?;
        Ok(domain_sha256(
            RECEIPT_DOMAIN,
            format::unsigned_receipt(self)?.as_bytes(),
        ))
    }

    pub(super) fn semantic_replay(&self) -> Result<RuntimeLinkageView, String> {
        verify_record(&self.record)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        verify_record(&self.record).map(|_| ())
    }
}

fn verify_record(
    record: &RecordedPreparedRuntimeObservation,
) -> Result<RuntimeLinkageView, String> {
    if record.stdout.is_empty() || record.stdout.len() > MAX_LOADER_OUTPUT_BYTES {
        return Err("prepared receipt stdout size is outside bounds".to_owned());
    }
    validate_sha256("prepared stdout", &record.stdout_sha256)?;
    if sha256(&record.stdout) != record.stdout_sha256 {
        return Err("prepared receipt stdout digest differs from its bytes".to_owned());
    }
    let replayed = parse_runtime_linkage_view(
        record.view.artifact_sha256(),
        record.view.elf_interpreter(),
        record.view.direct_needed(),
        &record.stdout,
    )?;
    if replayed != record.view {
        return Err("prepared receipt semantic replay differs from its view".to_owned());
    }
    validate_tool(record)?;
    validate_bindings(&record.bindings, &record.view)?;
    Ok(replayed)
}

fn validate_tool(record: &RecordedPreparedRuntimeObservation) -> Result<(), String> {
    validate_absolute_path(&record.bwrap_path, "receipt bubblewrap path")?;
    validate_sha256("receipt bubblewrap", &record.bwrap_sha256)?;
    if record.bwrap_byte_length == 0 || record.bwrap_byte_length > MAX_BWRAP_BYTES {
        return Err("receipt bubblewrap byte length is outside origin policy".to_owned());
    }
    validate_policy_id(&record.bwrap_executable_policy)
}

fn validate_bindings(
    bindings: &[RecordedBindingIdentity],
    view: &RuntimeLinkageView,
) -> Result<(), String> {
    if bindings.len() != view.resolved_objects().len() + 2
        || bindings.len() > MAX_RESOLVED_OBJECTS + 2
        || !bindings
            .windows(2)
            .all(|pair| pair[0].destination < pair[1].destination)
    {
        return Err("prepared receipt bindings are not an exact ordered set".to_owned());
    }
    let expected: BTreeSet<_> = view
        .resolved_objects()
        .iter()
        .map(|object| (object.soname(), object.resolved_path()))
        .collect();
    if expected
        .iter()
        .filter(|(name, _)| *name == "libc.so.6")
        .count()
        != 1
    {
        return Err("prepared receipt view lacks exactly one libc.so.6".to_owned());
    }
    let mut files = BTreeSet::new();
    let mut bytes = BTreeSet::new();
    let mut resolved = BTreeSet::new();
    let mut total = 0u64;
    let mut artifact = 0;
    let mut loader = 0;
    for binding in bindings {
        validate_object(&binding.object)?;
        total = total
            .checked_add(binding.object.byte_length)
            .ok_or_else(|| "prepared receipt binding byte accounting overflow".to_owned())?;
        if !files.insert((binding.object.device, binding.object.inode))
            || !bytes.insert((binding.object.byte_length, binding.object.sha256.as_str()))
        {
            return Err("prepared receipt repeats a source or byte identity".to_owned());
        }
        match binding.object.role {
            RecordedRuntimeRole::RootPie => {
                artifact += 1;
                if binding.destination != "/artifact"
                    || binding.mode != 0o444
                    || binding.object.soname.is_some()
                    || binding.object.sha256 != view.artifact_sha256()
                {
                    return Err(
                        "prepared receipt artifact binding differs from its view".to_owned()
                    );
                }
            }
            RecordedRuntimeRole::Loader => {
                loader += 1;
                let name = super::linux_basename(view.loader_path());
                if binding.destination != view.loader_path()
                    || binding.object.logical_path != binding.destination
                    || binding.object.soname.as_deref() != name
                    || binding.mode != 0o555
                {
                    return Err("prepared receipt loader binding differs from its view".to_owned());
                }
            }
            RecordedRuntimeRole::Libc | RecordedRuntimeRole::SharedObject => {
                let soname =
                    binding.object.soname.as_deref().ok_or_else(|| {
                        "prepared receipt runtime binding has no SONAME".to_owned()
                    })?;
                let expected_role = if soname == "libc.so.6" {
                    RecordedRuntimeRole::Libc
                } else {
                    RecordedRuntimeRole::SharedObject
                };
                if binding.object.role != expected_role
                    || binding.object.logical_path != binding.destination
                    || binding.mode != 0o444
                    || !resolved.insert((soname, binding.destination.as_str()))
                {
                    return Err("prepared receipt runtime binding differs from policy".to_owned());
                }
            }
        }
    }
    if artifact != 1 || loader != 1 || resolved != expected || total > MAX_PREPARED_SET_BYTES {
        return Err("prepared receipt binding closure differs from its view".to_owned());
    }
    Ok(())
}

fn validate_object(object: &RecordedObjectIdentity) -> Result<(), String> {
    validate_absolute_path(&object.logical_path, "receipt object path")?;
    if object.device == 0
        || object.inode == 0
        || object.byte_length == 0
        || object.byte_length > MAX_RUNTIME_OBJECT_BYTES
    {
        return Err("prepared receipt object identity is outside origin policy".to_owned());
    }
    validate_sha256("prepared receipt object", &object.sha256)?;
    if let Some(soname) = &object.soname {
        validate_soname(soname)?;
    }
    Ok(())
}

pub(super) fn role_name(role: RecordedRuntimeRole) -> &'static str {
    match role {
        RecordedRuntimeRole::RootPie => "root-pie",
        RecordedRuntimeRole::Loader => "loader",
        RecordedRuntimeRole::Libc => "libc",
        RecordedRuntimeRole::SharedObject => "shared-object",
    }
}

pub(super) fn parse_role(value: &str) -> Option<RecordedRuntimeRole> {
    match value {
        "root-pie" => Some(RecordedRuntimeRole::RootPie),
        "loader" => Some(RecordedRuntimeRole::Loader),
        "libc" => Some(RecordedRuntimeRole::Libc),
        "shared-object" => Some(RecordedRuntimeRole::SharedObject),
        _ => None,
    }
}

pub(super) fn validate_absolute_path(value: &str, label: &str) -> Result<(), String> {
    if !value.starts_with('/')
        || value.len() > 4096
        || value == "/"
        || value.contains("//")
        || value.contains('\\')
        || value.ends_with('/')
        || value
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        Err(format!("{label} must be a normalized absolute Linux path"))
    } else {
        Ok(())
    }
}

pub(super) fn validate_policy_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-._".contains(&byte)
        })
    {
        Err("prepared receipt executable policy is not canonical".to_owned())
    } else {
        Ok(())
    }
}

pub(super) fn validate_soname(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    {
        Err("prepared receipt SONAME is not canonical".to_owned())
    } else {
        Ok(())
    }
}

pub(super) fn validate_sha256(label: &str, value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!("invalid lowercase SHA-256 for {label}"))
    }
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn domain_sha256(domain: &[u8], bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update([0]);
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

pub(super) fn fixed_metadata() -> Vec<(&'static str, String)> {
    let mut values = vec![
        ("admission-result", ADMISSION_RESULT.to_owned()),
        ("authority", AUTHORITY.to_owned()),
        (
            "record-scope",
            "exact-private-prepared-loader-observation-record".to_owned(),
        ),
        ("candidate-loader-policy", LOADER_POLICY.to_owned()),
        ("hash-algorithm", "sha256".to_owned()),
        ("loader-policy", PREPARED_LOADER_POLICY.to_owned()),
        (
            "receipt-kind",
            "private-prepared-runtime-loader-observation".to_owned(),
        ),
        ("record-disposition", RECORD_DISPOSITION.to_owned()),
        (
            "semantic-replay-model",
            "provider-free-record-validation-no-execution-v1".to_owned(),
        ),
        (
            "stdout-encoding",
            "lowercase-hex-1024-byte-chunks-v1".to_owned(),
        ),
    ];
    values.extend(NONCLAIM_KEYS.map(|key| (key, NOT_ATTESTED.to_owned())));
    values
}
