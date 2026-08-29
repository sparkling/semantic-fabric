//! Immutable memfd snapshots derived only from already-held descriptors.

use std::fs::File;
use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::FileExt;

use sha2::{Digest, Sha256};

use super::{require_cloexec, FileIdentity};

const REQUIRED_SEALS: libc::c_int =
    libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;

#[derive(Debug)]
pub(in super::super) struct SealedBytes {
    pub(in super::super) file: File,
    pub(in super::super) sha256: String,
    pub(in super::super) byte_length: u64,
    pub(in super::super) bytes: Vec<u8>,
}

pub(in super::super) fn snapshot_source(
    source: &File,
    expected: FileIdentity,
    max_bytes: u64,
    label: &str,
) -> Result<SealedBytes, String> {
    snapshot_source_with_hook(source, expected, max_bytes, label, || {})
}

#[cfg(test)]
pub(in super::super) fn snapshot_source_with_phase_hook(
    source: &File,
    expected: FileIdentity,
    max_bytes: u64,
    label: &str,
    after_second_read: impl FnOnce(),
) -> Result<SealedBytes, String> {
    snapshot_source_with_hook(source, expected, max_bytes, label, after_second_read)
}

fn snapshot_source_with_hook(
    source: &File,
    expected: FileIdentity,
    max_bytes: u64,
    label: &str,
    after_second_read: impl FnOnce(),
) -> Result<SealedBytes, String> {
    if expected.size == 0 || expected.size > max_bytes {
        return Err(format!("{label} size is outside the sealed-byte budget"));
    }
    if FileIdentity::from_metadata(
        &source
            .metadata()
            .map_err(|error| format!("inspect {label} before snapshot: {error}"))?,
    ) != expected
    {
        return Err(format!("{label} identity changed before snapshot"));
    }
    let mut sealed = create_memfd()?;
    let (first_hash, first_bytes) = read_exact(source, expected.size, label)?;
    sealed
        .write_all(&first_bytes)
        .map_err(|error| format!("write runtime memfd: {error}"))?;
    let after_copy = FileIdentity::from_metadata(
        &source
            .metadata()
            .map_err(|error| format!("inspect {label} after snapshot: {error}"))?,
    );
    let (second_hash, second_bytes) = read_exact(source, expected.size, label)?;
    after_second_read();
    let after_rehash = FileIdentity::from_metadata(
        &source
            .metadata()
            .map_err(|error| format!("inspect {label} after rehash: {error}"))?,
    );
    if after_copy != expected
        || after_rehash != expected
        || first_hash != second_hash
        || first_bytes != second_bytes
    {
        return Err(format!("{label} changed while snapshotting"));
    }
    apply_seals(&sealed)?;
    let (sealed_hash, bytes) = read_exact(&sealed, expected.size, "sealed runtime bytes")?;
    if sealed_hash != first_hash || bytes != first_bytes {
        return Err(format!("{label} sealed-byte verification failed"));
    }
    Ok(SealedBytes {
        file: sealed,
        sha256: sealed_hash,
        byte_length: expected.size,
        bytes,
    })
}

pub(in super::super) fn assert_sealed_current(sealed: &SealedBytes) -> Result<(), String> {
    require_cloexec(&sealed.file, "sealed runtime bytes")?;
    require_exact_seals(&sealed.file)?;
    let (digest, bytes) = read_exact(&sealed.file, sealed.byte_length, "sealed runtime bytes")?;
    if digest != sealed.sha256 || bytes != sealed.bytes {
        return Err("sealed runtime bytes changed".to_owned());
    }
    Ok(())
}

pub(in super::super) fn assert_sealed_duplicate_current(
    duplicate: &File,
    sealed: &SealedBytes,
) -> Result<(), String> {
    require_cloexec(duplicate, "sealed runtime transfer")?;
    require_exact_seals(duplicate)?;
    let source_identity = FileIdentity::from_metadata(
        &sealed
            .file
            .metadata()
            .map_err(|error| format!("inspect sealed runtime source: {error}"))?,
    );
    let transfer_identity = FileIdentity::from_metadata(
        &duplicate
            .metadata()
            .map_err(|error| format!("inspect sealed runtime transfer: {error}"))?,
    );
    let (digest, bytes) = read_exact(duplicate, sealed.byte_length, "sealed runtime transfer")?;
    if transfer_identity != source_identity || digest != sealed.sha256 || bytes != sealed.bytes {
        return Err("sealed runtime transfer differs from its bound source".to_owned());
    }
    Ok(())
}

pub(super) fn read_exact(file: &File, size: u64, label: &str) -> Result<(String, Vec<u8>), String> {
    let capacity =
        usize::try_from(size).map_err(|_| format!("{label} size does not fit memory"))?;
    let mut bytes = vec![0u8; capacity];
    let mut offset = 0u64;
    while offset < size {
        let start =
            usize::try_from(offset).map_err(|_| format!("{label} offset does not fit memory"))?;
        let read = file
            .read_at(&mut bytes[start..], offset)
            .map_err(|error| format!("read {label}: {error}"))?;
        if read == 0 {
            return Err(format!("{label} became shorter while reading"));
        }
        offset = offset
            .checked_add(read as u64)
            .ok_or_else(|| format!("{label} read offset overflow"))?;
    }
    let mut extra = [0u8; 1];
    if file
        .read_at(&mut extra, size)
        .map_err(|error| format!("probe {label} length: {error}"))?
        != 0
    {
        return Err(format!("{label} grew while reading"));
    }
    Ok((format!("{:x}", Sha256::digest(&bytes)), bytes))
}

fn create_memfd() -> Result<File, String> {
    // SAFETY: the static name is NUL terminated; returned fd is owned.
    let descriptor = unsafe {
        libc::memfd_create(
            c"semantic-fabric-runtime-object".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if descriptor < 0 {
        return Err(format!(
            "create sealed runtime memfd: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: ownership of the newly returned descriptor transfers here.
    let file = unsafe { File::from_raw_fd(descriptor) };
    require_cloexec(&file, "runtime memfd")?;
    Ok(file)
}

fn apply_seals(file: &File) -> Result<(), String> {
    // SAFETY: fcntl operates on a live owned descriptor.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, REQUIRED_SEALS) } < 0 {
        return Err(format!(
            "seal runtime memfd: {}",
            std::io::Error::last_os_error()
        ));
    }
    require_exact_seals(file)
}

fn require_exact_seals(file: &File) -> Result<(), String> {
    // SAFETY: fcntl operates on a live owned descriptor.
    let seals = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GET_SEALS) };
    if seals != REQUIRED_SEALS {
        return Err("runtime memfd does not have the exact required seals".to_owned());
    }
    Ok(())
}
