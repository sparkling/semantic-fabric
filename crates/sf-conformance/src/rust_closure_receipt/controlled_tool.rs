use std::fs::{self, File, Metadata, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

const MAX_EXECUTABLE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct ToolAuthority {
    directories: [BoundDirectory; 2],
    tools: [BoundTool; 2],
}

impl ToolAuthority {
    pub(super) fn bind(toolchain: &Path, cargo: &Path, rustc: &Path) -> Result<Self, String> {
        let authority = Self {
            directories: [
                BoundDirectory::bind(toolchain, "toolchain root")?,
                BoundDirectory::bind(&toolchain.join("bin"), "toolchain bin")?,
            ],
            tools: [
                BoundTool::bind(cargo, "Cargo executable")?,
                BoundTool::bind(rustc, "rustc executable")?,
            ],
        };
        authority.ensure_unchanged()?;
        Ok(authority)
    }

    pub(super) fn ensure_unchanged(&self) -> Result<(), String> {
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
            ToolIdentity::read(&self.tools[0].path, self.tools[0].label)?,
            ToolIdentity::read(&self.tools[1].path, self.tools[1].label)?,
        ])
    }
}

#[derive(Clone)]
struct BoundDirectory {
    path: PathBuf,
    label: &'static str,
    identity: NodeIdentity,
}

impl BoundDirectory {
    fn bind(path: &Path, label: &'static str) -> Result<Self, String> {
        let canonical = fs::canonicalize(path)
            .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))?;
        if canonical != path {
            return Err(format!("{label} is not canonical"));
        }
        Ok(Self {
            path: path.to_path_buf(),
            label,
            identity: directory_identity(path, label)?,
        })
    }

    fn ensure_unchanged(&self) -> Result<(), String> {
        ensure_canonical(&self.path, self.label)?;
        if directory_identity(&self.path, self.label)? != self.identity {
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
    identity: ToolIdentity,
}

impl BoundTool {
    fn bind(path: &Path, label: &'static str) -> Result<Self, String> {
        let canonical = fs::canonicalize(path)
            .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))?;
        if canonical != path {
            return Err(format!("{label} is not canonical"));
        }
        Ok(Self {
            path: path.to_path_buf(),
            label,
            identity: ToolIdentity::read(path, label)?,
        })
    }

    fn ensure_unchanged(&self) -> Result<(), String> {
        ensure_canonical(&self.path, self.label)?;
        if ToolIdentity::read(&self.path, self.label)? != self.identity {
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
    fn read(path: &Path, label: &str) -> Result<Self, String> {
        let path_metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("inspect {label} {}: {error}", path.display()))?;
        validate_executable(&path_metadata, label)?;
        let path_node = NodeIdentity::from_metadata(&path_metadata);
        let mut file =
            open_tool(path).map_err(|error| format!("open {label} {}: {error}", path.display()))?;
        let opened_metadata = file
            .metadata()
            .map_err(|error| format!("inspect opened {label} {}: {error}", path.display()))?;
        validate_executable(&opened_metadata, label)?;
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
        validate_executable(&opened_after, label)?;
        if NodeIdentity::from_metadata(&opened_after) != path_node {
            return Err(format!("{label} changed during read"));
        }
        let current = fs::symlink_metadata(path)
            .map_err(|error| format!("re-inspect {label} {}: {error}", path.display()))?;
        validate_executable(&current, label)?;
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

fn directory_identity(path: &Path, label: &str) -> Result<NodeIdentity, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect {label} {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{label} is not a non-symlink directory"));
    }
    reject_writable(&metadata, label)?;
    Ok(NodeIdentity::from_metadata(&metadata))
}

fn validate_executable(metadata: &Metadata, label: &str) -> Result<(), String> {
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
    reject_writable(metadata, label)
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
