//! Held authority for the exact bubblewrap inode used by the prepared probe.

use std::fs::{self, File, Metadata, OpenOptions};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::super::linux::{self, FileIdentity};
use super::ExpectedBwrapIdentity;
use crate::binary_artifact_receipt::authority_guard::DirectoryGuard;
use crate::binary_artifact_receipt::runtime_linkage::MAX_BWRAP_BYTES;

#[derive(Debug)]
pub(super) struct HeldBwrap {
    path: PathBuf,
    parent: DirectoryGuard,
    file: File,
    identity: FileIdentity,
    sha256: String,
    effective_uid: u32,
    require_root: bool,
}

impl HeldBwrap {
    pub(super) fn bind(expected: &ExpectedBwrapIdentity) -> Result<Self, String> {
        Self::bind_with_policy(expected, true)
    }

    #[cfg(test)]
    pub(super) fn bind_fixture(expected: &ExpectedBwrapIdentity) -> Result<Self, String> {
        Self::bind_with_policy(expected, false)
    }

    fn bind_with_policy(
        expected: &ExpectedBwrapIdentity,
        require_root: bool,
    ) -> Result<Self, String> {
        let path = &expected.path;
        super::super::super::validate_absolute_path(path, "bubblewrap executable")?;
        let parent_path = path
            .parent()
            .ok_or_else(|| "bubblewrap executable has no parent".to_owned())?;
        let parent = DirectoryGuard::bind(parent_path, "bubblewrap executable")?;
        let before = inspect_path(path, require_root)?;
        let file = open(path)?;
        let opened = inspect_opened(&file, require_root)?;
        if before != opened {
            return Err("bubblewrap executable changed while opening".to_owned());
        }
        let sha256 = digest(&file, opened.size)?;
        if sha256 != expected.sha256 || opened.size != expected.byte_length {
            return Err("bubblewrap executable differs from authorized identity".to_owned());
        }
        let held = Self {
            path: path.to_path_buf(),
            parent,
            file,
            identity: opened,
            sha256,
            effective_uid: effective_uid(),
            require_root,
        };
        held.assert_current()?;
        Ok(held)
    }

    pub(super) fn raw_fd(&self) -> RawFd {
        self.file.as_raw_fd()
    }

    pub(super) fn sha256(&self) -> &str {
        &self.sha256
    }

    pub(super) fn byte_length(&self) -> u64 {
        self.identity.size
    }

    pub(super) fn assert_current(&self) -> Result<(), String> {
        if effective_uid() != self.effective_uid {
            return Err("effective UID changed while holding bubblewrap".to_owned());
        }
        self.parent.assert_current()?;
        linux::require_cloexec(&self.file, "held bubblewrap executable")?;
        let path_identity = inspect_path(&self.path, self.require_root)?;
        let opened_identity = inspect_opened(&self.file, self.require_root)?;
        let fresh = open(&self.path)?;
        let fresh_identity = inspect_opened(&fresh, self.require_root)?;
        if path_identity != self.identity
            || opened_identity != self.identity
            || fresh_identity != self.identity
            || digest(&self.file, self.identity.size)? != self.sha256
            || digest(&fresh, self.identity.size)? != self.sha256
        {
            return Err("held bubblewrap executable changed".to_owned());
        }
        self.parent.assert_current()?;
        Ok(())
    }
}

fn open(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    options
        .open(path)
        .map_err(|error| format!("open bubblewrap executable {}: {error}", path.display()))
}

fn inspect_path(path: &Path, require_root: bool) -> Result<FileIdentity, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect bubblewrap executable {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err("bubblewrap executable is a symlink".to_owned());
    }
    validate_metadata(&metadata, require_root)
}

fn inspect_opened(file: &File, require_root: bool) -> Result<FileIdentity, String> {
    linux::require_cloexec(file, "held bubblewrap executable")?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect opened bubblewrap executable: {error}"))?;
    let identity = validate_metadata(&metadata, require_root)?;
    reject_file_capabilities(file)?;
    Ok(identity)
}

fn validate_metadata(metadata: &Metadata, require_root: bool) -> Result<FileIdentity, String> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("bubblewrap executable is not a regular non-symlink file".to_owned());
    }
    let identity = FileIdentity::from_metadata(metadata);
    let allowed_owner = if require_root {
        identity.uid == 0 && identity.gid == 0
    } else {
        identity.uid == effective_uid()
    };
    if !allowed_owner
        || identity.links != 1
        || identity.mode & 0o7777 != 0o755
        || identity.size == 0
        || identity.size > MAX_BWRAP_BYTES
    {
        return Err("bubblewrap owner, mode, links, or size is outside policy".to_owned());
    }
    Ok(identity)
}

fn reject_file_capabilities(file: &File) -> Result<(), String> {
    // SAFETY: the descriptor is live, the xattr name is NUL terminated, and a
    // null value with size zero requests only the attribute length.
    let result = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            c"security.capability".as_ptr(),
            std::ptr::null_mut(),
            0,
        )
    };
    if result >= 0 {
        return Err("bubblewrap executable carries file capabilities".to_owned());
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ENODATA) | Some(libc::ENOTSUP) => Ok(()),
        _ => Err(format!("inspect bubblewrap file capabilities: {error}")),
    }
}

fn digest(file: &File, size: u64) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut offset = 0u64;
    while offset < size {
        let remaining = usize::try_from((size - offset).min(buffer.len() as u64))
            .map_err(|_| "bubblewrap digest size conversion failed".to_owned())?;
        let read = file
            .read_at(&mut buffer[..remaining], offset)
            .map_err(|error| format!("read held bubblewrap executable: {error}"))?;
        if read == 0 {
            return Err("held bubblewrap executable became shorter".to_owned());
        }
        hasher.update(&buffer[..read]);
        offset = offset
            .checked_add(read as u64)
            .ok_or_else(|| "bubblewrap digest offset overflow".to_owned())?;
    }
    let mut extra = [0u8; 1];
    if file
        .read_at(&mut extra, size)
        .map_err(|error| format!("probe held bubblewrap length: {error}"))?
        != 0
    {
        return Err("held bubblewrap executable grew while reading".to_owned());
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and dereferences no memory.
    unsafe { libc::geteuid() }
}
