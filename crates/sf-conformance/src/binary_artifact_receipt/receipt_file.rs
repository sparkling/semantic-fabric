//! External receipt I/O with no source-tree self-reference or overwrite.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{authority, parse, render, Receipt, MAX_RECEIPT_BYTES};

const TEMP_ATTEMPTS: usize = 128;
static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

/// Loads one private, canonical receipt outside the observed repository.
pub fn load_external(repository: &Path, path: &Path) -> Result<Receipt, String> {
    let path = external_path(repository, path, false)?;
    let file = authority::read(
        &path,
        MAX_RECEIPT_BYTES as u64,
        "external artifact observation receipt",
    )?;
    let text = String::from_utf8(file.bytes)
        .map_err(|error| format!("external receipt is not UTF-8: {error}"))?;
    parse(&text)
}

/// Creates one canonical receipt atomically and refuses to replace any target.
pub fn write_new_external(repository: &Path, path: &Path, receipt: &Receipt) -> Result<(), String> {
    let path = external_path(repository, path, true)?;
    let rendered = render(receipt)?;
    write_new(&path, rendered.as_bytes())
}

fn external_path(repository: &Path, path: &Path, require_missing: bool) -> Result<PathBuf, String> {
    validate_absolute_normal(repository, "repository")?;
    validate_absolute_normal(path, "external receipt")?;
    let repository = fs::canonicalize(repository)
        .map_err(|error| format!("canonicalize repository {}: {error}", repository.display()))?;
    authority::validate_directory(&repository, "repository")?;
    let parent = path
        .parent()
        .ok_or_else(|| "external receipt has no parent directory".to_owned())?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| format!("canonicalize receipt parent {}: {error}", parent.display()))?;
    if canonical_parent != parent {
        return Err("external receipt parent is not canonical".to_owned());
    }
    authority::validate_directory(parent, "external receipt parent")?;
    if path.starts_with(&repository) {
        return Err("artifact observation receipt must remain outside the repository".to_owned());
    }
    if require_missing {
        require_absent(path)?;
    }
    Ok(path.to_path_buf())
}

fn validate_absolute_normal(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(format!("{label} path must be absolute and normalized"));
    }
    Ok(())
}

fn require_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(format!(
            "external receipt target already exists: {}",
            path.display()
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "inspect external receipt target {}: {error}",
            path.display()
        )),
    }
}

fn write_new(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "external receipt target has no parent".to_owned())?;
    let (temporary, mut file) = create_temporary(parent)?;
    let mut linked = false;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|error| format!("write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("sync {}: {error}", temporary.display()))?;
        drop(file);
        require_absent(target)?;
        fs::hard_link(&temporary, target).map_err(|error| {
            format!(
                "atomically create {} from {}: {error}",
                target.display(),
                temporary.display()
            )
        })?;
        linked = true;
        fs::remove_file(&temporary)
            .map_err(|error| format!("unlink {}: {error}", temporary.display()))?;
        linked = false;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        if linked {
            let _ = fs::remove_file(target);
        }
    }
    result
}

fn create_temporary(parent: &Path) -> Result<(PathBuf, File), String> {
    for _ in 0..TEMP_ATTEMPTS {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".current-sf-cli-artifact-observation.tmp-{}-{serial}",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("create {}: {error}", path.display())),
        }
    }
    Err(format!(
        "could not create an exclusive receipt temporary after {TEMP_ATTEMPTS} attempts"
    ))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync directory {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private_directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "semantic-fabric-receipt-file-{name}-{}",
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

    #[test]
    fn creates_once_without_overwrite() {
        let root = private_directory("create");
        let target = root.join("receipt.tsv");
        write_new(&target, b"receipt\n").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"receipt\n");
        assert!(write_new(&target, b"replacement\n").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn requires_an_external_canonical_path() {
        let root = private_directory("path");
        let repository = root.join("repository");
        let output = root.join("output");
        fs::create_dir(&repository).unwrap();
        fs::create_dir(&output).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for directory in [&repository, &output] {
                fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        assert!(external_path(&repository, &repository.join("receipt.tsv"), true).is_err());
        assert_eq!(
            external_path(&repository, &output.join("receipt.tsv"), true).unwrap(),
            output.join("receipt.tsv")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
