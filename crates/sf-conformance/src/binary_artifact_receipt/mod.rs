//! Canonical M0 observation of the current broad `sf-cli` binary.
//!
//! This schema deliberately records one builder observation. It is not a
//! production, release, admission, minimality, or reproducibility attestation.

mod authority;
mod capture;
mod cargo;
mod elf;
mod format;
mod linker;
mod model;
mod process;
mod producer;
mod producer_paths;
mod receipt_file;
mod sandbox;
mod sandbox_environment;
mod source;
mod source_blobs;
mod source_tree;
#[cfg(test)]
mod tests;

pub use format::{parse, render, MAX_RECEIPT_BYTES};
pub use model::{
    ArtifactObservation, BuildScriptEvent, HostObservation, LinkInput, LinkInputOrigin,
    PortableAuthority, Receipt, ToolIdentity, ToolRole,
};
pub use producer::{capture, CaptureRequest};
pub use receipt_file::{load_external, write_new_external};
