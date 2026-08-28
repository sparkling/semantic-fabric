use std::fmt;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

pub const SCENARIOS_PATH: &str = "crates/sf-bench/config/performance-scenarios-v1.tsv";
pub const PROFILE_PATH: &str = "crates/sf-bench/config/performance-runner-profile-v1.tsv";
pub const BASELINE_PATH: &str = "crates/sf-bench/config/performance-baseline-v1.tsv";
pub const CANDIDATE_PATH: &str = "target/sf-performance/candidate-v1.tsv";
pub const WORK_PATH: &str = "target/sf-performance/work";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathError(pub String);

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PathError {}

#[derive(Debug, Clone)]
pub struct RepositoryLayout {
    root: PathBuf,
}

impl RepositoryLayout {
    pub fn discover() -> Result<Self, PathError> {
        let root = std::env::current_dir()
            .map_err(|error| PathError(format!("read current directory: {error}")))?;
        Self::new(root)
    }

    pub fn new(root: PathBuf) -> Result<Self, PathError> {
        if !root.is_absolute() {
            return Err(PathError("repository root must be absolute".into()));
        }
        reject_symlink(&root)?;
        let canonical = root
            .canonicalize()
            .map_err(|error| PathError(format!("resolve {}: {error}", root.display())))?;
        if canonical != root {
            return Err(PathError(
                "repository root must already be canonical".into(),
            ));
        }
        let git = root.join(".git");
        let git_metadata = std::fs::symlink_metadata(&git)
            .map_err(|error| PathError(format!("inspect {}: {error}", git.display())))?;
        if git_metadata.file_type().is_symlink()
            || (!git_metadata.is_dir() && !git_metadata.is_file())
        {
            return Err(PathError(format!(
                "{} is not a repository root (.git is invalid)",
                root.display()
            )));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn fixed_path(&self, relative: &str) -> Result<PathBuf, PathError> {
        validate_relative(relative)?;
        let path = self.root.join(relative);
        self.reject_symlink_chain(&path)?;
        Ok(path)
    }

    pub fn read_fixed(&self, relative: &str, maximum: usize) -> Result<Vec<u8>, PathError> {
        let path = self.fixed_path(relative)?;
        let path_metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| PathError(format!("inspect {}: {error}", path.display())))?;
        validate_file_authority(&path, &path_metadata)?;
        if path_metadata.len() > maximum as u64 {
            return Err(PathError(format!(
                "{} exceeds the {maximum} byte bound",
                path.display()
            )));
        }
        let mut file = File::open(&path)
            .map_err(|error| PathError(format!("open {}: {error}", path.display())))?;
        let opened = file
            .metadata()
            .map_err(|error| PathError(format!("inspect open {}: {error}", path.display())))?;
        validate_file_authority(&path, &opened)?;
        if !same_file(&path_metadata, &opened) {
            return Err(PathError(format!(
                "{} changed while opening",
                path.display()
            )));
        }
        validate_path_identity(&path, &opened)?;
        let mut bytes = Vec::with_capacity(path_metadata.len() as usize);
        Read::by_ref(&mut file)
            .take(maximum as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| PathError(format!("read {}: {error}", path.display())))?;
        if bytes.len() > maximum {
            return Err(PathError(format!(
                "{} grew beyond the byte bound while reading",
                path.display()
            )));
        }
        validate_path_identity(&path, &opened)?;
        Ok(bytes)
    }

    pub fn write_new_fixed(&self, relative: &str, bytes: &[u8]) -> Result<PathBuf, PathError> {
        validate_relative(relative)?;
        let path = self.root.join(relative);
        let parent = path
            .parent()
            .ok_or_else(|| PathError("fixed output has no parent".into()))?;
        self.create_contained_directories(parent)?;
        self.reject_symlink_chain(&path)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                PathError(format!(
                    "create new {} (existing outputs are never overwritten): {error}",
                    path.display()
                ))
            })?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| PathError(format!("write {}: {error}", path.display())))?;
        Ok(path)
    }

    pub fn require_fixed_absent(&self, relative: &str) -> Result<(), PathError> {
        let path = self.fixed_path(relative)?;
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err(PathError(format!(
                "{} already exists; capture outputs are never overwritten",
                path.display()
            ))),
            Err(error) => Err(PathError(format!("inspect {}: {error}", path.display()))),
        }
    }

    pub fn create_run_directory(&self, run_token: &str) -> Result<PathBuf, PathError> {
        if run_token.is_empty()
            || run_token.len() > 96
            || !run_token
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(PathError("invalid run token".into()));
        }
        let work = self.root.join(WORK_PATH);
        self.create_contained_directories(&work)?;
        let run = work.join(run_token);
        self.reject_symlink_chain(&run)?;
        std::fs::create_dir(&run).map_err(|error| {
            PathError(format!(
                "create fresh run directory {}: {error}",
                run.display()
            ))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| PathError(format!("secure {}: {error}", run.display())))?;
        }
        Ok(run)
    }

    pub fn remove_run_directory(&self, run: &Path) -> Result<(), PathError> {
        let work = self.root.join(WORK_PATH);
        if run.parent() != Some(work.as_path()) {
            return Err(PathError(
                "run directory is outside the fixed work path".into(),
            ));
        }
        self.reject_symlink_chain(run)?;
        std::fs::remove_dir(run)
            .map_err(|error| PathError(format!("remove {}: {error}", run.display())))
    }

    pub fn validate_contained_file(&self, path: &Path) -> Result<(), PathError> {
        if !path.is_absolute() || !path.starts_with(&self.root) {
            return Err(PathError("artifact must be inside the repository".into()));
        }
        self.reject_symlink_chain(path)?;
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| PathError(format!("inspect {}: {error}", path.display())))?;
        validate_file_authority(path, &metadata)
    }

    fn create_contained_directories(&self, path: &Path) -> Result<(), PathError> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| PathError("output parent escapes repository".into()))?;
        let mut current = self.root.clone();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(PathError("invalid output path component".into()));
            };
            current.push(name);
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(PathError(format!(
                            "{} is not a real directory",
                            current.display()
                        )));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::create_dir(&current).map_err(|create_error| {
                        PathError(format!("create {}: {create_error}", current.display()))
                    })?;
                }
                Err(error) => {
                    return Err(PathError(format!("inspect {}: {error}", current.display())))
                }
            }
        }
        Ok(())
    }

    fn reject_symlink_chain(&self, path: &Path) -> Result<(), PathError> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| PathError("path escapes repository".into()))?;
        let mut current = self.root.clone();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(PathError("invalid path component".into()));
            };
            current.push(name);
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(PathError(format!(
                        "symlink targets are forbidden: {}",
                        current.display()
                    )))
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => {
                    return Err(PathError(format!("inspect {}: {error}", current.display())))
                }
            }
        }
        Ok(())
    }
}

fn validate_relative(relative: &str) -> Result<(), PathError> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(PathError("fixed path is not a safe relative path".into()));
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), PathError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| PathError(format!("inspect {}: {error}", path.display())))?;
    if metadata.file_type().is_symlink() {
        return Err(PathError(format!(
            "symlink targets are forbidden: {}",
            path.display()
        )));
    }
    Ok(())
}

fn validate_file_authority(path: &Path, metadata: &Metadata) -> Result<(), PathError> {
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PathError(format!(
            "{} is not a regular non-symlink file",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(PathError(format!("{} is a hard link", path.display())));
        }
    }
    Ok(())
}

fn validate_path_identity(path: &Path, opened: &Metadata) -> Result<(), PathError> {
    let current = std::fs::symlink_metadata(path)
        .map_err(|error| PathError(format!("reinspect {}: {error}", path.display())))?;
    validate_file_authority(path, &current)?;
    if !same_file(&current, opened) {
        return Err(PathError(format!("{} changed during read", path.display())));
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
