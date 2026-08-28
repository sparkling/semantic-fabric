//! Trusted-principal ancestry policy and held directory identity guards.

use std::fs::{self, File, Metadata, OpenOptions};
use std::path::{Component, Path, PathBuf};

/// A directory identity held for one capture and checked at phase boundaries.
///
/// The descriptor detects persistent replacement. It is deliberately not a
/// descriptor-relative authority and does not claim same-principal ABA safety.
#[derive(Debug)]
pub(super) struct DirectoryGuard {
    path: PathBuf,
    label: String,
    directory: File,
}

impl DirectoryGuard {
    pub(super) fn bind(path: &Path, label: &str) -> Result<Self, String> {
        let before = validate_directory_chain(path, label, true)?;
        let directory = open_directory(path, label)?;
        let opened = directory.metadata().map_err(|error| {
            format!(
                "inspect opened {label} directory {}: {error}",
                path.display()
            )
        })?;
        validate_opened_directory(&opened, label)?;
        if !same_file(&before, &opened) {
            return Err(format!("{label} directory changed while opening"));
        }
        let guard = Self {
            path: path.to_path_buf(),
            label: label.to_owned(),
            directory,
        };
        guard.assert_current()?;
        Ok(guard)
    }

    pub(super) fn assert_current(&self) -> Result<(), String> {
        let current = validate_directory_chain(&self.path, &self.label, true)?;
        let opened = self.directory.metadata().map_err(|error| {
            format!(
                "inspect held {} directory {}: {error}",
                self.label,
                self.path.display()
            )
        })?;
        validate_opened_directory(&opened, &self.label)?;
        if !same_file(&opened, &current) {
            return Err(format!("{} directory identity changed", self.label));
        }
        Ok(())
    }
}

fn open_directory(path: &Path, label: &str) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    options
        .open(path)
        .map_err(|error| format!("open {label} directory {}: {error}", path.display()))
}

fn validate_opened_directory(metadata: &Metadata, label: &str) -> Result<(), String> {
    if !metadata.is_dir() {
        return Err(format!("{label} authority is not a directory"));
    }
    validate_owner_and_mode(metadata, label, false, false)
}

/// Validates every directory used to resolve a file authority.
pub(super) fn validate_parent_ancestry(path: &Path, label: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} has no parent directory"))?;
    validate_directory_chain(parent, label, false).map(|_| ())
}

/// Validates owner and mutation policy for a final authority node.
pub(super) fn validate_leaf(metadata: &Metadata, label: &str) -> Result<(), String> {
    validate_owner_and_mode(metadata, label, false, false)
}

fn validate_directory_chain(
    target: &Path,
    label: &str,
    final_is_authority: bool,
) -> Result<Metadata, String> {
    validate_absolute_normal(target, label)?;
    let mut current = PathBuf::new();
    let mut final_metadata = None;
    for component in target.components() {
        match component {
            Component::RootDir => current.push(Path::new("/")),
            Component::Normal(part) => current.push(part),
            _ => return Err(format!("{label} ancestry contains a non-normal component")),
        }
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            format!(
                "inspect {label} ancestor directory {}: {error}",
                current.display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "{label} ancestor directory is a symlink: {}",
                current.display()
            ));
        }
        if !metadata.is_dir() {
            return Err(format!(
                "{label} ancestor is not a directory: {}",
                current.display()
            ));
        }
        let is_final = current == target;
        let is_ancestor = !is_final || !final_is_authority;
        validate_owner_and_mode(&metadata, label, is_ancestor, is_ancestor)?;
        final_metadata = Some(metadata);
    }
    final_metadata.ok_or_else(|| format!("{label} directory path is empty"))
}

fn validate_absolute_normal(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        Err(format!("{label} path must be absolute and normalized"))
    } else {
        Ok(())
    }
}

fn validate_owner_and_mode(
    metadata: &Metadata,
    label: &str,
    allow_root_sticky: bool,
    ancestor: bool,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: `geteuid` has no preconditions and does not dereference memory.
        let effective_uid = unsafe { libc::geteuid() };
        validate_unix_policy(
            metadata.mode(),
            metadata.uid(),
            effective_uid,
            allow_root_sticky,
            ancestor,
            label,
        )?;
    }
    #[cfg(not(unix))]
    {
        let _ = (metadata, label, allow_root_sticky, ancestor);
    }
    Ok(())
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

#[cfg(unix)]
fn validate_unix_policy(
    mode: u32,
    owner: u32,
    effective_uid: u32,
    allow_root_sticky: bool,
    ancestor: bool,
    label: &str,
) -> Result<(), String> {
    let kind = if ancestor { "ancestor" } else { "authority" };
    if owner != 0 && owner != effective_uid {
        return Err(format!(
            "{label} {kind} is not owned by root or the effective capture user"
        ));
    }
    if mode & 0o022 != 0 {
        let root_sticky = allow_root_sticky && owner == 0 && mode & 0o1000 != 0;
        if !root_sticky {
            return Err(format!("{label} {kind} is group- or world-writable"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn held_guard_rejects_a_persistent_directory_replacement() {
        use std::os::unix::fs::PermissionsExt;

        let parent = std::env::temp_dir().join(format!(
            "semantic-fabric-directory-guard-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&parent);
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        let authority = parent.join("authority");
        let moved = parent.join("moved");
        fs::create_dir(&authority).unwrap();
        fs::set_permissions(&authority, fs::Permissions::from_mode(0o700)).unwrap();
        let guard = DirectoryGuard::bind(&authority, "fixture authority").unwrap();
        fs::rename(&authority, &moved).unwrap();
        fs::create_dir(&authority).unwrap();
        fs::set_permissions(&authority, fs::Permissions::from_mode(0o700)).unwrap();

        let error = guard.assert_current().unwrap_err();

        assert!(error.contains("changed"), "{error}");
        drop(guard);
        fs::remove_dir_all(parent).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn policy_rejects_an_authority_owned_by_another_principal() {
        let error =
            validate_unix_policy(0o100600, 2000, 1000, false, false, "fixture").unwrap_err();
        assert!(error.contains("owned by root or the effective capture user"));
    }

    #[cfg(unix)]
    #[test]
    fn policy_allows_a_root_owned_sticky_ancestor() {
        assert!(validate_unix_policy(0o041777, 0, 1000, true, true, "fixture").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn policy_rejects_a_writable_final_authority_even_when_sticky() {
        let error = validate_unix_policy(0o041777, 0, 1000, false, false, "fixture").unwrap_err();
        assert!(error.contains("group- or world-writable"));
    }
}
