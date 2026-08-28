use std::fs;
use std::path::{Component, Path, PathBuf};

use super::{prefixed, validate_absolute_normal, Kind, MappedPath, SandboxPathMap};
use crate::binary_artifact_receipt::{authority_guard, model::LinkInputOrigin};

const MAX_LINK_TARGET_BYTES: usize = 16 * 1024;
const MAX_PATH_COMPONENTS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::binary_artifact_receipt) enum LinkMappedPath {
    Direct(MappedPath),
    Alias(HostAliasMapping),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::binary_artifact_receipt) struct HostAliasMapping {
    pub(in crate::binary_artifact_receipt) alias_backing: PathBuf,
    pub(in crate::binary_artifact_receipt) alias_receipt_path: String,
    pub(in crate::binary_artifact_receipt) alias_root: PathBuf,
    pub(in crate::binary_artifact_receipt) raw_target: Vec<u8>,
    pub(in crate::binary_artifact_receipt) terminal_logical: PathBuf,
    pub(in crate::binary_artifact_receipt) terminal: MappedPath,
    pub(in crate::binary_artifact_receipt) terminal_root: PathBuf,
}

impl SandboxPathMap {
    /// Maps one linker input. The only exception to the generic no-symlink
    /// policy is a final, relative, one-hop HostSystem alias whose terminal
    /// independently maps back into a declared HostSystem root.
    pub(in crate::binary_artifact_receipt) fn map_link_input(
        &self,
        logical: &Path,
    ) -> Result<LinkMappedPath, String> {
        validate_absolute_normal(logical, "link input sandbox-logical path")?;
        let root = self.longest_root(logical)?;
        let relative = logical
            .strip_prefix(&root.logical)
            .map_err(|_| "link input root mapping changed".to_owned())?;
        if relative.as_os_str().is_empty() {
            return Err("link input cannot equal its mapped root".to_owned());
        }
        let alias_backing = root.backing.join(relative);
        let metadata = fs::symlink_metadata(&alias_backing)
            .map_err(|error| format!("inspect link input {}: {error}", alias_backing.display()))?;
        if !metadata.file_type().is_symlink() {
            return self.map(logical).map(LinkMappedPath::Direct);
        }
        if root.kind != Kind::HostSystem {
            return Err("only HostSystem final link inputs may be aliases".to_owned());
        }
        authority_guard::validate_parent_ancestry(&alias_backing, "host link alias")?;
        let target = fs::read_link(&alias_backing).map_err(|error| {
            format!("read host link alias {}: {error}", alias_backing.display())
        })?;
        let raw_target = raw_path_bytes(&target)?;
        let terminal_logical = normalized_relative_target(logical, &target, &raw_target)?;
        let terminal_root = self.longest_root(&terminal_logical)?;
        if terminal_root.kind != Kind::HostSystem {
            return Err("host link alias resolves outside HostSystem authority".to_owned());
        }
        let terminal = self.map(&terminal_logical)?;
        if terminal.origin != LinkInputOrigin::HostSystem {
            return Err("host link alias terminal lost HostSystem authority".to_owned());
        }
        let alias_relative = logical
            .strip_prefix("/")
            .map_err(|_| "host link alias lost its sandbox root".to_owned())?;
        Ok(LinkMappedPath::Alias(HostAliasMapping {
            alias_backing,
            alias_receipt_path: prefixed("host-system-alias", alias_relative)?,
            alias_root: root.backing.clone(),
            raw_target,
            terminal_logical,
            terminal,
            terminal_root: terminal_root.backing.clone(),
        }))
    }
}

fn normalized_relative_target(
    alias: &Path,
    target: &Path,
    raw_target: &[u8],
) -> Result<PathBuf, String> {
    if target.is_absolute()
        || raw_target.is_empty()
        || raw_target.len() > MAX_LINK_TARGET_BYTES
        || raw_target.contains(&b'\\')
        || raw_target.iter().any(|byte| byte.is_ascii_control())
    {
        return Err("host link alias target is not a bounded relative path".to_owned());
    }
    let target_text = std::str::from_utf8(raw_target)
        .map_err(|_| "host link alias target is not UTF-8".to_owned())?;
    let mut parts = alias
        .parent()
        .ok_or_else(|| "host link alias has no logical parent".to_owned())?
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for component in target_text.split('/') {
        match component {
            "" | "." => {
                return Err("host link alias target is not lexically canonical".to_owned());
            }
            ".." => {
                parts
                    .pop()
                    .ok_or_else(|| "host link alias target escapes sandbox root".to_owned())?;
            }
            normal => {
                if parts.len() == MAX_PATH_COMPONENTS {
                    return Err("host link alias target has too many components".to_owned());
                }
                parts.push(normal.into());
            }
        }
    }
    if parts.is_empty() {
        return Err("host link alias target resolves to sandbox root".to_owned());
    }
    let mut normalized = PathBuf::from("/");
    normalized.extend(parts);
    validate_absolute_normal(&normalized, "host link alias terminal")?;
    Ok(normalized)
}

#[cfg(unix)]
fn raw_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    std::str::from_utf8(bytes).map_err(|_| "host link alias target is not UTF-8".to_owned())?;
    Ok(bytes.to_vec())
}

#[cfg(not(unix))]
fn raw_path_bytes(path: &Path) -> Result<Vec<u8>, String> {
    path.to_str()
        .map(str::as_bytes)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| "host link alias target is not UTF-8".to_owned())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::binary_artifact_receipt::sandbox;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn fixture(name: &str) -> (PathBuf, SandboxPathMap) {
        let root = std::env::temp_dir().join(format!(
            "semantic-fabric-link-map-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        for directory in [
            "source",
            "registry",
            "toolchain",
            "target",
            "usr-lib",
            "usr-lib64",
        ] {
            let path = root.join(directory);
            fs::create_dir_all(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mounts = [
            sandbox::Mount {
                source: root.join("usr-lib"),
                destination: "/usr/lib",
            },
            sandbox::Mount {
                source: root.join("usr-lib"),
                destination: "/lib",
            },
            sandbox::Mount {
                source: root.join("usr-lib64"),
                destination: "/usr/lib64",
            },
            sandbox::Mount {
                source: root.join("usr-lib64"),
                destination: "/lib64",
            },
        ];
        let map = SandboxPathMap::new(
            &root.join("source"),
            &root.join("registry"),
            &root.join("toolchain"),
            &root.join("target"),
            &mounts,
        )
        .unwrap();
        (root, map)
    }

    fn write_private(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn maps_same_root_and_cross_mount_host_aliases_without_host_paths() {
        let (root, map) = fixture("valid");
        write_private(&root.join("usr-lib/libssl.so.3"), b"ssl");
        symlink("libssl.so.3", root.join("usr-lib/libssl.so")).unwrap();
        write_private(&root.join("usr-lib/ld-linux.so.2"), b"loader");
        symlink("../lib/ld-linux.so.2", root.join("usr-lib64/ld-linux.so.2")).unwrap();

        let LinkMappedPath::Alias(ssl) =
            map.map_link_input(Path::new("/usr/lib/libssl.so")).unwrap()
        else {
            panic!("expected host alias")
        };
        assert_eq!(
            ssl.alias_receipt_path,
            "host-system-alias/usr/lib/libssl.so"
        );
        assert_eq!(
            ssl.terminal_receipt_path(),
            "host-system/usr/lib/libssl.so.3"
        );
        assert!(!ssl.terminal.receipt_path.contains(root.to_str().unwrap()));

        let LinkMappedPath::Alias(loader) = map
            .map_link_input(Path::new("/lib64/ld-linux.so.2"))
            .unwrap()
        else {
            panic!("expected cross-mount host alias")
        };
        assert_eq!(loader.terminal_logical, Path::new("/lib/ld-linux.so.2"));
        assert_eq!(
            loader.terminal.receipt_path,
            "host-system/usr/lib/ld-linux.so.2"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_cross_origin_absolute_multihop_and_intermediate_aliases() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let (root, map) = fixture("reject");
        write_private(&root.join("source/evil"), b"evil");
        symlink("../../workspace/evil", root.join("usr-lib/cross-origin.so")).unwrap();
        assert!(map
            .map_link_input(Path::new("/usr/lib/cross-origin.so"))
            .unwrap_err()
            .contains("outside HostSystem"));

        symlink("/usr/lib/terminal", root.join("usr-lib/absolute.so")).unwrap();
        assert!(map
            .map_link_input(Path::new("/usr/lib/absolute.so"))
            .is_err());

        symlink("../../../escape", root.join("usr-lib/escape.so")).unwrap();
        assert!(map
            .map_link_input(Path::new("/usr/lib/escape.so"))
            .unwrap_err()
            .contains("escapes sandbox root"));
        symlink("../../opt/missing", root.join("usr-lib/unmapped.so")).unwrap();
        assert!(map
            .map_link_input(Path::new("/usr/lib/unmapped.so"))
            .unwrap_err()
            .contains("unmapped"));
        symlink("missing.so", root.join("usr-lib/dangling.so")).unwrap();
        assert!(map
            .map_link_input(Path::new("/usr/lib/dangling.so"))
            .is_err());
        symlink(
            OsString::from_vec(vec![0xff]),
            root.join("usr-lib/non-utf8.so"),
        )
        .unwrap();
        assert!(map
            .map_link_input(Path::new("/usr/lib/non-utf8.so"))
            .unwrap_err()
            .contains("UTF-8"));

        write_private(&root.join("usr-lib/terminal"), b"terminal");
        symlink("terminal", root.join("usr-lib/second.so")).unwrap();
        symlink("second.so", root.join("usr-lib/first.so")).unwrap();
        assert!(map
            .map_link_input(Path::new("/usr/lib/first.so"))
            .unwrap_err()
            .contains("symlink"));

        fs::create_dir(root.join("usr-lib/real-parent")).unwrap();
        fs::set_permissions(
            root.join("usr-lib/real-parent"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        symlink("terminal", root.join("usr-lib/real-parent/leaf.so")).unwrap();
        symlink("real-parent", root.join("usr-lib/linked-parent")).unwrap();
        assert!(map
            .map_link_input(Path::new("/usr/lib/linked-parent/leaf.so"))
            .unwrap_err()
            .contains("ancestor directory is a symlink"));
        fs::remove_dir_all(root).unwrap();
    }

    impl HostAliasMapping {
        fn terminal_receipt_path(&self) -> &str {
            &self.terminal.receipt_path
        }
    }
}
