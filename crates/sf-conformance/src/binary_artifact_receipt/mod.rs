//! Canonical M0 observation of the current broad `sf-cli` binary.
//!
//! This schema deliberately records one builder observation. It is not a
//! production, release, admission, minimality, or reproducibility attestation.

mod format;
mod model;
#[cfg(test)]
mod tests;

pub use format::{parse, render, MAX_RECEIPT_BYTES};
pub use model::{
    ArtifactObservation, BuildScriptEvent, HostObservation, LinkInput, LinkInputOrigin,
    PortableAuthority, Receipt, ToolIdentity, ToolRole,
};
