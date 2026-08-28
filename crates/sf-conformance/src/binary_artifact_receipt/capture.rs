//! Fresh-target build-script output inventory.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{authority, cargo};

const MAX_BUILD_SCRIPT_FILES: usize = 20_000;
const MAX_BUILD_SCRIPT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SCRIPT_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;
pub(super) const DIRECTIVES_FORMAT: &str = "semantic-fabric-canonical-build-script-directives-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BuildScriptInventory {
    pub(super) package_id: String,
    pub(super) out_dir: PathBuf,
    pub(super) directives_sha256: String,
    pub(super) directives_bytes: u64,
    pub(super) stderr_sha256: String,
    pub(super) stderr_bytes: u64,
    pub(super) out_tree_sha256: String,
    pub(super) out_file_count: usize,
    pub(super) out_bytes: u64,
}

pub(super) fn inventory(
    target_dir: &Path,
    scripts: &[cargo::BuildScript],
) -> Result<Vec<BuildScriptInventory>, String> {
    authority::validate_directory(target_dir, "fresh target directory")?;
    let mut inventories = Vec::with_capacity(scripts.len());
    for script in scripts {
        inventories.push(one(target_dir, script)?);
    }
    inventories.sort_by(|left, right| {
        left.package_id
            .cmp(&right.package_id)
            .then(left.out_dir.cmp(&right.out_dir))
    });
    if inventories
        .windows(2)
        .any(|pair| pair[0].package_id == pair[1].package_id && pair[0].out_dir == pair[1].out_dir)
    {
        return Err("duplicate build-script inventory".to_owned());
    }
    Ok(inventories)
}

fn one(target_dir: &Path, script: &cargo::BuildScript) -> Result<BuildScriptInventory, String> {
    authority::validate_beneath(target_dir, &script.out_dir, "build-script OUT_DIR")?;
    authority::validate_directory(&script.out_dir, "build-script OUT_DIR")?;
    let build_dir = script
        .out_dir
        .parent()
        .ok_or_else(|| "build-script OUT_DIR has no parent".to_owned())?;
    authority::validate_beneath(target_dir, build_dir, "build-script directory")?;
    let directives = authority::read(
        &build_dir.join("output"),
        MAX_SCRIPT_OUTPUT_BYTES,
        "build-script directives",
    )?;
    let stderr = authority::read(
        &build_dir.join("stderr"),
        MAX_SCRIPT_OUTPUT_BYTES,
        "build-script stderr",
    )?;
    let tree = tree(&script.out_dir)?;
    Ok(BuildScriptInventory {
        package_id: checked_package_id(&script.package_id)?,
        out_dir: script.out_dir.clone(),
        directives_sha256: canonical_directives_sha256(&directives.bytes)?,
        directives_bytes: directives.size,
        stderr_sha256: stderr.sha256,
        stderr_bytes: stderr.size,
        out_tree_sha256: tree.sha256,
        out_file_count: tree.file_count,
        out_bytes: tree.bytes,
    })
}

fn canonical_directives_sha256(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("build-script directives are not UTF-8: {error}"))?;
    if text.bytes().any(|byte| {
        byte == 0 || (byte.is_ascii_control() && !matches!(byte, b'\n' | b'\r' | b'\t'))
    }) {
        return Err("build-script directives contain an invalid control byte".to_owned());
    }
    let canonical = text.replace("\r\n", "\n");
    if canonical.contains('\r') {
        return Err("build-script directives contain a bare carriage return".to_owned());
    }
    let mut digest = Sha256::new();
    digest.update(DIRECTIVES_FORMAT.as_bytes());
    digest.update([0]);
    digest.update(canonical.as_bytes());
    if !canonical.ends_with('\n') {
        digest.update([b'\n']);
    }
    Ok(format!("{:x}", digest.finalize()))
}

struct Tree {
    sha256: String,
    file_count: usize,
    bytes: u64,
}

fn tree(root: &Path) -> Result<Tree, String> {
    let mut entries = Vec::new();
    let mut nodes = 0usize;
    visit(root, root, &mut entries, &mut nodes)?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest = Sha256::new();
    digest.update(b"semantic-fabric-build-script-out-tree-v1\0");
    let mut bytes = 0u64;
    for (relative, sha256, size) in &entries {
        bytes = bytes
            .checked_add(*size)
            .ok_or_else(|| "build-script OUT_DIR byte count overflow".to_owned())?;
        if bytes > MAX_BUILD_SCRIPT_BYTES {
            return Err(format!(
                "build-script OUT_DIR exceeds {MAX_BUILD_SCRIPT_BYTES} bytes"
            ));
        }
        digest.update(relative.as_bytes());
        digest.update([0]);
        digest.update(sha256.as_bytes());
        digest.update([0]);
        digest.update(size.to_string().as_bytes());
        digest.update([0]);
    }
    Ok(Tree {
        sha256: format!("{:x}", digest.finalize()),
        file_count: entries.len(),
        bytes,
    })
}

fn visit(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<(String, String, u64)>,
    nodes: &mut usize,
) -> Result<(), String> {
    *nodes = nodes
        .checked_add(1)
        .ok_or_else(|| "build-script OUT_DIR entry count overflow".to_owned())?;
    if *nodes > MAX_BUILD_SCRIPT_FILES {
        return Err(format!(
            "build-script OUT_DIR exceeds {MAX_BUILD_SCRIPT_FILES} entries"
        ));
    }
    if directory != root {
        authority::validate_beneath(root, directory, "build-script OUT_DIR entry")?;
    }
    authority::validate_directory(directory, "build-script OUT_DIR entry")?;
    let mut children: Vec<_> = fs::read_dir(directory)
        .map_err(|error| format!("read build-script OUT_DIR {}: {error}", directory.display()))?
        .collect::<Result<_, _>>()
        .map_err(|error| {
            format!(
                "enumerate build-script OUT_DIR {}: {error}",
                directory.display()
            )
        })?;
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        if entries.len() == MAX_BUILD_SCRIPT_FILES {
            return Err(format!(
                "build-script OUT_DIR exceeds {MAX_BUILD_SCRIPT_FILES} files"
            ));
        }
        let path = entry.path();
        authority::validate_beneath(root, &path, "build-script OUT_DIR entry")?;
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "inspect build-script OUT_DIR entry {}: {error}",
                path.display()
            )
        })?;
        if metadata.is_dir() {
            visit(root, &path, entries, nodes)?;
        } else {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "build-script OUT_DIR entry escaped its root".to_owned())?
                .to_str()
                .ok_or_else(|| "build-script OUT_DIR entry path is not UTF-8".to_owned())?
                .to_owned();
            let (sha256, size) =
                authority::digest(&path, MAX_BUILD_SCRIPT_BYTES, "build-script OUT_DIR file")?;
            entries.push((relative, sha256, size));
        }
    }
    Ok(())
}

fn checked_package_id(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > 16 * 1024
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        Err("invalid Cargo build-script package ID".to_owned())
    } else {
        Ok(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_digest_is_stable_over_directory_order() {
        let root =
            std::env::temp_dir().join(format!("semantic-fabric-out-tree-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("z"), b"z").unwrap();
        fs::write(root.join("a"), b"a").unwrap();
        let first = tree(&root).unwrap();
        let second = tree(&root).unwrap();
        assert_eq!(first.sha256, second.sha256);
        assert_eq!(first.file_count, 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directives_digest_normalizes_line_endings() {
        assert_eq!(
            canonical_directives_sha256(b"cargo:rustc-cfg=x\r\n").unwrap(),
            canonical_directives_sha256(b"cargo:rustc-cfg=x\n").unwrap()
        );
        assert!(canonical_directives_sha256(b"cargo:x\0").is_err());
    }
}
