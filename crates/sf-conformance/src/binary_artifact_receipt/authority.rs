//! Hardened reads for capture inputs and the generated artifact bundle.

use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use super::authority_guard;

const MAX_PATH_COMPONENTS: usize = 128;

/// A regular, immutable-enough file opened without following a final symlink.
#[derive(Debug)]
pub(super) struct AuthorityFile {
    pub(super) bytes: Vec<u8>,
    pub(super) sha256: String,
    pub(super) size: u64,
}

pub(super) fn read(path: &Path, max_bytes: u64, label: &str) -> Result<AuthorityFile, String> {
    authority_guard::validate_parent_ancestry(path, label)?;
    let before = inspect_file(path, label)?;
    let file =
        File::open(path).map_err(|error| format!("open {label} {}: {error}", path.display()))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("inspect opened {label} {}: {error}", path.display()))?;
    validate_file(&opened, label)?;
    if !same_file(&before, &opened) {
        return Err(format!("{label} changed while opening"));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {label} {}: {error}", path.display()))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!("{label} exceeds {max_bytes} bytes"));
    }
    let after = inspect_file(path, label)?;
    if !same_file(&opened, &after) || opened.len() != after.len() {
        return Err(format!("{label} changed during read"));
    }
    Ok(AuthorityFile {
        sha256: hex_digest(&bytes),
        size: bytes.len() as u64,
        bytes,
    })
}

pub(super) fn digest(path: &Path, max_bytes: u64, label: &str) -> Result<(String, u64), String> {
    authority_guard::validate_parent_ancestry(path, label)?;
    let before = inspect_file(path, label)?;
    if before.len() > max_bytes {
        return Err(format!("{label} exceeds {max_bytes} bytes"));
    }
    let mut file =
        File::open(path).map_err(|error| format!("open {label} {}: {error}", path.display()))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("inspect opened {label} {}: {error}", path.display()))?;
    validate_file(&opened, label)?;
    if !same_file(&before, &opened) {
        return Err(format!("{label} changed while opening"));
    }
    let mut digest = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read {label} {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| format!("{label} size overflow"))?;
        if total > max_bytes {
            return Err(format!("{label} exceeds {max_bytes} bytes"));
        }
        digest.update(&buffer[..read]);
    }
    let after = inspect_file(path, label)?;
    if !same_file(&opened, &after) || total != after.len() {
        return Err(format!("{label} changed during read"));
    }
    Ok((format!("{:x}", digest.finalize()), total))
}

pub(super) fn validate_directory(path: &Path, label: &str) -> Result<(), String> {
    authority_guard::DirectoryGuard::bind(path, label).map(|_| ())
}

/// Reject lexical escapes and every symlink component below `root`.
pub(super) fn validate_beneath(
    root: &Path,
    candidate: &Path,
    label: &str,
) -> Result<PathBuf, String> {
    if !root.is_absolute() || !candidate.is_absolute() {
        return Err(format!("{label} root and candidate must be absolute"));
    }
    validate_directory(root, label)?;
    let relative = candidate
        .strip_prefix(root)
        .map_err(|_| format!("{label} escapes its root"))?;
    if relative.as_os_str().is_empty() {
        return Err(format!("{label} cannot equal its root"));
    }
    let mut current = root.to_path_buf();
    let mut components = 0usize;
    for component in relative.components() {
        components = components
            .checked_add(1)
            .ok_or_else(|| format!("{label} component count overflow"))?;
        if components > MAX_PATH_COMPONENTS {
            return Err(format!("{label} has too many path components"));
        }
        let Component::Normal(part) = component else {
            return Err(format!("{label} path contains a non-normal component"));
        };
        current.push(part);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("inspect {label} component {}: {error}", current.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("{label} contains a symlink component"));
        }
        if current != candidate && !metadata.is_dir() {
            return Err(format!("{label} parent component is not a directory"));
        }
        validate_permissions(&metadata, label)?;
    }
    Ok(candidate.to_path_buf())
}

fn inspect_file(path: &Path, label: &str) -> Result<Metadata, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect {label} {}: {error}", path.display()))?;
    validate_file(&metadata, label)?;
    Ok(metadata)
}

fn validate_file(metadata: &Metadata, label: &str) -> Result<(), String> {
    if metadata.file_type().is_symlink() {
        return Err(format!("{label} is a symlink"));
    }
    if !metadata.is_file() {
        return Err(format!("{label} is not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(format!("{label} is a hard link"));
        }
    }
    validate_permissions(metadata, label)
}

fn validate_permissions(metadata: &Metadata, label: &str) -> Result<(), String> {
    authority_guard::validate_leaf(metadata, label)
}

#[cfg(unix)]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    left.len() == right.len()
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "semantic-fabric-artifact-authority-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        path
    }

    fn write_private(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[test]
    fn reads_regular_immutable_file() {
        let root = fixture("read");
        let file = root.join("input");
        write_private(&file, b"input");
        let authority = read(&file, 16, "fixture").unwrap();
        assert_eq!(authority.bytes, b"input");
        assert_eq!(authority.size, 5);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_hardlink_and_writable_file() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = fixture("links");
        let file = root.join("input");
        let alias = root.join("alias");
        let link = root.join("link");
        write_private(&file, b"input");
        fs::hard_link(&file, &alias).unwrap();
        assert!(read(&file, 16, "fixture")
            .unwrap_err()
            .contains("hard link"));
        fs::remove_file(&alias).unwrap();
        symlink(&file, &link).unwrap();
        assert!(read(&link, 16, "fixture").unwrap_err().contains("symlink"));
        fs::set_permissions(&file, fs::Permissions::from_mode(0o664)).unwrap();
        assert!(read(&file, 16, "fixture").unwrap_err().contains("writable"));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_private_file_beneath_a_writable_ancestor() {
        use std::os::unix::fs::PermissionsExt;

        let root = fixture("writable-ancestor");
        let writable = root.join("writable");
        let private = writable.join("private");
        fs::create_dir(&writable).unwrap();
        fs::set_permissions(&writable, fs::Permissions::from_mode(0o770)).unwrap();
        fs::create_dir(&private).unwrap();
        fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
        let file = private.join("input");
        write_private(&file, b"input");

        let error = read(&file, 16, "fixture").unwrap_err();

        assert!(
            error.contains("ancestor") && error.contains("writable"),
            "{error}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_escapes_and_symlink_components() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = fixture("beneath");
        let outside = fixture("outside");
        let nested = root.join("nested");
        fs::create_dir(&nested).unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
        let file = nested.join("file");
        write_private(&file, b"input");
        assert_eq!(validate_beneath(&root, &file, "fixture").unwrap(), file);
        assert!(validate_beneath(&root, &outside.join("x"), "fixture")
            .unwrap_err()
            .contains("escapes"));
        let link = root.join("link");
        symlink(&outside, &link).unwrap();
        assert!(validate_beneath(&root, &link.join("x"), "fixture")
            .unwrap_err()
            .contains("symlink"));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
