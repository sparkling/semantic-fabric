//! Descriptor-relative component walk for runtime leaves.

use std::ffi::OsString;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Component, Path};

use super::{openat2_object, require_cloexec, validate_directory, HeldMount};

#[derive(Debug)]
pub(in super::super) struct HeldLeaf {
    parent: File,
    name: OsString,
    pub(in super::super) handle: File,
}

pub(in super::super) fn open_leaf(mount: &HeldMount, relative: &Path) -> Result<HeldLeaf, String> {
    let components: Vec<OsString> = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value.to_os_string()),
            _ => Err("runtime path contains a non-normal component".to_owned()),
        })
        .collect::<Result<_, _>>()?;
    let (name, ancestors) = components
        .split_last()
        .ok_or_else(|| "runtime path has no leaf component".to_owned())?;
    let mut parent = duplicate_cloexec(&mount.root, "runtime mount root")?;
    for ancestor in ancestors {
        let directory = openat2_object(
            &parent,
            Path::new(ancestor),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )?;
        require_cloexec(&directory, "runtime path ancestor")?;
        let metadata = directory
            .metadata()
            .map_err(|error| format!("inspect runtime path ancestor: {error}"))?;
        validate_directory(&metadata)?;
        parent = directory;
    }
    let handle = openat2_object(
        &parent,
        Path::new(name),
        libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
    )?;
    require_cloexec(&handle, "held runtime path")?;
    Ok(HeldLeaf {
        parent,
        name: name.clone(),
        handle,
    })
}

pub(in super::super) fn open_regular(leaf: &HeldLeaf) -> Result<File, String> {
    let file = openat2_object(
        &leaf.parent,
        Path::new(&leaf.name),
        libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
    )?;
    require_cloexec(&file, "held runtime regular file")?;
    Ok(file)
}

fn duplicate_cloexec(file: &File, label: &str) -> Result<File, String> {
    // SAFETY: `file` is live and F_DUPFD_CLOEXEC returns an independently
    // owned close-on-exec descriptor on success.
    let descriptor = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if descriptor < 0 {
        return Err(format!(
            "duplicate {label} descriptor: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: ownership of the newly duplicated descriptor transfers here.
    let duplicate = unsafe { File::from_raw_fd(descriptor) };
    require_cloexec(&duplicate, label)?;
    Ok(duplicate)
}
