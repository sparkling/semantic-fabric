//! Fresh workspace and one typed sandbox-logical to host-path authority.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::{authority, model::LinkInputOrigin, sandbox};

#[derive(Debug)]
pub(super) struct Workspace {
    pub(super) source: PathBuf,
    pub(super) target: PathBuf,
    pub(super) temporary: PathBuf,
}

impl Workspace {
    pub(super) fn prepare(
        repository: &Path,
        scratch_root: &Path,
        toolchain_root: &Path,
        cargo_home: &Path,
    ) -> Result<Self, String> {
        let repository = canonical_directory(repository, "repository")?;
        let scratch = canonical_directory(scratch_root, "capture scratch root")?;
        let toolchain = canonical_directory(toolchain_root, "toolchain root")?;
        let cargo_home = canonical_directory(cargo_home, "controlled Cargo home")?;
        require_private(&scratch, "capture scratch root")?;
        require_empty(&scratch, "capture scratch root")?;
        for authority in [&repository, &toolchain, &cargo_home] {
            if overlaps(&scratch, authority) {
                return Err("capture scratch root overlaps a bound authority".to_owned());
            }
        }
        let source = create_private(&scratch.join("source"))?;
        let target = create_private(&scratch.join("target"))?;
        let temporary = create_private(&scratch.join("temporary"))?;
        Ok(Self {
            source,
            target,
            temporary,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Workspace,
    CargoRegistry,
    RustSysroot,
    BuildOutput,
    HostSystem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Root {
    logical: PathBuf,
    backing: PathBuf,
    kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MappedPath {
    pub(super) backing: PathBuf,
    pub(super) origin: LinkInputOrigin,
    pub(super) receipt_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SandboxPathMap {
    roots: Vec<Root>,
}

impl SandboxPathMap {
    pub(super) fn new(
        source: &Path,
        cargo_registry: &Path,
        toolchain: &Path,
        target: &Path,
        system_mounts: &[sandbox::Mount],
    ) -> Result<Self, String> {
        let mut roots = vec![
            root("/workspace", source, Kind::Workspace)?,
            root("/cargo-home/registry", cargo_registry, Kind::CargoRegistry)?,
            root("/toolchain", toolchain, Kind::RustSysroot)?,
            root("/target", target, Kind::BuildOutput)?,
        ];
        for mount in system_mounts {
            roots.push(root(mount.destination, &mount.source, Kind::HostSystem)?);
        }
        let logical: BTreeSet<_> = roots.iter().map(|root| &root.logical).collect();
        if logical.len() != roots.len() {
            return Err("sandbox path map contains a duplicate logical root".to_owned());
        }
        roots.sort_by(|left, right| left.logical.cmp(&right.logical));
        Ok(Self { roots })
    }

    pub(super) fn map(&self, logical: &Path) -> Result<MappedPath, String> {
        validate_absolute_normal(logical, "sandbox-logical path")?;
        let root = self.longest_root(logical)?;
        let relative = logical
            .strip_prefix(&root.logical)
            .map_err(|_| "sandbox path mapping changed".to_owned())?;
        if relative.as_os_str().is_empty() {
            return Err("sandbox-logical path cannot equal its root".to_owned());
        }
        let backing = root.backing.join(relative);
        authority::validate_beneath(&root.backing, &backing, "sandbox path mapping")?;
        let (origin, receipt_path) = match root.kind {
            Kind::Workspace => (LinkInputOrigin::Workspace, prefixed("workspace", relative)?),
            Kind::CargoRegistry => (
                LinkInputOrigin::CargoRegistry,
                prefixed("cargo-registry", relative)?,
            ),
            Kind::RustSysroot => (
                LinkInputOrigin::RustSysroot,
                prefixed("rust-sysroot", relative)?,
            ),
            Kind::BuildOutput => (
                LinkInputOrigin::BuildOutput,
                prefixed("build-output", relative)?,
            ),
            Kind::HostSystem => {
                let absolute_relative = logical
                    .strip_prefix("/")
                    .map_err(|_| "host-system path lost its root".to_owned())?;
                (
                    LinkInputOrigin::HostSystem,
                    prefixed("host-system", absolute_relative)?,
                )
            }
        };
        Ok(MappedPath {
            backing,
            origin,
            receipt_path,
        })
    }

    pub(super) fn map_target(&self, logical: &Path) -> Result<PathBuf, String> {
        let mapped = self.map(logical)?;
        if mapped.origin != LinkInputOrigin::BuildOutput {
            return Err("Cargo-selected path is not inside logical /target".to_owned());
        }
        Ok(mapped.backing)
    }

    fn longest_root(&self, logical: &Path) -> Result<&Root, String> {
        let mut matches: Vec<_> = self
            .roots
            .iter()
            .filter(|root| logical.starts_with(&root.logical))
            .collect();
        matches.sort_by_key(|root| std::cmp::Reverse(root.logical.components().count()));
        let Some(best) = matches.first() else {
            return Err(format!(
                "sandbox-logical path is unmapped: {}",
                logical.display()
            ));
        };
        if matches.get(1).is_some_and(|candidate| {
            candidate.logical.components().count() == best.logical.components().count()
        }) {
            return Err(format!(
                "sandbox-logical path mapping is ambiguous: {}",
                logical.display()
            ));
        }
        Ok(best)
    }
}

fn root(logical: &str, backing: &Path, kind: Kind) -> Result<Root, String> {
    let logical = PathBuf::from(logical);
    validate_absolute_normal(&logical, "sandbox logical root")?;
    let backing = canonical_directory(backing, "sandbox backing root")?;
    Ok(Root {
        logical,
        backing,
        kind,
    })
}

fn prefixed(prefix: &str, relative: &Path) -> Result<String, String> {
    let path = Path::new(prefix).join(relative);
    let value = path
        .to_str()
        .ok_or_else(|| "receipt logical path is not UTF-8".to_owned())?
        .to_owned();
    super::model::validate_logical_path(&value)?;
    Ok(value)
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    validate_absolute_normal(path, label)?;
    authority::validate_directory(path, label)?;
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))?;
    if canonical != path {
        return Err(format!("{label} is not canonical"));
    }
    Ok(canonical)
}

fn validate_absolute_normal(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        Err(format!("{label} must be absolute and normalized"))
    } else {
        Ok(())
    }
}

fn require_empty(path: &Path, label: &str) -> Result<(), String> {
    if fs::read_dir(path)
        .map_err(|error| format!("read {label}: {error}"))?
        .next()
        .transpose()
        .map_err(|error| format!("enumerate {label}: {error}"))?
        .is_some()
    {
        Err(format!("{label} must be empty"))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn require_private(path: &Path, label: &str) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let mode = fs::metadata(path)
        .map_err(|error| format!("inspect {label}: {error}"))?
        .mode()
        & 0o777;
    if mode != 0o700 {
        Err(format!("{label} mode must be exactly 0700"))
    } else {
        Ok(())
    }
}

#[cfg(not(unix))]
fn require_private(_path: &Path, _label: &str) -> Result<(), String> {
    Err("artifact observation capture requires Unix permissions".to_owned())
}

fn create_private(path: &Path) -> Result<PathBuf, String> {
    fs::create_dir(path).map_err(|error| format!("create {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("set private mode on {}: {error}", path.display()))?;
    }
    Ok(path.to_path_buf())
}

fn overlaps(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn maps_all_origin_classes_without_host_path_leakage() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = std::env::temp_dir().join(format!(
            "semantic-fabric-sandbox-map-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&fixture);
        for name in ["source", "registry", "toolchain", "target", "system"] {
            let path = fixture.join(name);
            fs::create_dir_all(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mounts = [sandbox::Mount {
            source: fixture.join("system"),
            destination: "/usr/lib",
        }];
        let map = SandboxPathMap::new(
            &fixture.join("source"),
            &fixture.join("registry"),
            &fixture.join("toolchain"),
            &fixture.join("target"),
            &mounts,
        )
        .unwrap();
        fs::write(fixture.join("source/Cargo.lock"), b"lock").unwrap();
        fs::write(fixture.join("system/libc.so"), b"libc").unwrap();
        for path in [
            fixture.join("source/Cargo.lock"),
            fixture.join("system/libc.so"),
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let workspace = map.map(Path::new("/workspace/Cargo.lock")).unwrap();
        assert_eq!(workspace.receipt_path, "workspace/Cargo.lock");
        let system = map.map(Path::new("/usr/lib/libc.so")).unwrap();
        assert_eq!(system.receipt_path, "host-system/usr/lib/libc.so");
        assert!(map.map(Path::new("/unmapped/x")).is_err());
        fs::remove_dir_all(fixture).unwrap();
    }
}
