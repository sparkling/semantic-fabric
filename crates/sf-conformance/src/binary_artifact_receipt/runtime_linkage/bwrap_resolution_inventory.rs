//! Provider-free replay of one private bubblewrap host name-resolution inventory.
//!
//! The inventory binds exact held bubblewrap identity metadata and bounded raw
//! output from a deliberately counterfactual loader invocation. The invoked
//! interpreter and reported DSOs are path-resolved, unheld, and undigested. A
//! successful replay therefore establishes only internal record consistency,
//! never actual bubblewrap execution, runtime closure, provenance, or admission.

use std::fmt;

use sha2::{Digest, Sha256};

use super::{
    parse_runtime_linkage_view, ResolvedRuntimeObject, RuntimeLinkageView, VirtualRuntimeObject,
    MAX_BWRAP_BYTES, MAX_LOADER_OUTPUT_BYTES,
};

mod format;
#[cfg(test)]
mod tests;

pub(super) const HEADER: &str =
    "semantic-fabric-bubblewrap-host-resolution-inventory-non-authority-v1";
pub(super) const AUTHORITY: &str = "none";
pub(super) const NOT_ATTESTED: &str = "not-attested";
pub(super) const RESOLUTION_RELATION: &str =
    "counterfactual-controlled-name-resolution-not-actual-exec";
pub(super) const BWRAP_HOST_RESOLUTION_POLICY: &str =
    "glibc-ldso-list-env-clear-lc-all-c-inhibit-cache-empty-hwcaps-bwrap-path-pre-post-fenced-v1";
pub(super) const STDOUT_CHUNK_BYTES: usize = 1024;
pub(super) const INVENTORY_DOMAIN: &[u8] =
    b"semantic-fabric:bubblewrap-host-resolution-inventory:v1";

pub(super) const NONCLAIM_KEYS: [&str; 33] = [
    "actual-bubblewrap-execution",
    "aggregate-cgroup-containment",
    "bubblewrap-host-runtime-closure",
    "default-cache-hwcaps-equivalence",
    "dlopen-closure",
    "dso-byte-identity",
    "external-witness",
    "final-child-fd-inventory",
    "hostile-filesystem-resistance",
    "hostile-kernel-resistance",
    "initializer-execution",
    "interpreter-byte-identity",
    "loader-output-origin",
    "lsm-state",
    "minimality",
    "nss-closure",
    "performance-authority",
    "preload-state",
    "production-admission",
    "provenance",
    "release-readiness",
    "replay-execution",
    "reproducibility",
    "resolution-target-exact-byte-consumption",
    "rollback-resistance",
    "root-race-resistance",
    "same-principal-race-resistance",
    "sbom",
    "signing",
    "target-seccomp-or-syscall-trace",
    "time-of-use",
    "transitive-static-dependency-completeness",
    "vdso-byte-closure",
];

/// Resolution names and paths without the prepared loader policy accessor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BwrapHostResolutionView(RuntimeLinkageView);

impl BwrapHostResolutionView {
    pub(super) fn from_runtime(view: RuntimeLinkageView) -> Self {
        Self(view)
    }

    pub(super) fn bwrap_sha256(&self) -> &str {
        self.0.artifact_sha256()
    }

    pub(super) fn elf_interpreter(&self) -> &str {
        self.0.elf_interpreter()
    }

    pub(super) fn direct_needed(&self) -> &[String] {
        self.0.direct_needed()
    }

    pub(super) fn loader_path(&self) -> &str {
        self.0.loader_path()
    }

    pub(super) fn resolved_objects(&self) -> &[ResolvedRuntimeObject] {
        self.0.resolved_objects()
    }

    pub(super) fn virtual_objects(&self) -> &[VirtualRuntimeObject] {
        self.0.virtual_objects()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordedBwrapHostResolution {
    pub(super) bwrap_path: String,
    pub(super) bwrap_sha256: String,
    pub(super) bwrap_byte_length: u64,
    pub(super) bwrap_executable_policy: String,
    pub(super) runtime_elf_policy: String,
    pub(super) runtime_elf_policy_sha256: String,
    pub(super) view: BwrapHostResolutionView,
    pub(super) stdout: Vec<u8>,
    pub(super) stdout_sha256: String,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct BwrapHostResolutionInventory {
    record: RecordedBwrapHostResolution,
}

impl fmt::Debug for BwrapHostResolutionInventory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BwrapHostResolutionInventory")
            .field("authority", &AUTHORITY)
            .field("relation", &RESOLUTION_RELATION)
            .field("bwrap_sha256", &self.record.bwrap_sha256)
            .field(
                "resolved_objects",
                &self.record.view.resolved_objects().len(),
            )
            .finish_non_exhaustive()
    }
}

impl BwrapHostResolutionInventory {
    pub(super) fn from_recorded(record: RecordedBwrapHostResolution) -> Result<Self, String> {
        let inventory = Self { record };
        inventory.validate()?;
        format::render(&inventory)?;
        Ok(inventory)
    }

    pub(super) fn view(&self) -> &BwrapHostResolutionView {
        &self.record.view
    }

    pub(super) fn semantic_replay(&self) -> Result<BwrapHostResolutionView, String> {
        verify_record(&self.record)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        verify_record(&self.record).map(|_| ())
    }

    pub(super) fn inventory_sha256(&self) -> Result<String, String> {
        self.validate()?;
        Ok(domain_sha256(
            INVENTORY_DOMAIN,
            format::unsigned_inventory(self)?.as_bytes(),
        ))
    }
}

pub(super) fn parse(input: &str) -> Result<BwrapHostResolutionInventory, String> {
    format::parse(input)
}

pub(super) fn render(inventory: &BwrapHostResolutionInventory) -> Result<String, String> {
    format::render(inventory)
}

fn verify_record(record: &RecordedBwrapHostResolution) -> Result<BwrapHostResolutionView, String> {
    validate_absolute_path(&record.bwrap_path, "inventory bubblewrap path")?;
    validate_sha256("inventory bubblewrap", &record.bwrap_sha256)?;
    if record.bwrap_byte_length == 0 || record.bwrap_byte_length > MAX_BWRAP_BYTES {
        return Err("inventory bubblewrap byte length is outside policy".to_owned());
    }
    validate_policy_id(
        &record.bwrap_executable_policy,
        "bubblewrap executable policy",
    )?;
    validate_policy_id(&record.runtime_elf_policy, "runtime ELF policy")?;
    validate_sha256(
        "inventory runtime ELF policy",
        &record.runtime_elf_policy_sha256,
    )?;
    if record.view.bwrap_sha256() != record.bwrap_sha256 {
        return Err("inventory view differs from bubblewrap byte identity".to_owned());
    }
    if record.stdout.is_empty() || record.stdout.len() > MAX_LOADER_OUTPUT_BYTES {
        return Err("inventory stdout size is outside bounds".to_owned());
    }
    validate_sha256("inventory stdout", &record.stdout_sha256)?;
    if sha256(&record.stdout) != record.stdout_sha256 {
        return Err("inventory stdout digest differs from its bytes".to_owned());
    }
    let replayed = parse_runtime_linkage_view(
        &record.bwrap_sha256,
        record.view.elf_interpreter(),
        record.view.direct_needed(),
        &record.stdout,
    )?;
    if replayed != record.view.0 {
        return Err("inventory semantic replay differs from its view".to_owned());
    }
    Ok(BwrapHostResolutionView::from_runtime(replayed))
}

pub(super) fn fixed_metadata() -> Vec<(&'static str, &'static str)> {
    let mut values = vec![
        ("authority", AUTHORITY),
        ("hash-algorithm", "sha256"),
        (
            "inventory-kind",
            "private-bubblewrap-host-resolution-name-path-inventory",
        ),
        ("resolution-policy", BWRAP_HOST_RESOLUTION_POLICY),
        ("resolution-relation", RESOLUTION_RELATION),
        ("runtime-role", "root-pie"),
        (
            "semantic-replay-model",
            "provider-free-record-validation-no-execution-v1",
        ),
        ("stdout-encoding", "lowercase-hex-1024-byte-chunks-v1"),
    ];
    values.extend(NONCLAIM_KEYS.map(|key| (key, NOT_ATTESTED)));
    values
}

pub(super) fn validate_absolute_path(value: &str, label: &str) -> Result<(), String> {
    if !value.starts_with('/')
        || value == "/"
        || value.len() > 4096
        || value.ends_with('/')
        || value.contains("//")
        || value.contains('\\')
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

pub(super) fn validate_policy_id(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-._".contains(&byte)
        })
    {
        Err(format!("{label} is not canonical"))
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
