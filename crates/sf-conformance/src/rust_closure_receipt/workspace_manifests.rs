use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{OriginKind, PackageRecord};

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

pub(super) struct Snapshot {
    files: Vec<(PathBuf, Vec<u8>)>,
    sha256: String,
}

impl Snapshot {
    pub(super) fn capture(root: &Path) -> Result<Self, String> {
        let mut paths = BTreeSet::from([PathBuf::from("Cargo.toml")]);
        for entry in std::fs::read_dir(root.join("crates"))
            .map_err(|error| format!("read workspace crates directory: {error}"))?
        {
            let entry = entry.map_err(|error| format!("read workspace crate entry: {error}"))?;
            let file_type = entry
                .file_type()
                .map_err(|error| format!("inspect workspace crate entry: {error}"))?;
            if file_type.is_symlink() {
                return Err("workspace crates directory contains a symlink".to_owned());
            }
            if file_type.is_dir() {
                let relative = PathBuf::from("crates")
                    .join(entry.file_name())
                    .join("Cargo.toml");
                if root.join(&relative).is_file() {
                    paths.insert(relative);
                }
            }
        }
        let mut digest = Sha256::new();
        digest.update(b"semantic-fabric-parser-worker-workspace-manifests-v1\0");
        let mut files = Vec::with_capacity(paths.len());
        for relative in paths {
            let bytes = super::authority::read(&root.join(&relative), MAX_MANIFEST_BYTES)?;
            let name = relative
                .to_str()
                .ok_or_else(|| "workspace manifest path is not UTF-8".to_owned())?;
            digest.update(name.as_bytes());
            digest.update([0]);
            digest.update((bytes.len() as u64).to_be_bytes());
            digest.update(&bytes);
            files.push((relative, bytes));
        }
        Ok(Self {
            files,
            sha256: format!("{:x}", digest.finalize()),
        })
    }

    pub(super) fn sha256(&self) -> &str {
        &self.sha256
    }

    pub(super) fn file_count(&self) -> usize {
        self.files.len()
    }

    pub(super) fn ensure_covers(&self, packages: &[PackageRecord]) -> Result<(), String> {
        let paths: BTreeSet<_> = self.files.iter().map(|(path, _)| path).collect();
        for package in packages {
            if package.origin_kind != OriginKind::Workspace {
                continue;
            }
            let path = PathBuf::from(&package.origin);
            if path.is_absolute()
                || path
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
                || !paths.contains(&path)
            {
                return Err(format!(
                    "workspace manifest snapshot does not cover {}",
                    package.origin
                ));
            }
        }
        Ok(())
    }

    pub(super) fn ensure_unchanged(&self, root: &Path) -> Result<(), String> {
        for (relative, expected) in &self.files {
            let observed = super::authority::read(&root.join(relative), MAX_MANIFEST_BYTES)?;
            if &observed != expected {
                return Err(format!(
                    "workspace manifest changed during capture: {}",
                    relative.display()
                ));
            }
        }
        Ok(())
    }
}
