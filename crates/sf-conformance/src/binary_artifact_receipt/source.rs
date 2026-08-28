//! Clean Git authority, materialization, and controlled Cargo-input digests.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::{authority, process, source_blobs, source_tree};

const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_GIT_OUTPUT: u64 = 64 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_CARGO_HOME_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const SOURCE_DOMAIN: &[u8] = b"semantic-fabric-source-inputs-v1\0";
const CARGO_HOME_DOMAIN: &[u8] = b"semantic-fabric-cargo-home-inputs-v1\0";
const EMPTY_CONFIG_DOMAIN: &[u8] = b"semantic-fabric-empty-cargo-config-set-v1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub(super) revision: String,
    pub(super) source_date_epoch: u64,
    pub(super) source_inputs_sha256: String,
    pub(super) materialized_root: PathBuf,
    git_sha256: String,
}

pub(super) fn materialize(
    git: &Path,
    repository: &Path,
    destination: &Path,
) -> Result<Snapshot, String> {
    let git_sha256 = validate_inputs(git, repository, destination)?;
    let before = clean_state(git, repository)?;
    checkout_index(git, repository, destination)?;
    let materialized = source_tree::inventory(destination, SOURCE_DOMAIN, true, MAX_SOURCE_BYTES)?;
    if materialized.files != before.files || materialized.directories != before.directories {
        return Err("materialized source files do not exactly match the Git authority".to_owned());
    }
    source_blobs::validate(
        git,
        repository,
        destination,
        &before.blobs,
        hardened_git_command,
    )?;
    let after = clean_state(git, repository)?;
    if before != after {
        return Err("Git authority changed during materialization".to_owned());
    }
    if git_sha256 != git_digest(git)? {
        return Err("bound Git executable changed during materialization".to_owned());
    }
    Ok(Snapshot {
        revision: before.revision,
        source_date_epoch: before.committer_epoch,
        source_inputs_sha256: materialized.sha256,
        materialized_root: destination.to_path_buf(),
        git_sha256,
    })
}

pub(super) fn assert_unchanged(
    git: &Path,
    repository: &Path,
    snapshot: &Snapshot,
) -> Result<(), String> {
    let current = clean_state(git, repository)?;
    if current.revision != snapshot.revision
        || current.committer_epoch != snapshot.source_date_epoch
    {
        return Err("Git authority revision or SOURCE_DATE_EPOCH changed".to_owned());
    }
    let materialized = source_tree::inventory(
        &snapshot.materialized_root,
        SOURCE_DOMAIN,
        true,
        MAX_SOURCE_BYTES,
    )?;
    if materialized.sha256 != snapshot.source_inputs_sha256
        || materialized.files != current.files
        || materialized.directories != current.directories
    {
        return Err("materialized source authority changed during build".to_owned());
    }
    if snapshot.git_sha256 != git_digest(git)? {
        return Err("bound Git executable changed during build".to_owned());
    }
    Ok(())
}

/// V1 deliberately accepts no source or Cargo-home configuration files.
pub(super) fn empty_cargo_config_set(source: &Path, cargo_home: &Path) -> Result<String, String> {
    authority::validate_directory(source, "materialized source")?;
    authority::validate_directory(cargo_home, "controlled Cargo home")?;
    let source_tree = source_tree::inventory(source, SOURCE_DOMAIN, true, MAX_SOURCE_BYTES)?;
    if let Some(record) = source_tree
        .files
        .iter()
        .find(|record| is_cargo_config(&record.path))
    {
        return Err(format!(
            "Cargo configuration is forbidden in v1: {}",
            source.join(&record.path).display()
        ));
    }
    let candidates = [
        source.join(".cargo/config"),
        source.join(".cargo/config.toml"),
        cargo_home.join("config"),
        cargo_home.join("config.toml"),
    ];
    for candidate in candidates {
        match fs::symlink_metadata(&candidate) {
            Ok(_) => {
                return Err(format!(
                    "Cargo configuration is forbidden in v1: {}",
                    candidate.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "inspect Cargo configuration {}: {error}",
                    candidate.display()
                ));
            }
        }
    }
    Ok(hex(EMPTY_CONFIG_DOMAIN))
}

fn is_cargo_config(path: &str) -> bool {
    matches!(path, ".cargo/config" | ".cargo/config.toml")
        || path.ends_with("/.cargo/config")
        || path.ends_with("/.cargo/config.toml")
}

/// Hashes a pre-provisioned read-only Cargo registry tree without copying it.
pub(super) fn cargo_home_inputs(registry_root: &Path) -> Result<String, String> {
    source_tree::inventory(
        registry_root,
        CARGO_HOME_DOMAIN,
        false,
        MAX_CARGO_HOME_BYTES,
    )
    .map(|tree| tree.sha256)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitState {
    revision: String,
    committer_epoch: u64,
    files: BTreeSet<source_tree::FileRecord>,
    directories: BTreeSet<String>,
    blobs: BTreeMap<String, String>,
}

fn validate_inputs(git: &Path, repository: &Path, destination: &Path) -> Result<String, String> {
    if !git.is_absolute() || !repository.is_absolute() || !destination.is_absolute() {
        return Err("Git, repository, and destination paths must be absolute".to_owned());
    }
    let git_sha256 = git_digest(git)?;
    authority::validate_directory(repository, "repository")?;
    authority::validate_directory(destination, "materialization destination")?;
    if destination.starts_with(repository) {
        return Err("materialization destination must be outside repository".to_owned());
    }
    if fs::read_dir(destination)
        .map_err(|error| format!("read materialization destination: {error}"))?
        .next()
        .transpose()
        .map_err(|error| format!("enumerate materialization destination: {error}"))?
        .is_some()
    {
        return Err("materialization destination must be new and empty".to_owned());
    }
    Ok(git_sha256)
}

fn git_digest(git: &Path) -> Result<String, String> {
    authority::digest(git, 2 * 1024 * 1024 * 1024, "Git executable").map(|(sha256, _)| sha256)
}

fn clean_state(git: &Path, repository: &Path) -> Result<GitState, String> {
    let revision = git_output(git, repository, ["rev-parse", "--verify", "HEAD^{commit}"])?;
    let status = git_output(
        git,
        repository,
        ["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    if !status.is_empty() {
        return Err("Git worktree, index, or untracked files are not clean".to_owned());
    }
    git_success(git, repository, ["diff", "--quiet"])?;
    git_success(git, repository, ["diff", "--cached", "--quiet"])?;
    let epoch = git_output(git, repository, ["show", "-s", "--format=%ct", "HEAD"])?;
    let committer_epoch = epoch
        .parse::<u64>()
        .map_err(|_| "Git committer timestamp is invalid".to_owned())?;
    if committer_epoch == 0 {
        return Err("Git committer timestamp must be non-zero".to_owned());
    }
    validate_hex(&revision, "Git revision")?;
    let tree_listing = git_bytes(
        git,
        repository,
        ["ls-tree", "-r", "-z", "--full-tree", "HEAD"],
    )?;
    let tree = validate_git_tree(&tree_listing)?;
    Ok(GitState {
        revision,
        committer_epoch,
        files: tree.files,
        directories: tree.directories,
        blobs: tree.blobs,
    })
}

fn checkout_index(git: &Path, repository: &Path, destination: &Path) -> Result<(), String> {
    let prefix = format!("{}/", destination.display());
    git_success(
        git,
        repository,
        ["checkout-index", "--all", "--force", "--prefix", &prefix],
    )
}

fn git_output<const N: usize>(
    git: &Path,
    repository: &Path,
    args: [&str; N],
) -> Result<String, String> {
    let bytes = git_bytes(git, repository, args)?;
    let value =
        String::from_utf8(bytes).map_err(|error| format!("Git output is not UTF-8: {error}"))?;
    let value = value.strip_suffix('\n').unwrap_or(&value);
    if value.contains('\n') || value.contains('\r') || value.len() > 4096 {
        return Err("Git command produced an invalid scalar result".to_owned());
    }
    Ok(value.to_owned())
}

fn git_success<const N: usize>(
    git: &Path,
    repository: &Path,
    args: [&str; N],
) -> Result<(), String> {
    let output = git_run(git, repository, args)?;
    if !output.stdout.is_empty() || !output.stderr.is_empty() {
        return Err("Git clean-state command produced unexpected output".to_owned());
    }
    Ok(())
}

fn git_bytes<const N: usize>(
    git: &Path,
    repository: &Path,
    args: [&str; N],
) -> Result<Vec<u8>, String> {
    let output = git_run(git, repository, args)?;
    if !output.stderr.is_empty() {
        return Err(format!(
            "Git wrote stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}

fn git_run<const N: usize>(
    git: &Path,
    repository: &Path,
    args: [&str; N],
) -> Result<process::Output, String> {
    let mut command = hardened_git_command(git, repository);
    command.args(args);
    process::run(
        command,
        "bound Git",
        MAX_GIT_OUTPUT,
        MAX_GIT_OUTPUT,
        GIT_TIMEOUT,
    )
}

fn hardened_git_command(git: &Path, repository: &Path) -> Command {
    let mut command = Command::new(git);
    command
        .current_dir(repository)
        .env_clear()
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("HOME", "/nonexistent")
        .env("LC_ALL", "C")
        .env("PATH", "/usr/bin")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.attributesfile=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.eol=lf",
            "-c",
            "filter.lfs.process=",
            "-c",
            "filter.lfs.required=false",
        ]);
    command
}

#[derive(Debug)]
struct GitTree {
    files: BTreeSet<source_tree::FileRecord>,
    directories: BTreeSet<String>,
    blobs: BTreeMap<String, String>,
}

fn validate_git_tree(bytes: &[u8]) -> Result<GitTree, String> {
    let mut records = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut blobs = BTreeMap::new();
    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let separator = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| "malformed Git tree record".to_owned())?;
        let (left, path) = (&record[..separator], &record[separator + 1..]);
        let fields: Vec<_> = left.split(|byte| *byte == b' ').collect();
        let [mode, kind, object] = fields.as_slice() else {
            return Err("malformed Git tree record".to_owned());
        };
        if !matches!(*mode, b"100644" | b"100755") || *kind != b"blob" {
            return Err("Git tree contains a symlink, gitlink, or unsupported mode".to_owned());
        }
        let path =
            std::str::from_utf8(path).map_err(|_| "Git tree path is not UTF-8".to_owned())?;
        validate_relative(path)?;
        if path == ".gitattributes" || path.ends_with("/.gitattributes") {
            return Err("Git attributes are forbidden in source materialization v1".to_owned());
        }
        let object =
            std::str::from_utf8(object).map_err(|_| "Git object is not UTF-8".to_owned())?;
        validate_hex(object, "Git object")?;
        let mode = match *mode {
            b"100644" => 0o644,
            b"100755" => 0o755,
            _ => unreachable!("the allowed Git modes were matched above"),
        };
        if !records.insert(source_tree::FileRecord {
            mode,
            path: path.to_owned(),
        }) {
            return Err("Git tree paths are not unique".to_owned());
        }
        if blobs.insert(path.to_owned(), object.to_owned()).is_some() {
            return Err("Git tree blob paths are not unique".to_owned());
        }
        let mut parent = Path::new(path).parent();
        while let Some(directory) = parent {
            let directory = directory
                .to_str()
                .ok_or_else(|| "Git tree directory is not UTF-8".to_owned())?;
            if directory.is_empty() {
                break;
            }
            directories.insert(directory.to_owned());
            parent = Path::new(directory).parent();
        }
    }
    if records.is_empty() {
        Err("Git tree is empty".to_owned())
    } else {
        Ok(GitTree {
            files: records,
            directories,
            blobs,
        })
    }
}

fn validate_hex(value: &str, label: &str) -> Result<(), String> {
    if matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!("invalid {label}"))
    }
}

fn validate_relative(path: &str) -> Result<(), String> {
    let value = Path::new(path);
    if value.is_absolute()
        || path.is_empty()
        || path.bytes().any(|byte| byte.is_ascii_control())
        || value
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        Err("unsafe Git tree path".to_owned())
    } else {
        Ok(())
    }
}

fn hex(domain: &[u8]) -> String {
    format!("{:x}", Sha256::digest(domain))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_gitlinks() {
        assert!(validate_git_tree(
            b"160000 commit 0123456789012345678901234567890123456789\tchild\0"
        )
        .is_err());
    }

    #[test]
    fn rejects_attribute_driven_checkout_transforms() {
        assert!(validate_git_tree(
            b"100644 blob 0123456789012345678901234567890123456789\t.gitattributes\0"
        )
        .unwrap_err()
        .contains("attributes"));
    }
    #[test]
    fn empty_config_digest_is_domain_separated() {
        assert_eq!(hex(EMPTY_CONFIG_DOMAIN).len(), 64);
    }

    #[test]
    fn detects_nested_cargo_configurations() {
        assert!(is_cargo_config(".cargo/config.toml"));
        assert!(is_cargo_config("member/.cargo/config"));
        assert!(!is_cargo_config("member/config.toml"));
    }
}
