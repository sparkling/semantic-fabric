use super::*;
use crate::binary_artifact_receipt::{
    model::LinkInputOrigin,
    producer_paths::{HostAliasMapping, MappedPath},
};
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "semantic-fabric-host-link-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(root.join("lib"), fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn write_private(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn mapping(root: &Path) -> HostAliasMapping {
    HostAliasMapping {
        alias_backing: root.join("lib/alias.so"),
        alias_receipt_path: "host-system-alias/usr/lib/alias.so".to_owned(),
        alias_root: root.join("lib"),
        raw_target: b"terminal.so".to_vec(),
        terminal_logical: PathBuf::from("/usr/lib/terminal.so"),
        terminal: MappedPath {
            backing: root.join("lib/terminal.so"),
            origin: LinkInputOrigin::HostSystem,
            receipt_path: "host-system/usr/lib/terminal.so".to_owned(),
        },
        terminal_root: root.join("lib"),
    }
}

#[test]
fn binds_and_rechecks_the_alias_and_terminal() {
    let root = fixture("bind");
    write_private(&root.join("lib/terminal.so"), b"terminal");
    symlink("terminal.so", root.join("lib/alias.so")).unwrap();
    let authority = HostLinkAuthority::bind(mapping(&root), 1024).unwrap();
    assert_eq!(authority.terminal_byte_length, 8);
    assert_eq!(authority.resolution_sha256.len(), 64);
    authority.assert_current().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn detects_persistent_alias_and_terminal_changes() {
    let alias_root = fixture("alias-change");
    write_private(&alias_root.join("lib/terminal.so"), b"terminal");
    symlink("terminal.so", alias_root.join("lib/alias.so")).unwrap();
    let alias_authority = HostLinkAuthority::bind(mapping(&alias_root), 1024).unwrap();
    fs::remove_file(alias_root.join("lib/alias.so")).unwrap();
    symlink("terminal.so", alias_root.join("lib/alias.so")).unwrap();
    assert!(alias_authority.assert_current().is_err());
    drop(alias_authority);
    fs::remove_dir_all(alias_root).unwrap();

    let terminal_root = fixture("terminal-change");
    write_private(&terminal_root.join("lib/terminal.so"), b"terminal");
    symlink("terminal.so", terminal_root.join("lib/alias.so")).unwrap();
    let terminal_authority = HostLinkAuthority::bind(mapping(&terminal_root), 1024).unwrap();
    fs::write(terminal_root.join("lib/terminal.so"), b"changed!").unwrap();
    assert!(terminal_authority.assert_current().is_err());
    drop(terminal_authority);
    fs::remove_dir_all(terminal_root).unwrap();
}

#[test]
fn rejects_hardlinked_writable_and_nonregular_terminals() {
    let alias_root = fixture("alias-hardlink");
    write_private(&alias_root.join("lib/terminal.so"), b"terminal");
    symlink("terminal.so", alias_root.join("lib/alias.so")).unwrap();
    fs::hard_link(
        alias_root.join("lib/alias.so"),
        alias_root.join("lib/alias-copy.so"),
    )
    .unwrap();
    assert!(HostLinkAuthority::bind(mapping(&alias_root), 1024)
        .unwrap_err()
        .contains("link count"));
    fs::remove_dir_all(alias_root).unwrap();

    let hardlink_root = fixture("hardlink");
    write_private(&hardlink_root.join("lib/terminal.so"), b"terminal");
    fs::hard_link(
        hardlink_root.join("lib/terminal.so"),
        hardlink_root.join("lib/other.so"),
    )
    .unwrap();
    symlink("terminal.so", hardlink_root.join("lib/alias.so")).unwrap();
    assert!(HostLinkAuthority::bind(mapping(&hardlink_root), 1024)
        .unwrap_err()
        .contains("hard link"));
    fs::remove_dir_all(hardlink_root).unwrap();

    let writable_root = fixture("writable");
    write_private(&writable_root.join("lib/terminal.so"), b"terminal");
    fs::set_permissions(
        writable_root.join("lib/terminal.so"),
        fs::Permissions::from_mode(0o666),
    )
    .unwrap();
    symlink("terminal.so", writable_root.join("lib/alias.so")).unwrap();
    assert!(HostLinkAuthority::bind(mapping(&writable_root), 1024)
        .unwrap_err()
        .contains("writable"));
    fs::remove_dir_all(writable_root).unwrap();

    let directory_root = fixture("directory");
    fs::create_dir(directory_root.join("lib/terminal.so")).unwrap();
    fs::set_permissions(
        directory_root.join("lib/terminal.so"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    symlink("terminal.so", directory_root.join("lib/alias.so")).unwrap();
    assert!(HostLinkAuthority::bind(mapping(&directory_root), 1024)
        .unwrap_err()
        .contains("not a regular"));
    fs::remove_dir_all(directory_root).unwrap();
}

#[test]
fn resolution_digest_binds_raw_alias_topology() {
    let first = resolution_digest(
        "host-system-alias/lib/a",
        b"../lib/a.1",
        "/lib/a.1",
        "host-system/usr/lib/a.1",
    );
    assert_ne!(
        first,
        resolution_digest(
            "host-system-alias/lib/a",
            b"a.1",
            "/lib/a.1",
            "host-system/usr/lib/a.1",
        )
    );
}
