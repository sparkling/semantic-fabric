//! Canonical M0 observation of the current broad `sf-cli` binary.
//!
//! This schema deliberately records one builder observation. It is not a
//! production, release, admission, minimality, or reproducibility attestation.

mod artifact_pair;
mod authority;
mod authority_guard;
mod capture;
mod cargo;
mod elf;
mod format;
mod host_link_authority;
mod linker;
mod model;
mod process;
mod producer;
mod producer_paths;
mod receipt_file;
mod runtime_elf;
mod runtime_linkage;
mod sandbox;
mod sandbox_environment;
mod source;
mod source_blobs;
mod source_tree;
#[cfg(test)]
mod tests;

pub use format::{parse, render, MAX_RECEIPT_BYTES};
pub use model::{
    ArtifactObservation, BuildScriptEvent, HostObservation, LinkInput, LinkInputAlias,
    LinkInputOrigin, PortableAuthority, Receipt, ToolIdentity, ToolRole,
};
pub use producer::{capture, CaptureRequest};
pub use receipt_file::{load_external, write_new_external};
pub use runtime_elf::{
    parse_runtime_elf, runtime_elf_policy_sha256, RuntimeElfRole, RuntimeElfView,
    RUNTIME_ELF_POLICY,
};
pub use runtime_linkage::{
    parse_runtime_linkage_view, plan_runtime_linkage, ResolvedRuntimeObject, RuntimeLinkageView,
    RuntimeLoaderPlan, VirtualRuntimeObject,
};
