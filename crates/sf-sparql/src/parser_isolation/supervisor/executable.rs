//! Held identity for the already-running product executable.

use std::fs::{File, OpenOptions};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{FileExt, OpenOptionsExt};

use sha2::{Digest, Sha256};

use super::SupervisorError;

const ELF_MAGIC: [u8; 4] = *b"\x7fELF";
pub(super) const MAX_EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;

/// A SHA-256 observation of bytes read from the held descriptor.
///
/// This is correlation material only. It is not a signature, release
/// provenance, build authority, or an attestation of dynamically loaded code.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct ObservedExecutableFingerprint([u8; 32]);

impl ObservedExecutableFingerprint {
    pub(super) const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

impl std::fmt::Debug for ObservedExecutableFingerprint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ObservedExecutableFingerprint(<non-authoritative>)")
    }
}

/// Kernel identity and observed bytes for one already-open executable inode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct HeldExecutableIdentity {
    device: u64,
    inode: u64,
    byte_len: u64,
    mode: u32,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
    fingerprint: ObservedExecutableFingerprint,
}

impl HeldExecutableIdentity {
    pub(super) const fn device(self) -> u64 {
        self.device
    }

    pub(super) const fn inode(self) -> u64 {
        self.inode
    }

    pub(super) const fn byte_len(self) -> u64 {
        self.byte_len
    }

    pub(super) const fn mode(self) -> u32 {
        self.mode
    }

    pub(super) const fn fingerprint(self) -> ObservedExecutableFingerprint {
        self.fingerprint
    }
}

/// Owns the sole path resolution used by this supervisor.
///
/// `/proc/self/exe` is opened exactly once. Launches duplicate this descriptor;
/// they never read the symlink target or reopen a derived filesystem path.
pub(super) struct PreparedParserExecutable {
    file: File,
    identity: HeldExecutableIdentity,
}

impl PreparedParserExecutable {
    pub(super) fn current() -> Result<Self, SupervisorError> {
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            return Err(SupervisorError::UnsupportedPlatform);
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_CLOEXEC)
                .open("/proc/self/exe")
                .map_err(SupervisorError::operation("open current executable"))?;
            Self::prepare(file, true)
        }
    }

    pub(super) const fn identity(&self) -> HeldExecutableIdentity {
        self.identity
    }

    pub(super) fn duplicate_for_launch(&self, minimum: i32) -> Result<OwnedFd, SupervisorError> {
        if minimum < 3 {
            return Err(SupervisorError::InvalidLimits(
                "launch descriptor floor is below the private descriptor range",
            ));
        }
        // SAFETY: fcntl duplicates this live descriptor and atomically applies
        // CLOEXEC; ownership of a successful result transfers below.
        let descriptor = unsafe {
            libc::fcntl(
                self.file.as_raw_fd(),
                libc::F_DUPFD_CLOEXEC,
                minimum as libc::c_int,
            )
        };
        if descriptor < 0 {
            return Err(SupervisorError::operation(
                "duplicate held executable for launch",
            )(std::io::Error::last_os_error()));
        }
        // SAFETY: `descriptor` is a new fd returned by F_DUPFD_CLOEXEC.
        let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
        let duplicate_identity = metadata_identity(descriptor.as_raw_fd())?;
        if duplicate_identity != self.identity_without_fingerprint() {
            return Err(SupervisorError::InvalidExecutable(
                "launch descriptor identity drifted",
            ));
        }
        Ok(descriptor)
    }

    fn prepare(file: File, require_elf: bool) -> Result<Self, SupervisorError> {
        require_cloexec(file.as_raw_fd())?;
        let before = metadata_identity(file.as_raw_fd())?;
        if before.byte_len == 0 || before.mode & libc::S_IFMT != libc::S_IFREG {
            return Err(SupervisorError::InvalidExecutable(
                "descriptor is not a non-empty regular file",
            ));
        }
        if before.byte_len > MAX_EXECUTABLE_BYTES {
            return Err(SupervisorError::InvalidExecutable(
                "descriptor exceeds the fixed fingerprint byte ceiling",
            ));
        }
        if before.mode & 0o111 == 0 {
            return Err(SupervisorError::InvalidExecutable(
                "descriptor has no executable mode bit",
            ));
        }
        if require_elf {
            let mut magic = [0_u8; ELF_MAGIC.len()];
            file.read_exact_at(&mut magic, 0)
                .map_err(SupervisorError::operation("read executable ELF identity"))?;
            if magic != ELF_MAGIC {
                return Err(SupervisorError::InvalidExecutable(
                    "CLOEXEC descriptor execution requires an ELF image",
                ));
            }
        }
        let fingerprint = fingerprint(&file, before.byte_len)?;
        let after = metadata_identity(file.as_raw_fd())?;
        if before != after {
            return Err(SupervisorError::InvalidExecutable(
                "descriptor metadata changed while fingerprinting",
            ));
        }
        Ok(Self {
            file,
            identity: HeldExecutableIdentity {
                device: before.device,
                inode: before.inode,
                byte_len: before.byte_len,
                mode: before.mode,
                modified_seconds: before.modified_seconds,
                modified_nanoseconds: before.modified_nanoseconds,
                changed_seconds: before.changed_seconds,
                changed_nanoseconds: before.changed_nanoseconds,
                fingerprint,
            },
        })
    }

    fn identity_without_fingerprint(&self) -> MetadataIdentity {
        MetadataIdentity {
            device: self.identity.device,
            inode: self.identity.inode,
            byte_len: self.identity.byte_len,
            mode: self.identity.mode,
            modified_seconds: self.identity.modified_seconds,
            modified_nanoseconds: self.identity.modified_nanoseconds,
            changed_seconds: self.identity.changed_seconds,
            changed_nanoseconds: self.identity.changed_nanoseconds,
        }
    }

    #[cfg(test)]
    pub(super) fn from_file_for_test(
        file: File,
        require_elf: bool,
    ) -> Result<Self, SupervisorError> {
        Self::prepare(file, require_elf)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MetadataIdentity {
    device: u64,
    inode: u64,
    byte_len: u64,
    mode: u32,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

fn metadata_identity(descriptor: RawFd) -> Result<MetadataIdentity, SupervisorError> {
    // SAFETY: zero is a valid initialization for stat and fstat fills it on
    // success using the supplied live descriptor.
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    // SAFETY: `metadata` is writable and `descriptor` remains live.
    if unsafe { libc::fstat(descriptor, &mut metadata) } != 0 {
        return Err(SupervisorError::operation("inspect held executable")(
            std::io::Error::last_os_error(),
        ));
    }
    Ok(MetadataIdentity {
        device: metadata.st_dev,
        inode: metadata.st_ino,
        byte_len: metadata
            .st_size
            .try_into()
            .map_err(|_| SupervisorError::InvalidExecutable("descriptor length is negative"))?,
        mode: metadata.st_mode,
        modified_seconds: metadata.st_mtime,
        modified_nanoseconds: metadata.st_mtime_nsec,
        changed_seconds: metadata.st_ctime,
        changed_nanoseconds: metadata.st_ctime_nsec,
    })
}

fn require_cloexec(descriptor: RawFd) -> Result<(), SupervisorError> {
    // SAFETY: F_GETFD only inspects the supplied live descriptor.
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
    if flags < 0 {
        return Err(SupervisorError::operation(
            "inspect held executable descriptor flags",
        )(std::io::Error::last_os_error()));
    }
    if flags & libc::FD_CLOEXEC == 0 {
        return Err(SupervisorError::InvalidExecutable(
            "descriptor is not close-on-exec",
        ));
    }
    Ok(())
}

fn fingerprint(
    file: &File,
    expected_len: u64,
) -> Result<ObservedExecutableFingerprint, SupervisorError> {
    let mut digest = Sha256::new();
    let mut offset = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    while offset < expected_len {
        let remaining = expected_len - offset;
        let requested = usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| {
            SupervisorError::InvalidExecutable("fingerprint read length does not fit memory")
        })?;
        let count = file
            .read_at(&mut buffer[..requested], offset)
            .map_err(SupervisorError::operation("fingerprint held executable"))?;
        if count == 0 {
            return Err(SupervisorError::InvalidExecutable(
                "descriptor became shorter while fingerprinting",
            ));
        }
        digest.update(&buffer[..count]);
        offset = offset
            .checked_add(count as u64)
            .ok_or(SupervisorError::InvalidExecutable(
                "fingerprint offset overflow",
            ))?;
    }
    let mut trailing = [0_u8; 1];
    if file
        .read_at(&mut trailing, expected_len)
        .map_err(SupervisorError::operation(
            "verify held executable fingerprint length",
        ))?
        != 0
    {
        return Err(SupervisorError::InvalidExecutable(
            "descriptor grew while fingerprinting",
        ));
    }
    Ok(ObservedExecutableFingerprint(digest.finalize().into()))
}
