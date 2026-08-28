use std::fs;
use std::path::{Path, PathBuf};

use super::model::validate_relative_path;

#[derive(Debug, Clone, Copy)]
enum LeafKind {
    File,
    Directory,
    OutputFile,
}

pub fn canonical_root(root: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(root)
        .map_err(|error| format!("canonicalize repository root {}: {error}", root.display()))?;
    let metadata = fs::symlink_metadata(&canonical)
        .map_err(|error| format!("inspect repository root {}: {error}", canonical.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!(
            "repository root {} is not a real directory",
            canonical.display()
        ));
    }
    Ok(canonical)
}

pub fn existing_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    resolve(root, relative, LeafKind::File)
}

pub fn existing_directory(root: &Path, relative: &str) -> Result<PathBuf, String> {
    resolve(root, relative, LeafKind::Directory)
}

pub fn output_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    resolve(root, relative, LeafKind::OutputFile)
}

fn resolve(root: &Path, relative: &str, leaf_kind: LeafKind) -> Result<PathBuf, String> {
    validate_relative_path(relative)?;
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| format!("canonicalize repository root {}: {error}", root.display()))?;
    if !root.is_absolute() || canonical_root != root {
        return Err("repository root must be canonical and absolute".to_owned());
    }
    let components: Vec<_> = Path::new(relative).components().collect();
    let mut current = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        let last = index + 1 == components.len();
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "fixed path contains symlink component {}",
                    current.display()
                ));
            }
            Ok(metadata) if !last && !metadata.is_dir() => {
                return Err(format!(
                    "fixed path parent {} is not a directory",
                    current.display()
                ));
            }
            Ok(metadata) if last => match leaf_kind {
                LeafKind::File if !metadata.is_file() => {
                    return Err(format!("fixed path {} is not a file", current.display()));
                }
                LeafKind::Directory if !metadata.is_dir() => {
                    return Err(format!(
                        "fixed path {} is not a directory",
                        current.display()
                    ));
                }
                LeafKind::OutputFile if !metadata.is_file() => {
                    return Err(format!("fixed output {} is not a file", current.display()));
                }
                _ => {}
            },
            Ok(_) => {}
            Err(error)
                if last
                    && matches!(leaf_kind, LeafKind::OutputFile)
                    && error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("inspect fixed path {}: {error}", current.display()));
            }
        }
    }
    if !current.starts_with(root) {
        return Err(format!("fixed path escapes repository root: {relative}"));
    }
    Ok(current)
}
