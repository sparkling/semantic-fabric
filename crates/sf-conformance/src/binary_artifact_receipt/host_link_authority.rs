//! Held authority for the narrow HostSystem final-leaf linker alias exception.

use std::fs::{self, File, Metadata, OpenOptions};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{
    authority_guard::{self, DirectoryGuard},
    producer_paths::HostAliasMapping,
};

#[derive(Debug)]
pub(super) struct HostLinkAuthority {
    pub(super) alias_receipt_path: String,
    pub(super) terminal_receipt_path: String,
    pub(super) resolution_sha256: String,
    pub(super) terminal_sha256: String,
    pub(super) terminal_byte_length: u64,
    pub(super) terminal_backing: PathBuf,
    alias_path: PathBuf,
    raw_target: Vec<u8>,
    alias_identity: FileIdentity,
    alias_file: File,
    alias_root: DirectoryGuard,
    terminal_root: DirectoryGuard,
    terminal: HeldRegular,
}

impl HostLinkAuthority {
    pub(super) fn bind(mapping: HostAliasMapping, max_bytes: u64) -> Result<Self, String> {
        #[cfg(not(target_os = "linux"))]
        return Err("host link authority requires Linux O_PATH semantics".to_owned());

        let alias_root = DirectoryGuard::bind(&mapping.alias_root, "host link alias root")?;
        let terminal_root =
            DirectoryGuard::bind(&mapping.terminal_root, "host link terminal root")?;
        authority_guard::validate_parent_ancestry(&mapping.alias_backing, "host link alias")?;
        let before = fs::symlink_metadata(&mapping.alias_backing).map_err(|error| {
            format!(
                "inspect host link alias {}: {error}",
                mapping.alias_backing.display()
            )
        })?;
        validate_symlink(&before)?;
        let alias_file = open_symlink(&mapping.alias_backing)?;
        let opened = alias_file
            .metadata()
            .map_err(|error| format!("inspect held host link alias: {error}"))?;
        validate_symlink(&opened)?;
        let alias_identity = FileIdentity::from_metadata(&opened);
        if FileIdentity::from_metadata(&before) != alias_identity {
            return Err("host link alias changed while opening".to_owned());
        }
        let raw_target = read_link_fd(&alias_file)?;
        if raw_target != mapping.raw_target
            || raw_path_bytes(&fs::read_link(&mapping.alias_backing).map_err(|error| {
                format!(
                    "read host link alias {}: {error}",
                    mapping.alias_backing.display()
                )
            })?)?
                != raw_target
        {
            return Err("host link alias target changed while binding".to_owned());
        }
        let after = fs::symlink_metadata(&mapping.alias_backing)
            .map_err(|error| format!("reinspect host link alias: {error}"))?;
        if FileIdentity::from_metadata(&after) != alias_identity {
            return Err("host link alias changed while binding".to_owned());
        }

        let terminal = HeldRegular::bind(&mapping.terminal.backing, max_bytes)?;
        let terminal_logical = mapping
            .terminal_logical
            .to_str()
            .ok_or_else(|| "host link terminal logical path is not UTF-8".to_owned())?;
        let resolution_sha256 = resolution_digest(
            &mapping.alias_receipt_path,
            &raw_target,
            terminal_logical,
            &mapping.terminal.receipt_path,
        );
        let authority = Self {
            alias_receipt_path: mapping.alias_receipt_path,
            terminal_receipt_path: mapping.terminal.receipt_path,
            resolution_sha256,
            terminal_sha256: terminal.sha256.clone(),
            terminal_byte_length: terminal.byte_length,
            terminal_backing: mapping.terminal.backing,
            alias_path: mapping.alias_backing,
            raw_target,
            alias_identity,
            alias_file,
            alias_root,
            terminal_root,
            terminal,
        };
        authority.assert_current()?;
        Ok(authority)
    }

    pub(super) fn assert_current(&self) -> Result<(), String> {
        self.alias_root.assert_current()?;
        self.terminal_root.assert_current()?;
        authority_guard::validate_parent_ancestry(&self.alias_path, "host link alias")?;
        let path_metadata = fs::symlink_metadata(&self.alias_path)
            .map_err(|error| format!("reinspect host link alias: {error}"))?;
        validate_symlink(&path_metadata)?;
        let held_metadata = self
            .alias_file
            .metadata()
            .map_err(|error| format!("inspect held host link alias: {error}"))?;
        validate_symlink(&held_metadata)?;
        if FileIdentity::from_metadata(&path_metadata) != self.alias_identity
            || FileIdentity::from_metadata(&held_metadata) != self.alias_identity
            || read_link_fd(&self.alias_file)? != self.raw_target
            || raw_path_bytes(&fs::read_link(&self.alias_path).map_err(|error| {
                format!(
                    "reread host link alias {}: {error}",
                    self.alias_path.display()
                )
            })?)?
                != self.raw_target
        {
            return Err("host link alias authority changed".to_owned());
        }
        self.terminal.assert_current()?;
        self.alias_root.assert_current()?;
        self.terminal_root.assert_current()
    }
}

#[derive(Debug)]
struct HeldRegular {
    path: PathBuf,
    file: File,
    identity: FileIdentity,
    sha256: String,
    byte_length: u64,
    max_bytes: u64,
}

impl HeldRegular {
    fn bind(path: &Path, max_bytes: u64) -> Result<Self, String> {
        authority_guard::validate_parent_ancestry(path, "host link terminal")?;
        let before = fs::symlink_metadata(path)
            .map_err(|error| format!("inspect host link terminal {}: {error}", path.display()))?;
        validate_regular(&before)?;
        let file = open_regular(path)?;
        let opened = file
            .metadata()
            .map_err(|error| format!("inspect held host link terminal: {error}"))?;
        validate_regular(&opened)?;
        let identity = FileIdentity::from_metadata(&opened);
        if FileIdentity::from_metadata(&before) != identity {
            return Err("host link terminal changed while opening".to_owned());
        }
        let (sha256, byte_length) = digest_file(&file, max_bytes)?;
        let after = fs::symlink_metadata(path)
            .map_err(|error| format!("reinspect host link terminal: {error}"))?;
        if FileIdentity::from_metadata(&after) != identity || after.len() != byte_length {
            return Err("host link terminal changed while hashing".to_owned());
        }
        Ok(Self {
            path: path.to_path_buf(),
            file,
            identity,
            sha256,
            byte_length,
            max_bytes,
        })
    }

    fn assert_current(&self) -> Result<(), String> {
        authority_guard::validate_parent_ancestry(&self.path, "host link terminal")?;
        let current = fs::symlink_metadata(&self.path)
            .map_err(|error| format!("reinspect host link terminal: {error}"))?;
        validate_regular(&current)?;
        let held = self
            .file
            .metadata()
            .map_err(|error| format!("inspect held host link terminal: {error}"))?;
        validate_regular(&held)?;
        let (sha256, byte_length) = digest_file(&self.file, self.max_bytes)?;
        if FileIdentity::from_metadata(&current) != self.identity
            || FileIdentity::from_metadata(&held) != self.identity
            || byte_length != self.byte_length
            || sha256 != self.sha256
        {
            return Err("host link terminal authority changed".to_owned());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    nlink: u64,
    size: u64,
}

impl FileIdentity {
    #[cfg(unix)]
    fn from_metadata(metadata: &Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            nlink: metadata.nlink(),
            size: metadata.size(),
        }
    }

    #[cfg(not(unix))]
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            dev: 0,
            ino: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            nlink: 1,
            size: metadata.len(),
        }
    }
}

fn validate_symlink(metadata: &Metadata) -> Result<(), String> {
    if !metadata.file_type().is_symlink() {
        return Err("host link alias is not a symlink".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: `geteuid` has no preconditions and dereferences no memory.
        let effective_uid = unsafe { libc::geteuid() };
        if metadata.nlink() != 1 || (metadata.uid() != 0 && metadata.uid() != effective_uid) {
            return Err("host link alias has untrusted owner or link count".to_owned());
        }
    }
    Ok(())
}

fn validate_regular(metadata: &Metadata) -> Result<(), String> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("host link terminal is not a regular non-symlink file".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("host link terminal is a hard link".to_owned());
        }
    }
    authority_guard::validate_leaf(metadata, "host link terminal")
}

fn open_regular(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    options
        .open(path)
        .map_err(|error| format!("open host link terminal {}: {error}", path.display()))
}

#[cfg(target_os = "linux")]
fn open_symlink(path: &Path) -> Result<File, String> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "host link alias path contains NUL".to_owned())?;
    // SAFETY: `path` is a valid NUL-terminated string; returned fd is owned.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(format!(
            "open host link alias: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: `fd` is newly returned and ownership transfers to `File`.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(not(target_os = "linux"))]
fn open_symlink(_path: &Path) -> Result<File, String> {
    Err("host link authority requires Linux O_PATH semantics".to_owned())
}

#[cfg(target_os = "linux")]
fn read_link_fd(file: &File) -> Result<Vec<u8>, String> {
    use std::os::fd::AsRawFd;

    let mut bytes = vec![0u8; 16 * 1024 + 1];
    // SAFETY: the fd denotes an O_PATH symlink; buffer is valid and writable.
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
            "read held host link alias: {}",
            std::io::Error::last_os_error()
        ));
    }
    if length as usize == bytes.len() {
        return Err("host link alias target exceeds bounds".to_owned());
    }
    bytes.truncate(length as usize);
    Ok(bytes)
}

#[cfg(not(target_os = "linux"))]
fn read_link_fd(_file: &File) -> Result<Vec<u8>, String> {
    Err("host link authority requires Linux readlinkat semantics".to_owned())
}

#[cfg(unix)]
fn digest_file(file: &File, max_bytes: u64) -> Result<(String, u64), String> {
    use std::os::unix::fs::FileExt;

    let mut digest = Sha256::new();
    let mut offset = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read_at(&mut buffer, offset)
            .map_err(|error| format!("read held host link terminal: {error}"))?;
        if read == 0 {
            break;
        }
        offset = offset
            .checked_add(read as u64)
            .ok_or_else(|| "host link terminal size overflow".to_owned())?;
        if offset > max_bytes {
            return Err(format!("host link terminal exceeds {max_bytes} bytes"));
        }
        digest.update(&buffer[..read]);
    }
    Ok((format!("{:x}", digest.finalize()), offset))
}

#[cfg(not(unix))]
fn digest_file(_file: &File, _max_bytes: u64) -> Result<(String, u64), String> {
    Err("host link authority requires Unix descriptor reads".to_owned())
}

#[cfg(unix)]
fn raw_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    use std::os::unix::ffi::OsStrExt;
    Ok(path.as_os_str().as_bytes().to_vec())
}

#[cfg(not(unix))]
fn raw_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    path.to_str()
        .map(str::as_bytes)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| "host link alias target is not UTF-8".to_owned())
}

fn resolution_digest(alias: &str, raw: &[u8], terminal: &str, receipt: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"semantic-fabric:host-link-alias-resolution:v1\0");
    for value in [
        alias.as_bytes(),
        raw,
        terminal.as_bytes(),
        receipt.as_bytes(),
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value);
    }
    format!("{:x}", digest.finalize())
}

#[cfg(all(test, unix))]
#[path = "host_link_authority/tests.rs"]
mod tests;
