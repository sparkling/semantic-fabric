//! Bounded, fail-closed filesystem inventories for source-capture inputs.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::authority;

const MAX_FILES: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct FileRecord {
    pub(super) mode: u32,
    pub(super) path: String,
}

impl FileRecord {
    fn parse(record: &str) -> Option<Self> {
        let mut fields = record.split('\t');
        if fields.next()? != "f" {
            return None;
        }
        let mode = u32::from_str_radix(fields.next()?, 8).ok()?;
        let path = fields.next()?;
        let _size = fields.next()?;
        let _sha256 = fields.next()?;
        if fields.next().is_some() {
            return None;
        }
        Some(Self {
            mode,
            path: path.to_owned(),
        })
    }
}

pub(super) struct TreeDigest {
    pub(super) sha256: String,
    pub(super) files: BTreeSet<FileRecord>,
    pub(super) directories: BTreeSet<String>,
}

pub(super) fn inventory(
    root: &Path,
    domain: &[u8],
    tracked_source: bool,
    max_bytes: u64,
) -> Result<TreeDigest, String> {
    if max_bytes == 0 {
        return Err("inventory byte bound must be non-zero".to_owned());
    }
    authority::validate_directory(root, "inventory root")?;
    let mut state = WalkState {
        tracked_source,
        records: Vec::new(),
        bytes: 0,
        entries: 0,
        directories: BTreeSet::new(),
        max_bytes,
    };
    walk(root, root, &mut state)?;
    state.records.sort();
    let mut digest = Sha256::new();
    digest.update(domain);
    for record in &state.records {
        digest.update(record.as_bytes());
        digest.update([0]);
    }
    let files = state
        .records
        .iter()
        .filter_map(|record| FileRecord::parse(record))
        .collect();
    Ok(TreeDigest {
        sha256: format!("{:x}", digest.finalize()),
        files,
        directories: state.directories,
    })
}

struct WalkState {
    tracked_source: bool,
    records: Vec<String>,
    bytes: u64,
    entries: usize,
    directories: BTreeSet<String>,
    max_bytes: u64,
}

fn walk(root: &Path, directory: &Path, state: &mut WalkState) -> Result<(), String> {
    let mut children: Vec<_> = fs::read_dir(directory)
        .map_err(|error| format!("read inventory directory {}: {error}", directory.display()))?
        .collect::<Result<_, _>>()
        .map_err(|error| format!("enumerate inventory directory: {error}"))?;
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        state.entries = state
            .entries
            .checked_add(1)
            .ok_or_else(|| "inventory entry count overflow".to_owned())?;
        if state.entries > MAX_FILES {
            return Err(format!("inventory exceeds {MAX_FILES} entries"));
        }
        let path = entry.path();
        authority::validate_beneath(root, &path, "inventory entry")?;
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "inventory path escaped root".to_owned())?
            .to_str()
            .ok_or_else(|| "inventory path is not UTF-8".to_owned())?;
        validate_relative(relative)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("inspect inventory entry: {error}"))?;
        if metadata.is_dir() {
            state.directories.insert(relative.to_owned());
            if !state.tracked_source {
                state
                    .records
                    .push(format!("d\t{:o}\t{relative}", mode(&metadata)?));
            }
            walk(root, &path, state)?;
        } else {
            let (sha, size) = authority::digest(&path, state.max_bytes, "inventory file")?;
            state.bytes = state
                .bytes
                .checked_add(size)
                .ok_or_else(|| "inventory byte count overflow".to_owned())?;
            if state.bytes > state.max_bytes {
                return Err(format!("inventory exceeds {} bytes", state.max_bytes));
            }
            state.records.push(format!(
                "f\t{:o}\t{relative}\t{size}\t{sha}",
                mode(&metadata)?
            ));
        }
    }
    Ok(())
}

fn mode(metadata: &fs::Metadata) -> Result<u32, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(metadata.mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        Err("source inventory requires Unix mode bits".to_owned())
    }
}

fn validate_relative(path: &str) -> Result<(), String> {
    let value = Path::new(path);
    if value.is_absolute()
        || path.is_empty()
        || path.bytes().any(|byte| byte.is_ascii_control())
        || value
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        Err("unsafe inventory path".to_owned())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_files_are_parsed_but_directories_are_not() {
        assert_eq!(
            FileRecord::parse("f\t755\tbin/app\t3\tabc").unwrap(),
            FileRecord {
                mode: 0o755,
                path: "bin/app".to_owned(),
            }
        );
        assert!(FileRecord::parse("d\t755\tbin").is_none());
    }

    #[test]
    fn rejects_an_empty_aggregate_byte_bound() {
        assert!(inventory(Path::new("/does-not-matter"), b"domain\0", false, 0).is_err());
    }
}
