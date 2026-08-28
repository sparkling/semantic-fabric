use std::fs::{self, File, Metadata, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

const MAX_EXECUTABLE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct ToolAuthority {
    trusted_uid: u32,
    directories: Vec<BoundDirectory>,
    tools: [BoundTool; 2],
}

impl ToolAuthority {
    pub(super) fn bind(toolchain: &Path, cargo: &Path, rustc: &Path) -> Result<Self, String> {
        let trusted_uid = effective_uid()?;
        let authority = Self {
            trusted_uid,
            directories: bind_toolchain_ancestry(toolchain, trusted_uid)?,
            tools: [
                BoundTool::bind(cargo, "Cargo executable", trusted_uid)?,
                BoundTool::bind(rustc, "rustc executable", trusted_uid)?,
            ],
        };
        authority.ensure_unchanged()?;
        Ok(authority)
    }

    pub(super) fn ensure_unchanged(&self) -> Result<(), String> {
        if effective_uid()? != self.trusted_uid {
            return Err("effective UID changed during closure verification".to_owned());
        }
        for directory in &self.directories {
            directory.ensure_unchanged()?;
        }
        for tool in &self.tools {
            tool.ensure_unchanged()?;
        }
        for directory in &self.directories {
            directory.ensure_unchanged()?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn current_tools(&self) -> Result<[ToolIdentity; 2], String> {
        Ok([
            ToolIdentity::read(&self.tools[0].path, self.tools[0].label, self.trusted_uid)?,
            ToolIdentity::read(&self.tools[1].path, self.tools[1].label, self.trusted_uid)?,
        ])
    }
}

fn bind_toolchain_ancestry(
    toolchain: &Path,
    trusted_uid: u32,
) -> Result<Vec<BoundDirectory>, String> {
    let mut paths: Vec<_> = toolchain.ancestors().collect();
    paths.reverse();
    let mut directories = Vec::with_capacity(paths.len() + 1);
    for path in paths {
        let is_toolchain_root = path == toolchain;
        directories.push(BoundDirectory::bind(
            path,
            if is_toolchain_root {
                "toolchain root"
            } else {
                "toolchain ancestor"
            },
            trusted_uid,
            !is_toolchain_root,
        )?);
    }
    directories.push(BoundDirectory::bind(
        &toolchain.join("bin"),
        "toolchain bin",
        trusted_uid,
        false,
    )?);
    Ok(directories)
}

#[derive(Clone)]
struct BoundDirectory {
    path: PathBuf,
    label: &'static str,
    trusted_uid: u32,
    allow_root_sticky: bool,
    identity: DirectoryIdentity,
}

impl BoundDirectory {
    fn bind(
        path: &Path,
        label: &'static str,
        trusted_uid: u32,
        allow_root_sticky: bool,
    ) -> Result<Self, String> {
        let canonical = fs::canonicalize(path)
            .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))?;
        if canonical != path {
            return Err(format!("{label} is not canonical"));
        }
        Ok(Self {
            path: path.to_path_buf(),
            label,
            trusted_uid,
            allow_root_sticky,
            identity: directory_identity(path, label, trusted_uid, allow_root_sticky)?,
        })
    }

    fn ensure_unchanged(&self) -> Result<(), String> {
        ensure_canonical(&self.path, self.label)?;
        if directory_identity(
            &self.path,
            self.label,
            self.trusted_uid,
            self.allow_root_sticky,
        )? != self.identity
        {
            return Err(format!(
                "{} changed during closure verification",
                self.label
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct BoundTool {
    path: PathBuf,
    label: &'static str,
    trusted_uid: u32,
    identity: ToolIdentity,
}

impl BoundTool {
    fn bind(path: &Path, label: &'static str, trusted_uid: u32) -> Result<Self, String> {
        let canonical = fs::canonicalize(path)
            .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))?;
        if canonical != path {
            return Err(format!("{label} is not canonical"));
        }
        Ok(Self {
            path: path.to_path_buf(),
            label,
            trusted_uid,
            identity: ToolIdentity::read(path, label, trusted_uid)?,
        })
    }

    fn ensure_unchanged(&self) -> Result<(), String> {
        ensure_canonical(&self.path, self.label)?;
        if ToolIdentity::read(&self.path, self.label, self.trusted_uid)? != self.identity {
            return Err(format!(
                "{} changed during closure verification",
                self.label
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ToolIdentity {
    node: NodeIdentity,
    sha256: String,
    byte_length: u64,
}

impl ToolIdentity {
    fn read(path: &Path, label: &str, trusted_uid: u32) -> Result<Self, String> {
        let path_metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("inspect {label} {}: {error}", path.display()))?;
        validate_executable(&path_metadata, label, trusted_uid)?;
        let path_node = NodeIdentity::from_metadata(&path_metadata);
        let mut file =
            open_tool(path).map_err(|error| format!("open {label} {}: {error}", path.display()))?;
        let opened_metadata = file
            .metadata()
            .map_err(|error| format!("inspect opened {label} {}: {error}", path.display()))?;
        validate_executable(&opened_metadata, label, trusted_uid)?;
        if NodeIdentity::from_metadata(&opened_metadata) != path_node {
            return Err(format!("{label} changed while opening"));
        }
        let (sha256, byte_length) = digest(&mut file, label)?;
        if byte_length != path_node.byte_length {
            return Err(format!("{label} changed during read"));
        }
        let opened_after = file
            .metadata()
            .map_err(|error| format!("re-inspect opened {label}: {error}"))?;
        validate_executable(&opened_after, label, trusted_uid)?;
        if NodeIdentity::from_metadata(&opened_after) != path_node {
            return Err(format!("{label} changed during read"));
        }
        let current = fs::symlink_metadata(path)
            .map_err(|error| format!("re-inspect {label} {}: {error}", path.display()))?;
        validate_executable(&current, label, trusted_uid)?;
        if NodeIdentity::from_metadata(&current) != path_node {
            return Err(format!("{label} path changed during read"));
        }
        Ok(Self {
            node: path_node,
            sha256,
            byte_length,
        })
    }
}

fn open_tool(path: &Path) -> Result<File, std::io::Error> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    options.open(path)
}

fn ensure_canonical(path: &Path, label: &str) -> Result<(), String> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))?;
    if canonical != path {
        return Err(format!("{label} path changed or is not canonical"));
    }
    Ok(())
}

fn digest(file: &mut File, label: &str) -> Result<(String, u64), String> {
    let mut digest = Sha256::new();
    let mut byte_length = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read {label}: {error}"))?;
        if read == 0 {
            break;
        }
        byte_length = byte_length
            .checked_add(read as u64)
            .ok_or_else(|| format!("{label} size overflow"))?;
        if byte_length > MAX_EXECUTABLE_BYTES {
            return Err(format!("{label} exceeds its byte bound"));
        }
        digest.update(&buffer[..read]);
    }
    Ok((format!("{:x}", digest.finalize()), byte_length))
}

fn directory_identity(
    path: &Path,
    label: &str,
    trusted_uid: u32,
    allow_root_sticky: bool,
) -> Result<DirectoryIdentity, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect {label} {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{label} is not a non-symlink directory"));
    }
    reject_untrusted_owner(&metadata, label, trusted_uid)?;
    reject_insecure_directory(&metadata, label, allow_root_sticky)?;
    Ok(DirectoryIdentity::from_metadata(&metadata))
}

fn validate_executable(metadata: &Metadata, label: &str, trusted_uid: u32) -> Result<(), String> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{label} is not a non-symlink regular file"));
    }
    if metadata.len() > MAX_EXECUTABLE_BYTES {
        return Err(format!("{label} exceeds its byte bound"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(format!("{label} is a hard link"));
        }
        if metadata.mode() & 0o111 == 0 {
            return Err(format!("{label} is not executable"));
        }
    }
    #[cfg(not(unix))]
    return Err(format!("{label} requires Unix identity metadata"));
    reject_untrusted_owner(metadata, label, trusted_uid)?;
    reject_writable(metadata, label)
}

fn reject_untrusted_owner(
    metadata: &Metadata,
    label: &str,
    trusted_uid: u32,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != 0 && metadata.uid() != trusted_uid {
            return Err(format!("{label} is foreign-owned"));
        }
    }
    Ok(())
}

fn reject_insecure_directory(
    metadata: &Metadata,
    label: &str,
    allow_root_sticky: bool,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mode = metadata.mode();
        let root_sticky_exception = root_sticky_exception(metadata.uid(), mode, allow_root_sticky);
        if mode & 0o022 != 0 && !root_sticky_exception {
            return Err(format!("{label} is group- or world-writable"));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn root_sticky_exception(owner: u32, mode: u32, allowed: bool) -> bool {
    allowed && owner == 0 && mode & libc::S_ISVTX != 0
}

fn reject_writable(metadata: &Metadata, label: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o022 != 0 {
            return Err(format!("{label} is group- or world-writable"));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn effective_uid() -> Result<u32, String> {
    // SAFETY: geteuid reads process credentials and has no preconditions.
    Ok(unsafe { libc::geteuid() })
}

#[cfg(not(unix))]
fn effective_uid() -> Result<u32, String> {
    Err("controlled tool authority requires Unix identity metadata".to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectoryIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    mode: u32,
    #[cfg(unix)]
    owner: u32,
    #[cfg(unix)]
    group: u32,
}

impl DirectoryIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        #[cfg(not(unix))]
        let _ = metadata;
        Self {
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            mode: metadata.mode(),
            #[cfg(unix)]
            owner: metadata.uid(),
            #[cfg(unix)]
            group: metadata.gid(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeIdentity {
    byte_length: u64,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    mode: u32,
    #[cfg(unix)]
    owner: u32,
    #[cfg(unix)]
    group: u32,
    #[cfg(unix)]
    modified_seconds: i64,
    #[cfg(unix)]
    modified_nanoseconds: i64,
    #[cfg(unix)]
    changed_seconds: i64,
    #[cfg(unix)]
    changed_nanoseconds: i64,
}

impl NodeIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            byte_length: metadata.len(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            mode: metadata.mode(),
            #[cfg(unix)]
            owner: metadata.uid(),
            #[cfg(unix)]
            group: metadata.gid(),
            #[cfg(unix)]
            modified_seconds: metadata.mtime(),
            #[cfg(unix)]
            modified_nanoseconds: metadata.mtime_nsec(),
            #[cfg(unix)]
            changed_seconds: metadata.ctime(),
            #[cfg(unix)]
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use super::*;

    #[test]
    fn rejects_toolchain_root_owned_by_an_untrusted_uid() {
        let path = std::env::temp_dir().join(format!(
            "semantic-fabric-foreign-toolchain-owner-{}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let owner = fs::symlink_metadata(&path).unwrap().uid();
        let untrusted_uid = owner.checked_add(1).unwrap_or(owner - 1);
        let error = directory_identity(&path, "toolchain root", untrusted_uid, false).unwrap_err();
        assert!(error.contains("foreign-owned"), "{error}");
        fs::remove_dir(path).unwrap();
    }

    #[test]
    fn limits_root_sticky_exception_to_strict_ancestors() {
        assert_eq!(
            [
                root_sticky_exception(0, 0o1777, true),
                root_sticky_exception(0, 0o1777, false),
            ],
            [true, false]
        );
    }
}
