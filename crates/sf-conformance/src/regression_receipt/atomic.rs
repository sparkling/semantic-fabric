use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const TEMP_ATTEMPTS: usize = 128;
static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

pub fn replace_fixed(
    root: &Path,
    relative: &str,
    expected_name: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let target = super::path::output_file(root, relative)?;
    if target.file_name() != Some(OsStr::new(expected_name)) {
        return Err(format!("baseline target must be named {expected_name}"));
    }
    let parent = target
        .parent()
        .ok_or_else(|| format!("baseline target {} has no parent", target.display()))?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|error| format!("canonicalize {}: {error}", parent.display()))?;
    if canonical_parent != parent || !canonical_parent.starts_with(root) {
        return Err(format!(
            "baseline parent {} is not a contained canonical directory",
            parent.display()
        ));
    }
    validate_target(&target)?;
    let (temporary, mut file) = create_temporary(parent, expected_name)?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|error| format!("write {}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("sync {}: {error}", temporary.display()))?;
        drop(file);
        if super::path::output_file(root, relative)? != target {
            return Err("fixed baseline target changed during generation".to_owned());
        }
        validate_target(&target)?;
        fs::rename(&temporary, &target).map_err(|error| {
            format!(
                "atomically replace {} from {}: {error}",
                target.display(),
                temporary.display()
            )
        })?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn create_temporary(parent: &Path, name: &str) -> Result<(PathBuf, File), String> {
    for _ in 0..TEMP_ATTEMPTS {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".{name}.tmp-{}-{serial}", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o644);
        }
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("create {}: {error}", path.display())),
        }
    }
    Err(format!(
        "could not create exclusive baseline temporary after {TEMP_ATTEMPTS} attempts"
    ))
}

fn validate_target(target: &Path) -> Result<(), String> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("baseline target is a symlink".to_owned())
        }
        Ok(metadata) if !metadata.is_file() => Err("baseline target is not a file".to_owned()),
        #[cfg(unix)]
        Ok(metadata)
            if {
                use std::os::unix::fs::MetadataExt;
                metadata.nlink() > 1
            } =>
        {
            Err("baseline target is a hard link".to_owned())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect {}: {error}", target.display())),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync directory {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}
