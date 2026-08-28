use sha2::{Digest, Sha256};

use std::fmt;
use std::io::Read;
use std::path::Path;

pub const MAX_ARTIFACT_BYTES: u64 = 1_073_741_824;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestError(pub String);

impl fmt::Display for DigestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DigestError {}

pub fn sha256_hex(input: &[u8]) -> String {
    format!("{:x}", Sha256::digest(input))
}

pub fn sha256_file(path: &Path) -> Result<String, DigestError> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| DigestError(format!("inspect {}: {error}", path.display())))?;
    if !metadata.is_file() || metadata.len() > MAX_ARTIFACT_BYTES {
        return Err(DigestError(format!(
            "{} is not a bounded regular artifact",
            path.display()
        )));
    }
    let mut file = std::fs::File::open(path)
        .map_err(|error| DigestError(format!("open {}: {error}", path.display())))?;
    let opened = file
        .metadata()
        .map_err(|error| DigestError(format!("inspect open {}: {error}", path.display())))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| DigestError(format!("read {}: {error}", path.display())))?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or_else(|| DigestError("artifact byte count overflow".into()))?;
        if total > MAX_ARTIFACT_BYTES {
            return Err(DigestError("artifact grew beyond byte bound".into()));
        }
        digest.update(&buffer[..count]);
    }
    let ending = std::fs::metadata(path)
        .map_err(|error| DigestError(format!("reinspect {}: {error}", path.display())))?;
    if !same_file(&opened, &ending) {
        return Err(DigestError("artifact changed while hashing".into()));
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(unix)]
fn same_file(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
}

#[cfg(not(unix))]
fn same_file(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    left.len() == right.len() && left.modified().ok() == right.modified().ok()
}
