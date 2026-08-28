use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::Path;

pub(super) fn read(path: &Path, max_bytes: u64) -> Result<Vec<u8>, String> {
    let path_metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect receipt authority {}: {error}", path.display()))?;
    validate(&path_metadata)?;
    let file = File::open(path)
        .map_err(|error| format!("open receipt authority {}: {error}", path.display()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("inspect opened receipt {}: {error}", path.display()))?;
    validate(&opened_metadata)?;
    if !same_file(&path_metadata, &opened_metadata) {
        return Err("receipt authority changed while opening".to_owned());
    }
    validate_path_identity(path, &opened_metadata)?;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read receipt authority {}: {error}", path.display()))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!("receipt exceeds {max_bytes} bytes"));
    }
    validate_path_identity(path, &opened_metadata)?;
    Ok(bytes)
}

pub(super) fn ensure_unchanged(path: &Path, expected: &[u8], max_bytes: u64) -> Result<(), String> {
    if read(path, max_bytes)? != expected {
        return Err("receipt authority changed during verification".to_owned());
    }
    Ok(())
}

fn validate(metadata: &Metadata) -> Result<(), String> {
    if metadata.file_type().is_symlink() {
        return Err("receipt authority is a symlink".to_owned());
    }
    if !metadata.is_file() {
        return Err("receipt authority is not a regular file".to_owned());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("receipt authority is a hard link".to_owned());
        }
    }
    Ok(())
}

fn validate_path_identity(path: &Path, opened: &Metadata) -> Result<(), String> {
    let current = fs::symlink_metadata(path)
        .map_err(|error| format!("re-inspect receipt authority {}: {error}", path.display()))?;
    validate(&current)?;
    if !same_file(&current, opened) {
        return Err("receipt authority changed during read".to_owned());
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn fixture_dir() -> std::path::PathBuf {
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "semantic-fabric-rust-closure-authority-{}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_and_hard_link_authorities() {
        use std::os::unix::fs::symlink;

        let directory = fixture_dir();
        let target = directory.join("target");
        let symlink_path = directory.join("symlink");
        let hard_link = directory.join("hard-link");
        fs::write(&target, b"authority").unwrap();
        symlink(&target, &symlink_path).unwrap();
        assert!(read(&symlink_path, 64).unwrap_err().contains("symlink"));
        fs::hard_link(&target, &hard_link).unwrap();
        assert!(read(&target, 64).unwrap_err().contains("hard link"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn detects_authority_mutation() {
        let directory = fixture_dir();
        let target = directory.join("target");
        fs::write(&target, b"before").unwrap();
        let before = read(&target, 64).unwrap();
        fs::write(&target, b"after").unwrap();
        let error = ensure_unchanged(&target, &before, 64).unwrap_err();
        assert!(error.contains("changed"), "{error}");
        fs::remove_dir_all(directory).unwrap();
    }
}
