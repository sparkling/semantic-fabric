//! Held identity for Cargo's top-level binary and its hashed linker hard link.

use std::fs::{self, File, Metadata, OpenOptions};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::authority_guard;

const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Cargo currently exposes one inode at two final paths: the JSON-selected
/// top-level artifact and the GNU linker depfile target. This narrowly-scoped
/// guard accepts exactly that pair; generic authority reads still reject every
/// hard link.
#[derive(Debug)]
pub(super) struct ArtifactPair {
    selected_path: PathBuf,
    linker_path: PathBuf,
    selected: File,
    linker: File,
    identity: LeafIdentity,
    effective_uid: u32,
    sha256: String,
}

impl ArtifactPair {
    pub(super) fn bind(selected_path: &Path, linker_path: &Path) -> Result<Self, String> {
        #[cfg(not(unix))]
        return Err("artifact hard-link binding requires Unix file identities".to_owned());

        if selected_path == linker_path {
            return Err("artifact hard-link paths must be distinct".to_owned());
        }
        let selected_before = inspect_path(selected_path, "Cargo-selected artifact")?;
        let linker_before = inspect_path(linker_path, "linker-output artifact")?;
        require_pair(&selected_before, &linker_before)?;
        let selected = open(selected_path, "Cargo-selected artifact")?;
        let linker = open(linker_path, "linker-output artifact")?;
        let selected_opened = inspect_opened(&selected, "Cargo-selected artifact")?;
        let linker_opened = inspect_opened(&linker, "linker-output artifact")?;
        if selected_before != selected_opened || linker_before != linker_opened {
            return Err("artifact hard-link identity changed while opening".to_owned());
        }
        require_pair(&selected_opened, &linker_opened)?;
        let sha256 = digest(&selected, selected_opened.size)?;
        let pair = Self {
            selected_path: selected_path.to_path_buf(),
            linker_path: linker_path.to_path_buf(),
            selected,
            linker,
            identity: selected_opened,
            effective_uid: effective_uid(),
            sha256,
        };
        pair.assert_current()?;
        Ok(pair)
    }

    pub(super) fn assert_current(&self) -> Result<(), String> {
        if effective_uid() != self.effective_uid {
            return Err("effective UID changed during artifact observation".to_owned());
        }
        let selected_path = inspect_path(&self.selected_path, "Cargo-selected artifact")?;
        let linker_path = inspect_path(&self.linker_path, "linker-output artifact")?;
        let selected_opened = inspect_opened(&self.selected, "Cargo-selected artifact")?;
        let linker_opened = inspect_opened(&self.linker, "linker-output artifact")?;
        for current in [
            &selected_path,
            &linker_path,
            &selected_opened,
            &linker_opened,
        ] {
            if current != &self.identity {
                return Err("artifact hard-link identity changed".to_owned());
            }
        }
        require_pair(&selected_path, &linker_path)?;
        if digest(&self.selected, self.identity.size)? != self.sha256
            || digest(&self.linker, self.identity.size)? != self.sha256
        {
            return Err("artifact hard-link bytes changed".to_owned());
        }
        Ok(())
    }

    pub(super) fn selected_path(&self) -> &Path {
        &self.selected_path
    }

    pub(super) fn sha256(&self) -> &str {
        &self.sha256
    }

    pub(super) fn byte_length(&self) -> u64 {
        self.identity.size
    }

    pub(super) fn unix_mode(&self) -> u32 {
        self.identity.mode
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LeafIdentity {
    device: u64,
    inode: u64,
    links: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    size: u64,
}

fn inspect_path(path: &Path, label: &str) -> Result<LeafIdentity, String> {
    authority_guard::validate_parent_ancestry(path, label)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect {label} {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{label} is not a regular non-symlink file"));
    }
    leaf(&metadata, label)
}

fn inspect_opened(file: &File, label: &str) -> Result<LeafIdentity, String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect opened {label}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("opened {label} is not a regular file"));
    }
    leaf(&metadata, label)
}

fn leaf(metadata: &Metadata, label: &str) -> Result<LeafIdentity, String> {
    authority_guard::validate_leaf(metadata, label)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let identity = LeafIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            links: metadata.nlink(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            mode: metadata.mode() & 0o7777,
            size: metadata.len(),
        };
        if identity.links != 2 {
            return Err(format!("{label} must have exactly two hard links"));
        }
        if identity.mode != 0o755 {
            return Err(format!("{label} mode must be exactly 0755"));
        }
        if identity.size == 0 || identity.size > MAX_ARTIFACT_BYTES {
            return Err(format!("{label} size is outside bounds"));
        }
        Ok(identity)
    }
    #[cfg(not(unix))]
    {
        let _ = (metadata, label);
        Err("artifact hard-link identity requires Unix metadata".to_owned())
    }
}

fn require_pair(left: &LeafIdentity, right: &LeafIdentity) -> Result<(), String> {
    if left != right {
        Err("Cargo-selected and linker-output artifacts are not one exact inode".to_owned())
    } else {
        Ok(())
    }
}

fn open(path: &Path, label: &str) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    options
        .open(path)
        .map_err(|error| format!("open {label} {}: {error}", path.display()))
}

#[cfg(unix)]
fn digest(file: &File, size: u64) -> Result<String, String> {
    use std::os::unix::fs::FileExt;
    let mut digest = Sha256::new();
    let mut offset = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    while offset < size {
        let remaining = usize::try_from((size - offset).min(buffer.len() as u64))
            .map_err(|_| "artifact digest size conversion failed".to_owned())?;
        let read = file
            .read_at(&mut buffer[..remaining], offset)
            .map_err(|error| format!("read held artifact: {error}"))?;
        if read == 0 {
            return Err("held artifact became shorter during digest".to_owned());
        }
        digest.update(&buffer[..read]);
        offset = offset
            .checked_add(read as u64)
            .ok_or_else(|| "artifact digest offset overflow".to_owned())?;
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(not(unix))]
fn digest(_file: &File, _size: u64) -> Result<String, String> {
    Err("artifact digest requires Unix positional reads".to_owned())
}

fn effective_uid() -> u32 {
    #[cfg(unix)]
    {
        // SAFETY: `geteuid` has no preconditions and dereferences no pointers.
        unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "semantic-fabric-artifact-pair-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        root
    }

    fn binary(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn binds_exactly_two_paths_and_detects_mutation_or_replacement() {
        let root = fixture("stable");
        let selected = root.join("selected");
        let linked = root.join("linked");
        binary(&selected, b"artifact");
        fs::hard_link(&selected, &linked).unwrap();
        let pair = ArtifactPair::bind(&selected, &linked).unwrap();
        assert_eq!(pair.byte_length(), 8);
        binary(&selected, b"changed!");
        assert!(pair.assert_current().is_err());
        drop(pair);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_different_inodes_third_links_and_symlinks() {
        let root = fixture("reject");
        let selected = root.join("selected");
        let linked = root.join("linked");
        binary(&selected, b"same");
        binary(&linked, b"same");
        assert!(ArtifactPair::bind(&selected, &linked).is_err());
        fs::remove_file(&linked).unwrap();
        fs::hard_link(&selected, &linked).unwrap();
        let third = root.join("third");
        fs::hard_link(&selected, &third).unwrap();
        assert!(ArtifactPair::bind(&selected, &linked).is_err());
        fs::remove_file(&third).unwrap();
        fs::remove_file(&linked).unwrap();
        symlink(&selected, &linked).unwrap();
        assert!(ArtifactPair::bind(&selected, &linked).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_persistent_replacement_of_both_paths() {
        let root = fixture("replacement");
        let selected = root.join("selected");
        let linked = root.join("linked");
        binary(&selected, b"artifact");
        fs::hard_link(&selected, &linked).unwrap();
        let pair = ArtifactPair::bind(&selected, &linked).unwrap();
        fs::remove_file(&selected).unwrap();
        fs::remove_file(&linked).unwrap();
        binary(&selected, b"artifact");
        fs::hard_link(&selected, &linked).unwrap();
        assert!(pair.assert_current().is_err());
        drop(pair);
        fs::remove_dir_all(root).unwrap();
    }
}
