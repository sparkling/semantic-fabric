//! Opaque holder and private one-shot executor for one discovered runtime set.
//!
//! Immutable snapshot bytes are sealed in memfds copied from descriptor-rooted
//! sources. The private executor transfers read-only copies from those memfds
//! into a fresh bubblewrap tmpfs and checks that the loader view is unchanged.
//! It does not expose descriptors. A finished observation can be converted into
//! a private canonical self-consistency record, but that mapping makes no direct-
//! memfd, execution-provenance, admission, performance, minimality, or release
//! claim and cannot recreate live authority.
//! Discovery necessarily precedes the holder and is not authorized by it.
//! Same-principal/root ABA, rollback, and hostile-kernel or hostile-filesystem
//! resistance remain explicitly out of scope.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::path::Path;

use super::{
    validate_plan, RuntimeLinkageView, RuntimeLoaderPlan, MAX_RESOLVED_OBJECTS,
    MAX_RUNTIME_OBJECT_BYTES,
};
use crate::binary_artifact_receipt::{
    artifact_pair::ArtifactPair,
    runtime_elf::{
        parse_runtime_elf, runtime_elf_policy_identity, RuntimeElfPolicyIdentity, RuntimeElfRole,
        RuntimeElfView,
    },
};

mod linux;
mod prepared_probe;
#[cfg(test)]
mod tests;

const MAX_RUNTIME_SET_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeObjectIdentity {
    pub(super) logical_path: String,
    pub(super) role: RuntimeElfRole,
    pub(super) soname: Option<String>,
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) byte_length: u64,
    pub(super) sha256: String,
}

#[derive(Debug)]
enum HeldSource {
    Artifact {
        file: File,
        identity: linux::FileIdentity,
    },
    Runtime(linux::ResolvedSource),
}

#[derive(Debug)]
struct HeldRuntimeObject {
    identity: RuntimeObjectIdentity,
    source: HeldSource,
    sealed: linux::SealedBytes,
    elf: RuntimeElfView,
}

#[derive(Debug)]
pub(super) struct HeldRuntimeInputs<'a> {
    artifact_pair: &'a ArtifactPair,
    bwrap_path: std::path::PathBuf,
    candidate: RuntimeLinkageView,
    roots: Vec<linux::HeldMount>,
    artifact: HeldRuntimeObject,
    loader: HeldRuntimeObject,
    objects: Vec<HeldRuntimeObject>,
    identities: Vec<RuntimeObjectIdentity>,
    runtime_elf_policy: RuntimeElfPolicyIdentity,
    total_bytes: u64,
    effective_uid: u32,
}

pub(super) fn hold_runtime_inputs<'a>(
    artifact: &'a ArtifactPair,
    plan: &RuntimeLoaderPlan,
    discovered: &RuntimeLinkageView,
) -> Result<HeldRuntimeInputs<'a>, String> {
    validate_bindings(artifact, plan, discovered)?;
    let roots = linux::hold_mounts(&plan.mounts)?;
    let mut total_bytes = 0u64;

    let artifact_file = artifact.duplicate_selected()?;
    let artifact_source = linux::inspect_artifact(&artifact_file)?;
    reserve_bytes(&mut total_bytes, artifact_source.size)?;
    let artifact_sealed = linux::snapshot_source(
        &artifact_file,
        artifact_source,
        MAX_RUNTIME_OBJECT_BYTES,
        "root artifact",
    )?;
    if artifact_sealed.sha256 != discovered.artifact_sha256() {
        return Err("sealed root artifact digest differs from linkage discovery".to_owned());
    }
    let artifact_elf = parse_runtime_elf(&artifact_sealed.bytes, RuntimeElfRole::RootPie)?;
    validate_root_elf(&artifact_elf, discovered)?;
    let artifact_object = held_object(
        plan.artifact
            .to_str()
            .ok_or_else(|| "artifact path is not UTF-8".to_owned())?,
        RuntimeElfRole::RootPie,
        None,
        HeldSource::Artifact {
            file: artifact_file,
            identity: artifact_source,
        },
        artifact_source,
        artifact_sealed,
        artifact_elf,
    );

    let loader_source = linux::resolve_source(&roots, discovered.loader_path(), true)?;
    reserve_bytes(&mut total_bytes, loader_source.identity.size)?;
    let loader_sealed = linux::snapshot_source(
        &loader_source.file,
        loader_source.identity,
        MAX_RUNTIME_OBJECT_BYTES,
        "runtime loader",
    )?;
    let loader_elf = parse_runtime_elf(&loader_sealed.bytes, RuntimeElfRole::Loader)?;
    let loader_name = Path::new(discovered.loader_path())
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "runtime loader path has no UTF-8 file name".to_owned())?;
    if loader_elf.soname() != Some(loader_name) || !loader_elf.needed().is_empty() {
        return Err(
            "sealed loader identity or dependency policy differs from discovery".to_owned(),
        );
    }
    let loader_identity = loader_source.identity;
    let loader_object = held_object(
        discovered.loader_path(),
        RuntimeElfRole::Loader,
        Some(loader_name),
        HeldSource::Runtime(loader_source),
        loader_identity,
        loader_sealed,
        loader_elf,
    );

    let mut candidates = discovered.resolved_objects().to_vec();
    candidates.sort_by(|left, right| left.soname().cmp(right.soname()));
    if candidates.len() > MAX_RESOLVED_OBJECTS {
        return Err("runtime object count exceeds the held-set budget".to_owned());
    }
    let mut names = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let mut objects = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if !names.insert(candidate.soname().to_owned())
            || !paths.insert(candidate.resolved_path().to_owned())
            || candidate.soname() == loader_name
            || Path::new(candidate.resolved_path())
                .file_name()
                .and_then(|name| name.to_str())
                != Some(candidate.soname())
        {
            return Err("runtime object name or path is ambiguous".to_owned());
        }
        let role = if candidate.soname() == "libc.so.6" {
            RuntimeElfRole::Libc
        } else {
            RuntimeElfRole::SharedObject
        };
        let source = linux::resolve_source(&roots, candidate.resolved_path(), false)?;
        reserve_bytes(&mut total_bytes, source.identity.size)?;
        let sealed = linux::snapshot_source(
            &source.file,
            source.identity,
            MAX_RUNTIME_OBJECT_BYTES,
            candidate.soname(),
        )?;
        let elf = parse_runtime_elf(&sealed.bytes, role)?;
        if elf.soname() != Some(candidate.soname()) {
            return Err(format!(
                "sealed runtime object SONAME differs from discovery: {}",
                candidate.soname()
            ));
        }
        let source_identity = source.identity;
        objects.push(held_object(
            candidate.resolved_path(),
            role,
            Some(candidate.soname()),
            HeldSource::Runtime(source),
            source_identity,
            sealed,
            elf,
        ));
    }
    if !names.contains("libc.so.6") {
        return Err("held runtime closure does not contain exactly one libc.so.6".to_owned());
    }

    validate_unique_authorities(
        std::iter::once(&artifact_object.identity)
            .chain(std::iter::once(&loader_object.identity))
            .chain(objects.iter().map(|object| &object.identity)),
    )?;
    validate_static_needed_graph(&artifact_object, &loader_object, &objects)?;
    let runtime_elf_policy = runtime_elf_policy_identity();
    let identities = std::iter::once(&artifact_object)
        .chain(std::iter::once(&loader_object))
        .chain(objects.iter())
        .map(|object| object.identity.clone())
        .collect();
    let held = HeldRuntimeInputs {
        artifact_pair: artifact,
        bwrap_path: plan.executable.clone(),
        candidate: discovered.clone(),
        roots,
        artifact: artifact_object,
        loader: loader_object,
        objects,
        identities,
        runtime_elf_policy,
        total_bytes,
        effective_uid: effective_uid(),
    };
    held.assert_current()?;
    Ok(held)
}

fn observe_bwrap_host_resolution(
    expected_bwrap: prepared_probe::ExpectedBwrapIdentity,
    expected_runtime_elf_policy: prepared_probe::ExpectedRuntimeElfPolicy,
) -> Result<prepared_probe::BwrapHostResolutionObservation, String> {
    prepared_probe::observe_bwrap_host_resolution(expected_bwrap, expected_runtime_elf_policy)
}

impl HeldRuntimeInputs<'_> {
    pub(super) fn identities(&self) -> &[RuntimeObjectIdentity] {
        &self.identities
    }

    pub(super) fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub(super) fn runtime_elf_policy(&self) -> &RuntimeElfPolicyIdentity {
        &self.runtime_elf_policy
    }

    pub(super) fn assert_current(&self) -> Result<(), String> {
        self.assert_current_with_phase_hook(|| {})
    }

    fn execute_prepared(
        self,
        expected_bwrap: prepared_probe::ExpectedBwrapIdentity,
        expected_runtime_elf_policy: prepared_probe::ExpectedRuntimeElfPolicy,
        expected_seccomp_policy: prepared_probe::ExpectedPreparedSeccompPolicy,
    ) -> Result<prepared_probe::PreparedRuntimeObservation, String> {
        prepared_probe::execute(
            self,
            expected_bwrap,
            expected_runtime_elf_policy,
            expected_seccomp_policy,
        )
    }

    fn assert_current_with_phase_hook(
        &self,
        before_final_fence: impl FnOnce(),
    ) -> Result<(), String> {
        if effective_uid() != self.effective_uid {
            return Err("effective UID changed during runtime snapshot".to_owned());
        }
        self.artifact_pair.assert_current()?;
        linux::assert_mounts_current(&self.roots)?;
        for object in std::iter::once(&self.artifact)
            .chain(std::iter::once(&self.loader))
            .chain(self.objects.iter())
        {
            linux::assert_sealed_current(&object.sealed)?;
            match &object.source {
                HeldSource::Artifact { file, identity } => {
                    linux::assert_plain_source_current(file, *identity, &object.identity.sha256)?;
                }
                HeldSource::Runtime(source) => {
                    linux::assert_source_current(source, &self.roots, &object.identity.sha256)?;
                }
            }
        }
        before_final_fence();
        linux::assert_mounts_current(&self.roots)?;
        self.artifact_pair.assert_current()?;
        if effective_uid() != self.effective_uid {
            return Err("effective UID changed during runtime snapshot".to_owned());
        }
        Ok(())
    }
}

fn held_object(
    logical_path: &str,
    role: RuntimeElfRole,
    soname: Option<&str>,
    source: HeldSource,
    source_identity: linux::FileIdentity,
    sealed: linux::SealedBytes,
    elf: RuntimeElfView,
) -> HeldRuntimeObject {
    HeldRuntimeObject {
        identity: RuntimeObjectIdentity {
            logical_path: logical_path.to_owned(),
            role,
            soname: soname.map(str::to_owned),
            device: source_identity.device,
            inode: source_identity.inode,
            byte_length: sealed.byte_length,
            sha256: sealed.sha256.clone(),
        },
        source,
        sealed,
        elf,
    }
}

fn validate_bindings(
    artifact: &ArtifactPair,
    plan: &RuntimeLoaderPlan,
    discovered: &RuntimeLinkageView,
) -> Result<(), String> {
    validate_plan(plan)?;
    artifact.assert_current()?;
    if plan.artifact != artifact.selected_path()
        || plan.interpreter != discovered.elf_interpreter()
        || plan.interpreter != discovered.loader_path()
        || artifact.sha256() != discovered.artifact_sha256()
    {
        return Err("artifact, plan, and runtime discovery are not exactly bound".to_owned());
    }
    Ok(())
}

fn validate_root_elf(root: &RuntimeElfView, discovered: &RuntimeLinkageView) -> Result<(), String> {
    let mut needed = root.needed().to_vec();
    needed.sort();
    if root.interpreter() != Some(discovered.elf_interpreter())
        || needed != discovered.direct_needed()
    {
        return Err("sealed root interpreter or DT_NEEDED set differs from discovery".to_owned());
    }
    Ok(())
}

fn reserve_bytes(total: &mut u64, size: u64) -> Result<(), String> {
    if size == 0 || size > MAX_RUNTIME_OBJECT_BYTES {
        return Err("runtime object size is outside the per-object budget".to_owned());
    }
    let next = total
        .checked_add(size)
        .ok_or_else(|| "runtime set byte accounting overflow".to_owned())?;
    if next > MAX_RUNTIME_SET_BYTES {
        return Err("runtime set exceeds the aggregate byte budget".to_owned());
    }
    *total = next;
    Ok(())
}

fn validate_unique_authorities<'a>(
    identities: impl IntoIterator<Item = &'a RuntimeObjectIdentity>,
) -> Result<(), String> {
    let mut files = BTreeSet::new();
    let mut bytes = BTreeSet::new();
    for identity in identities {
        if identity.device == 0
            || identity.inode == 0
            || !files.insert((identity.device, identity.inode))
            || !bytes.insert((identity.byte_length, identity.sha256.as_str()))
        {
            return Err("runtime set contains a duplicate source or byte identity".to_owned());
        }
    }
    Ok(())
}

fn validate_static_needed_graph(
    artifact: &HeldRuntimeObject,
    loader: &HeldRuntimeObject,
    objects: &[HeldRuntimeObject],
) -> Result<(), String> {
    let loader_name = loader
        .identity
        .soname
        .as_deref()
        .ok_or_else(|| "held loader has no SONAME".to_owned())?;
    let mut providers = BTreeMap::new();
    if providers.insert(loader_name, loader).is_some() {
        return Err("duplicate loader provider".to_owned());
    }
    for object in objects {
        let soname = object
            .identity
            .soname
            .as_deref()
            .ok_or_else(|| "held runtime object has no SONAME".to_owned())?;
        if providers.insert(soname, object).is_some() {
            return Err("runtime graph has an ambiguous provider".to_owned());
        }
    }
    let mut queue: VecDeque<&str> = artifact.elf.needed().iter().map(String::as_str).collect();
    queue.push_back(loader_name);
    let mut reached = BTreeSet::new();
    while let Some(name) = queue.pop_front() {
        if !reached.insert(name) {
            continue;
        }
        let provider = providers
            .get(name)
            .ok_or_else(|| format!("runtime graph has no provider for {name}"))?;
        queue.extend(provider.elf.needed().iter().map(String::as_str));
    }
    if reached.len() != providers.len() {
        return Err("static DT_NEEDED graph contains an unreachable extra provider".to_owned());
    }
    Ok(())
}

fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and dereferences no memory.
    unsafe { libc::geteuid() }
}
