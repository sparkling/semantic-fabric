use std::ffi::CStr;
use std::fs::File;
use std::path::{Component, Path};

use rustix::fs::{
    fchmod, fstat, mkdirat, open, openat, statat, unlinkat, AtFlags, FileType, Mode, OFlags, Stat,
};
use rustix::io::Errno;

use super::SpillError;

pub(super) const MAX_BLOCKS: usize = 64;
const MAX_ROOT_COMPONENTS: usize = 64;
const NAME_LEN: usize = 37;
const NAME_ATTEMPTS: usize = 8;
const RUN_PREFIX: &[u8; 4] = b"run-";
const BLOCK_PREFIX: &[u8; 4] = b"blk-";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    fn from_stat(stat: &Stat) -> Self {
        Self {
            device: stat.st_dev,
            inode: stat.st_ino,
        }
    }

    fn matches(self, stat: &Stat) -> bool {
        self == Self::from_stat(stat)
    }
}

#[derive(Clone, Copy)]
struct EntryName([u8; NAME_LEN]);

impl EntryName {
    fn random(prefix: &[u8; 4]) -> Result<Self, SpillError> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| SpillError::EntropyUnavailable)?;
        let mut bytes = [0_u8; NAME_LEN];
        bytes[..prefix.len()].copy_from_slice(prefix);
        for (index, byte) in random.into_iter().enumerate() {
            bytes[prefix.len() + index * 2] = hex(byte >> 4);
            bytes[prefix.len() + index * 2 + 1] = hex(byte & 0x0f);
        }
        Ok(Self(bytes))
    }

    fn as_c_str(&self) -> &CStr {
        CStr::from_bytes_with_nul(&self.0).expect("fixed ASCII name with trailing NUL")
    }

    #[cfg(test)]
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0[..NAME_LEN - 1]).expect("fixed ASCII name")
    }
}

fn hex(nibble: u8) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble,
        _ => b'a' + nibble - 10,
    }
}

#[derive(Clone, Copy)]
struct BlockRecord {
    name: EntryName,
    identity: FileIdentity,
}

#[derive(Clone, Copy)]
enum EntryKind {
    File,
    Directory,
}

/// Arms immediately after an exclusive create. Before `fstat` establishes an
/// inode it removes only the just-created random name; afterwards it also
/// requires identity and kind to match. The same-UID race nonclaim documented
/// on this module's cleanup still applies to the final check/unlink pair.
struct PendingEntry<'a> {
    parent: &'a File,
    name: EntryName,
    kind: EntryKind,
    identity: Option<FileIdentity>,
    armed: bool,
}

impl<'a> PendingEntry<'a> {
    fn new(parent: &'a File, name: EntryName, kind: EntryKind) -> Self {
        Self {
            parent,
            name,
            kind,
            identity: None,
            armed: true,
        }
    }

    fn bind(&mut self, identity: FileIdentity) {
        self.identity = Some(identity);
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for PendingEntry<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let flags = match self.kind {
            EntryKind::File => AtFlags::empty(),
            EntryKind::Directory => AtFlags::REMOVEDIR,
        };
        let matches = match self.identity {
            None => true,
            Some(identity) => statat(self.parent, self.name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)
                .is_ok_and(|stat| {
                    identity.matches(&stat)
                        && match self.kind {
                            EntryKind::File => valid_regular_file(&stat),
                            EntryKind::Directory => is_directory(&stat),
                        }
                }),
        };
        if matches {
            let _ = unlinkat(self.parent, self.name.as_c_str(), flags);
        }
    }
}

pub(super) struct OpenedBlock {
    pub(super) file: File,
    pub(super) size: u64,
}

pub(super) struct OwnedArtifacts {
    // FD accounting invariant: these are the two descriptors retained for the
    // run lifetime. Block create/open returns one transient `File` at a time.
    root: File,
    run: File,
    run_name: EntryName,
    run_identity: FileIdentity,
    blocks: [Option<BlockRecord>; MAX_BLOCKS],
}

impl OwnedArtifacts {
    pub(super) fn create(root_path: &Path) -> Result<Self, SpillError> {
        Self::create_with_hook(root_path, || Ok(()))
    }

    fn create_with_hook(
        root_path: &Path,
        after_mkdir: impl FnOnce() -> Result<(), SpillError>,
    ) -> Result<Self, SpillError> {
        let root = open_root(root_path)?;
        let mut after_mkdir = Some(after_mkdir);
        for _ in 0..NAME_ATTEMPTS {
            let run_name = EntryName::random(RUN_PREFIX)?;
            match mkdirat(&root, run_name.as_c_str(), Mode::RWXU) {
                Ok(()) => {}
                Err(error) if error == Errno::EXIST => continue,
                Err(_) => return Err(SpillError::RunCreateFailed),
            }
            let mut pending = PendingEntry::new(&root, run_name, EntryKind::Directory);
            let hook = after_mkdir.take().ok_or(SpillError::RunCreateFailed)?;
            hook()?;

            let run_fd = match openat(
                &root,
                run_name.as_c_str(),
                directory_open_flags(),
                Mode::empty(),
            ) {
                Ok(fd) => fd,
                Err(_) => return Err(SpillError::RunCreateFailed),
            };
            let run = File::from(run_fd);
            let stat = fstat(&run).map_err(|_| SpillError::RunCreateFailed)?;
            let run_identity = FileIdentity::from_stat(&stat);
            pending.bind(run_identity);
            if !is_directory(&stat) || fchmod(&run, Mode::RWXU).is_err() {
                return Err(SpillError::RunCreateFailed);
            }
            let stat = fstat(&run).map_err(|_| SpillError::RunCreateFailed)?;
            if !run_identity.matches(&stat) || permission_bits(&stat) != 0o700 {
                return Err(SpillError::RunCreateFailed);
            }
            pending.disarm();
            return Ok(Self {
                root,
                run,
                run_name,
                run_identity,
                blocks: [None; MAX_BLOCKS],
            });
        }
        Err(SpillError::RunCreateFailed)
    }

    pub(super) fn create_block(&mut self, index: usize) -> Result<File, SpillError> {
        self.create_block_with_hook(index, || Ok(()))
    }

    fn create_block_with_hook(
        &mut self,
        index: usize,
        after_create: impl FnOnce() -> Result<(), SpillError>,
    ) -> Result<File, SpillError> {
        if index >= MAX_BLOCKS || self.blocks[index].is_some() {
            return Err(SpillError::QuotaExceeded);
        }
        let mut after_create = Some(after_create);
        for _ in 0..NAME_ATTEMPTS {
            let name = EntryName::random(BLOCK_PREFIX)?;
            let fd = match openat(
                &self.run,
                name.as_c_str(),
                OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            ) {
                Ok(fd) => fd,
                Err(error) if error == Errno::EXIST => continue,
                Err(_) => return Err(SpillError::FileCreateFailed),
            };
            let mut pending = PendingEntry::new(&self.run, name, EntryKind::File);
            let hook = after_create.take().ok_or(SpillError::FileCreateFailed)?;
            hook()?;
            let file = File::from(fd);
            let stat = fstat(&file).map_err(|_| SpillError::FileCreateFailed)?;
            let record = BlockRecord {
                name,
                identity: FileIdentity::from_stat(&stat),
            };
            pending.bind(record.identity);
            if !valid_regular_file(&stat) || fchmod(&file, Mode::RUSR | Mode::WUSR).is_err() {
                return Err(SpillError::FileCreateFailed);
            }
            let stat = fstat(&file).map_err(|_| SpillError::FileCreateFailed)?;
            if !record.identity.matches(&stat)
                || !valid_regular_file(&stat)
                || permission_bits(&stat) != 0o600
            {
                return Err(SpillError::FileCreateFailed);
            }
            pending.disarm();
            self.blocks[index] = Some(record);
            return Ok(file);
        }
        Err(SpillError::FileCreateFailed)
    }

    pub(super) fn open_block(&self, index: usize) -> Result<OpenedBlock, SpillError> {
        let record = self
            .blocks
            .get(index)
            .and_then(Option::as_ref)
            .ok_or(SpillError::OwnershipMismatch)?;
        let before = statat(&self.run, record.name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| SpillError::OwnershipMismatch)?;
        if !record.identity.matches(&before) || !valid_regular_file(&before) {
            return Err(SpillError::OwnershipMismatch);
        }
        let fd = openat(
            &self.run,
            record.name.as_c_str(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| SpillError::OwnershipMismatch)?;
        let file = File::from(fd);
        let after = fstat(&file).map_err(|_| SpillError::OwnershipMismatch)?;
        if !record.identity.matches(&after) || !valid_regular_file(&after) {
            return Err(SpillError::OwnershipMismatch);
        }
        let size = u64::try_from(after.st_size).map_err(|_| SpillError::LengthMismatch)?;
        Ok(OpenedBlock { file, size })
    }

    /// Best-effort, descriptor-relative cleanup. A stable name is removed only
    /// when no-follow metadata identifies the exact inode we created.
    ///
    /// Linux has no unprivileged compare-and-unlink-by-inode operation. A
    /// hostile same-UID actor racing a rename between `statat` and `unlinkat`
    /// is therefore outside this private prototype's claim. Production would
    /// additionally require process/mount-namespace isolation or equivalent.
    pub(super) fn cleanup(&mut self) -> bool {
        let mut complete = true;
        for slot in &mut self.blocks {
            let Some(record) = *slot else {
                continue;
            };
            match statat(&self.run, record.name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
                Err(error) if error == Errno::NOENT => *slot = None,
                Ok(stat) if record.identity.matches(&stat) && valid_regular_file(&stat) => {
                    if unlinkat(&self.run, record.name.as_c_str(), AtFlags::empty()).is_ok() {
                        *slot = None;
                    } else {
                        complete = false;
                    }
                }
                _ => complete = false,
            }
        }

        match statat(
            &self.root,
            self.run_name.as_c_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Err(error) if error == Errno::NOENT => {}
            Ok(stat) if self.run_identity.matches(&stat) && is_directory(&stat) => {
                if unlinkat(&self.root, self.run_name.as_c_str(), AtFlags::REMOVEDIR).is_err() {
                    complete = false;
                }
            }
            _ => complete = false,
        }
        complete
    }

    #[cfg(test)]
    pub(super) fn run_name(&self) -> &str {
        self.run_name.as_str()
    }

    #[cfg(test)]
    pub(super) fn block_name(&self, index: usize) -> &str {
        self.blocks[index]
            .as_ref()
            .expect("written block")
            .name
            .as_str()
    }
}

pub(super) fn validate_root_path(path: &Path) -> Result<(), SpillError> {
    if !path.is_absolute() || path == Path::new("/") {
        return Err(SpillError::InvalidConfiguration);
    }
    let mut normal_components = 0_usize;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(_) => {
                normal_components = normal_components
                    .checked_add(1)
                    .ok_or(SpillError::InvalidConfiguration)?;
                if normal_components > MAX_ROOT_COMPONENTS {
                    return Err(SpillError::InvalidConfiguration);
                }
            }
            _ => return Err(SpillError::InvalidConfiguration),
        }
    }
    if normal_components == 0 {
        return Err(SpillError::InvalidConfiguration);
    }
    Ok(())
}

fn open_root(path: &Path) -> Result<File, SpillError> {
    // Defense in depth: `OwnedArtifacts` validates independently rather than
    // trusting its caller to have enforced the bounded component walk.
    validate_root_path(path)?;
    let root_fd = open("/", directory_open_flags(), Mode::empty())
        .map_err(|_| SpillError::RootUnavailable)?;
    let mut current = File::from(root_fd);
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                let next = openat(&current, name, directory_open_flags(), Mode::empty())
                    .map_err(|_| SpillError::RootUnavailable)?;
                current = File::from(next);
            }
            _ => return Err(SpillError::InvalidConfiguration),
        }
    }
    let stat = fstat(&current).map_err(|_| SpillError::RootUnavailable)?;
    if !is_directory(&stat) {
        return Err(SpillError::RootUnavailable);
    }
    Ok(current)
}

fn directory_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

fn permission_bits(stat: &Stat) -> u32 {
    stat.st_mode & 0o777
}

fn is_directory(stat: &Stat) -> bool {
    FileType::from_raw_mode(stat.st_mode).is_dir()
}

fn valid_regular_file(stat: &Stat) -> bool {
    FileType::from_raw_mode(stat.st_mode).is_file() && stat.st_nlink == 1
}

#[cfg(test)]
mod tests {
    use super::super::tests::Fixture;
    use super::*;

    #[test]
    fn pending_run_guard_cleans_failure_before_first_fstat() {
        let fixture = Fixture::new();
        assert!(matches!(
            OwnedArtifacts::create_with_hook(fixture.root(), || Err(SpillError::RunCreateFailed)),
            Err(SpillError::RunCreateFailed)
        ));
        assert_eq!(fixture.entries(), 0);
    }

    #[test]
    fn pending_file_guard_cleans_failure_before_first_fstat() {
        let fixture = Fixture::new();
        let mut artifacts = OwnedArtifacts::create(fixture.root()).unwrap();
        assert!(matches!(
            artifacts.create_block_with_hook(0, || Err(SpillError::FileCreateFailed)),
            Err(SpillError::FileCreateFailed)
        ));
        let run = fixture.root().join(artifacts.run_name());
        assert_eq!(std::fs::read_dir(run).unwrap().count(), 0);
        assert!(artifacts.cleanup());
        assert_eq!(fixture.entries(), 0);
    }

    #[test]
    fn owned_artifact_entry_rejects_an_overlong_component_walk_itself() {
        let mut path = std::path::PathBuf::from("/");
        for _ in 0..=MAX_ROOT_COMPONENTS {
            path.push("bounded");
        }
        assert!(matches!(
            OwnedArtifacts::create(&path),
            Err(SpillError::InvalidConfiguration)
        ));
    }
}
