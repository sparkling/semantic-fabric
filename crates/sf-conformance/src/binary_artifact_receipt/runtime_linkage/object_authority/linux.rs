//! Linux descriptor acquisition and sealed-byte primitives.

use std::collections::BTreeSet;
use std::ffi::CString;
use std::fs::{File, Metadata};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use super::super::super::authority_guard::DirectoryGuard;
use super::super::RuntimeReadOnlyMount;

pub(super) mod component;
mod logical_path;
mod sealed;

#[cfg(test)]
pub(super) use sealed::snapshot_source_with_phase_hook;
pub(super) use sealed::{
    assert_sealed_current, assert_sealed_duplicate_current, snapshot_source, SealedBytes,
};

const RESOLVE_NO_XDEV: u64 = 0x01;
const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;
const RESOLVE_POLICY: u64 = RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_SYMLINKS;
pub(super) const OBJECT_RESOLVE_POLICY: u64 = RESOLVE_POLICY | RESOLVE_NO_XDEV;
const MAX_ALIAS_BYTES: usize = 4096;

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct FileIdentity {
    pub(super) device: u64,
    pub(super) inode: u64,
    pub(super) links: u64,
    pub(super) uid: u32,
    pub(super) gid: u32,
    pub(super) mode: u32,
    pub(super) size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl FileIdentity {
    pub(super) fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            links: metadata.nlink(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            mode: metadata.mode(),
            size: metadata.size(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

#[derive(Debug)]
pub(super) struct HeldMount {
    pub(super) destination: String,
    source: PathBuf,
    guard: DirectoryGuard,
    root: File,
    identity: FileIdentity,
}

#[derive(Debug)]
pub(super) struct HeldAlias {
    file: File,
    identity: FileIdentity,
    pub(super) logical_path: String,
    raw_target: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct ResolvedSource {
    pub(super) logical_path: String,
    pub(super) terminal_logical_path: String,
    pub(super) file: File,
    pub(super) identity: FileIdentity,
    pub(super) alias: Option<HeldAlias>,
}

pub(super) fn hold_mounts(mounts: &[RuntimeReadOnlyMount]) -> Result<Vec<HeldMount>, String> {
    let root = raw_open(c"/", libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)?;
    require_cloexec(&root, "held filesystem root")?;
    let mut destinations = BTreeSet::new();
    let mut held = Vec::with_capacity(mounts.len());
    for mount in mounts {
        if !destinations.insert(mount.destination.as_str()) {
            return Err("duplicate held runtime mount destination".to_owned());
        }
        let guard = DirectoryGuard::bind(&mount.source, "runtime mount source")?;
        let relative = mount
            .source
            .strip_prefix("/")
            .map_err(|_| "runtime mount source is not absolute".to_owned())?;
        let directory = openat2(
            &root,
            relative,
            libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )?;
        require_cloexec(&directory, "held runtime mount")?;
        let metadata = directory
            .metadata()
            .map_err(|error| format!("inspect held runtime mount: {error}"))?;
        validate_directory(&metadata)?;
        held.push(HeldMount {
            destination: mount.destination.clone(),
            source: mount.source.clone(),
            guard,
            root: directory,
            identity: FileIdentity::from_metadata(&metadata),
        });
    }
    held.sort_by(|left, right| left.destination.cmp(&right.destination));
    Ok(held)
}

pub(super) fn resolve_source(
    mounts: &[HeldMount],
    logical_path: &str,
    allow_loader_alias: bool,
) -> Result<ResolvedSource, String> {
    logical_path::validate(logical_path)?;
    let (mount, relative) = map_logical(mounts, logical_path)?;
    let path_handle = component::open_leaf(mount, &relative)?;
    let metadata = path_handle
        .handle
        .metadata()
        .map_err(|error| format!("inspect held runtime path: {error}"))?;
    if metadata.file_type().is_symlink() {
        if !allow_loader_alias {
            return Err("runtime object final symlink is not the loader alias".to_owned());
        }
        let alias_identity = validate_alias(&metadata)?;
        let raw_target = read_link_fd(&path_handle.handle)?;
        let terminal_logical = logical_path::normalize_relative_alias(logical_path, &raw_target)?;
        let (terminal_mount, terminal_relative) = map_logical(mounts, &terminal_logical)?;
        let terminal_handle = component::open_leaf(terminal_mount, &terminal_relative)?;
        let terminal_metadata = terminal_handle
            .handle
            .metadata()
            .map_err(|error| format!("inspect loader alias terminal: {error}"))?;
        if terminal_metadata.file_type().is_symlink() {
            return Err("loader alias must resolve in exactly one hop".to_owned());
        }
        let file = component::open_regular(&terminal_handle)?;
        let identity = inspect_runtime_regular(&file, true)?;
        if FileIdentity::from_metadata(&terminal_metadata) != identity {
            return Err("loader alias terminal changed while opening".to_owned());
        }
        return Ok(ResolvedSource {
            logical_path: logical_path.to_owned(),
            terminal_logical_path: terminal_logical,
            file,
            identity,
            alias: Some(HeldAlias {
                file: path_handle.handle,
                identity: alias_identity,
                logical_path: logical_path.to_owned(),
                raw_target,
            }),
        });
    }
    let file = component::open_regular(&path_handle)?;
    let identity = inspect_runtime_regular(&file, allow_loader_alias)?;
    if FileIdentity::from_metadata(&metadata) != identity {
        return Err("runtime object changed while opening".to_owned());
    }
    Ok(ResolvedSource {
        logical_path: logical_path.to_owned(),
        terminal_logical_path: logical_path.to_owned(),
        file,
        identity,
        alias: None,
    })
}

pub(super) fn inspect_artifact(file: &File) -> Result<FileIdentity, String> {
    require_cloexec(file, "held artifact duplicate")?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect held artifact duplicate: {error}"))?;
    validate_regular_metadata(&metadata, 2, true, "held artifact")
}

pub(super) fn assert_mounts_current(mounts: &[HeldMount]) -> Result<(), String> {
    let filesystem_root = raw_open(c"/", libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)?;
    for mount in mounts {
        mount.guard.assert_current()?;
        require_cloexec(&mount.root, "held runtime mount")?;
        let metadata = mount
            .root
            .metadata()
            .map_err(|error| format!("reinspect held runtime mount: {error}"))?;
        validate_directory(&metadata)?;
        let relative = mount
            .source
            .strip_prefix("/")
            .map_err(|_| "runtime mount source is not absolute".to_owned())?;
        let current = openat2(
            &filesystem_root,
            relative,
            libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )?;
        if FileIdentity::from_metadata(&metadata) != mount.identity
            || FileIdentity::from_metadata(
                &current
                    .metadata()
                    .map_err(|error| format!("reinspect runtime mount path: {error}"))?,
            ) != mount.identity
        {
            return Err("held runtime mount identity changed".to_owned());
        }
    }
    Ok(())
}

pub(super) fn assert_source_current(
    source: &ResolvedSource,
    mounts: &[HeldMount],
    sha256: &str,
) -> Result<(), String> {
    require_cloexec(&source.file, "held runtime source")?;
    let current = inspect_runtime_regular(&source.file, source.alias.is_some())?;
    let (digest, _) =
        sealed::read_exact(&source.file, source.identity.size, "held runtime source")?;
    if current != source.identity || digest != sha256 {
        return Err("held runtime source changed".to_owned());
    }
    if let Some(alias) = &source.alias {
        require_cloexec(&alias.file, "held loader alias")?;
        let metadata = alias
            .file
            .metadata()
            .map_err(|error| format!("reinspect held loader alias: {error}"))?;
        if validate_alias(&metadata)? != alias.identity
            || read_link_fd(&alias.file)? != alias.raw_target
        {
            return Err("held loader alias changed".to_owned());
        }
    }
    let fresh = resolve_source(mounts, &source.logical_path, source.alias.is_some())?;
    if fresh.identity != source.identity
        || fresh.terminal_logical_path != source.terminal_logical_path
        || alias_shape(&fresh.alias) != alias_shape(&source.alias)
    {
        return Err("runtime source resolution changed".to_owned());
    }
    Ok(())
}

pub(super) fn assert_plain_source_current(
    file: &File,
    identity: FileIdentity,
    sha256: &str,
) -> Result<(), String> {
    require_cloexec(file, "held artifact duplicate")?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("reinspect held artifact duplicate: {error}"))?;
    let (digest, _) = sealed::read_exact(file, identity.size, "held artifact duplicate")?;
    if FileIdentity::from_metadata(&metadata) != identity || digest != sha256 {
        return Err("held artifact duplicate changed".to_owned());
    }
    Ok(())
}

fn inspect_runtime_regular(file: &File, loader: bool) -> Result<FileIdentity, String> {
    require_cloexec(file, "held runtime regular file")?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect held runtime regular file: {error}"))?;
    validate_regular_metadata(&metadata, 1, loader, "runtime object")
}

fn validate_regular_metadata(
    metadata: &Metadata,
    links: u64,
    executable: bool,
    label: &str,
) -> Result<FileIdentity, String> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("{label} is not a regular non-symlink file"));
    }
    let identity = FileIdentity::from_metadata(metadata);
    let permissions = identity.mode & 0o7777;
    if identity.links != links
        || (identity.uid != 0 && identity.uid != effective_uid())
        || permissions & 0o7022 != 0
        || (executable && permissions & 0o111 == 0)
    {
        return Err(format!(
            "{label} owner, mode, or link count is outside policy"
        ));
    }
    Ok(identity)
}

fn validate_directory(metadata: &Metadata) -> Result<(), String> {
    if !metadata.is_dir() {
        return Err("runtime mount authority is not a directory".to_owned());
    }
    let mode = metadata.mode() & 0o7777;
    if (metadata.uid() != 0 && metadata.uid() != effective_uid()) || mode & 0o7022 != 0 {
        return Err("runtime mount authority owner or mode is outside policy".to_owned());
    }
    Ok(())
}

fn validate_alias(metadata: &Metadata) -> Result<FileIdentity, String> {
    if !metadata.file_type().is_symlink() {
        return Err("loader alias is not a symlink".to_owned());
    }
    let identity = FileIdentity::from_metadata(metadata);
    if identity.links != 1 || (identity.uid != 0 && identity.uid != effective_uid()) {
        return Err("loader alias owner or link count is outside policy".to_owned());
    }
    Ok(identity)
}

fn map_logical<'a>(
    mounts: &'a [HeldMount],
    logical: &str,
) -> Result<(&'a HeldMount, PathBuf), String> {
    let path = Path::new(logical);
    let mut candidates: Vec<_> = mounts
        .iter()
        .filter_map(|mount| {
            path.strip_prefix(&mount.destination)
                .ok()
                .map(|relative| (mount, relative.to_path_buf()))
        })
        .filter(|(_, relative)| relative.components().next().is_some())
        .collect();
    candidates.sort_by_key(|(mount, _)| std::cmp::Reverse(mount.destination.len()));
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| "runtime path has no exact held mount mapping".to_owned())
}

fn alias_shape(alias: &Option<HeldAlias>) -> Option<(&str, &[u8], FileIdentity)> {
    alias.as_ref().map(|value| {
        (
            value.logical_path.as_str(),
            value.raw_target.as_slice(),
            value.identity,
        )
    })
}

fn openat2(root: &File, relative: &Path, flags: libc::c_int) -> Result<File, String> {
    openat2_with_policy(root, relative, flags, RESOLVE_POLICY)
}

fn openat2_object(root: &File, relative: &Path, flags: libc::c_int) -> Result<File, String> {
    openat2_with_policy(root, relative, flags, OBJECT_RESOLVE_POLICY)
}

#[cfg(test)]
pub(super) fn open_object_beneath_filesystem_root(relative: &Path) -> Result<File, String> {
    let root = raw_open(c"/", libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)?;
    openat2_object(
        &root,
        relative,
        libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
    )
}

fn openat2_with_policy(
    root: &File,
    relative: &Path,
    flags: libc::c_int,
    resolve: u64,
) -> Result<File, String> {
    if relative.is_absolute() || relative.components().next().is_none() {
        return Err("openat2 path must be nonempty and relative".to_owned());
    }
    let path = CString::new(relative.as_os_str().as_bytes())
        .map_err(|_| "openat2 path contains NUL".to_owned())?;
    let how = OpenHow {
        flags: flags as u64,
        mode: 0,
        resolve,
    };
    // SAFETY: arguments point to initialized memory and the returned descriptor
    // is transferred to `File`. Unsupported kernels fail closed.
    let descriptor = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            root.as_raw_fd(),
            path.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        ) as libc::c_int
    };
    if descriptor < 0 {
        return Err(format!(
            "descriptor-relative openat2 failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: ownership of the newly returned descriptor transfers here.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn raw_open(path: &std::ffi::CStr, flags: libc::c_int) -> Result<File, String> {
    // SAFETY: `path` is NUL terminated and the returned descriptor is owned.
    let descriptor = unsafe { libc::open(path.as_ptr(), flags) };
    if descriptor < 0 {
        return Err(format!(
            "open held filesystem root: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: ownership of the newly returned descriptor transfers here.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn read_link_fd(file: &File) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0u8; MAX_ALIAS_BYTES + 1];
    // SAFETY: `file` is a held O_PATH symlink descriptor and `bytes` is writable.
    let length = unsafe {
        libc::readlinkat(
            file.as_raw_fd(),
            c"".as_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    if length < 0 {
        return Err(format!(
            "read held loader alias: {}",
            std::io::Error::last_os_error()
        ));
    }
    if length as usize == bytes.len() {
        return Err("loader alias target exceeds bounds".to_owned());
    }
    bytes.truncate(length as usize);
    Ok(bytes)
}

pub(super) fn require_cloexec(file: &File, label: &str) -> Result<(), String> {
    // SAFETY: fcntl operates on a live owned descriptor.
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
    if flags < 0 || flags & libc::FD_CLOEXEC == 0 {
        return Err(format!("{label} is not close-on-exec"));
    }
    Ok(())
}

fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and dereferences no memory.
    unsafe { libc::geteuid() }
}
