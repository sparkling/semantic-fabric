//! Fail-closed manifests for the locally supported SPARQL Query and Protocol surface.

mod format;
mod model;

pub use format::{manifest_sha256, parse_manifest, render_manifest};
pub use model::{Case, ExpectedStatus, Manifest, ManifestSeal, StandardReference, Surface};

/// Build a seal for tests or a deliberate manifest revision.
pub fn seal_for(text: &str, manifest: &Manifest) -> ManifestSeal {
    ManifestSeal {
        profile_id: manifest.profile_id.clone(),
        surface: manifest.surface,
        case_count: manifest.cases.len(),
        manifest_sha256: manifest_sha256(text.as_bytes()),
    }
}

/// Require the complete canonical manifest to match a compiled profile seal.
pub fn verify_seal(text: &str, manifest: &Manifest, seal: &ManifestSeal) -> Result<(), String> {
    if manifest.profile_id != seal.profile_id
        || manifest.surface != seal.surface
        || manifest.cases.len() != seal.case_count
        || render_manifest(manifest) != text
        || manifest_sha256(text.as_bytes()) != seal.manifest_sha256
    {
        Err("supported-surface manifest does not match its fixed seal".to_owned())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
