use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::rust_closure_receipt::{OriginKind, PackageRecord};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> (PathBuf, ControlledCheckRequest<'static>) {
    let serial = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "semantic-fabric-controlled-closure-{}-{serial}",
        std::process::id()
    ));
    for directory in ["source/tests", "toolchain/bin", "cargo-home", "temporary"] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for directory in ["toolchain", "toolchain/bin"] {
            fs::set_permissions(root.join(directory), fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    for tool in ["cargo", "rustc"] {
        let path = root.join("toolchain/bin").join(tool);
        fs::write(&path, b"#!/bin/sh\nexit 99\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    fs::write(root.join("source/Cargo.lock"), b"lock").unwrap();
    fs::write(root.join("source/rust-toolchain.toml"), b"toolchain").unwrap();
    let leaked = Box::leak(Box::new(root.clone()));
    let request = ControlledCheckRequest {
        materialized_source: leaked.join("source").leak(),
        cargo: leaked.join("toolchain/bin/cargo").leak(),
        rustc: leaked.join("toolchain/bin/rustc").leak(),
        toolchain_root: leaked.join("toolchain").leak(),
        cargo_home: leaked.join("cargo-home").leak(),
        temporary_dir: leaked.join("temporary").leak(),
        source_date_epoch: 1_700_000_000,
    };
    (root, request)
}

fn receipt(lock: String, toolchain: String) -> Receipt {
    let mut package = PackageRecord {
        key: String::new(),
        name: "sf-cli".to_owned(),
        version: "0.0.0".to_owned(),
        origin_kind: OriginKind::Workspace,
        origin: "crates/sf-cli/Cargo.toml".to_owned(),
        features: Vec::new(),
        edges: Vec::new(),
    };
    package.key = package.computed_key();
    Receipt::from_parts(
        lock,
        toolchain,
        "cargo fixture",
        "rustc fixture",
        super::super::TARGET,
        vec![package],
    )
    .unwrap()
}

#[test]
fn command_specs_clear_and_set_the_exact_environment() {
    let (root, request) = fixture();
    let context = Context::new(&request).unwrap();
    let command = context.metadata_command();
    let names: Vec<_> = command
        .environment
        .keys()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    assert!(command.clear_environment);
    assert_eq!(
        names,
        [
            "CARGO_HOME",
            "CARGO_INCREMENTAL",
            "CARGO_NET_OFFLINE",
            "HOME",
            "LC_ALL",
            "PATH",
            "RUSTC",
            "RUSTUP_HOME",
            "SOURCE_DATE_EPOCH",
            "TMPDIR",
            "TZ",
        ]
    );
    let args: Vec<_> = command
        .arguments
        .iter()
        .map(|value| value.to_string_lossy())
        .collect();
    assert!(args.iter().any(|value| value == "--locked"));
    assert!(args.iter().any(|value| value == "--offline"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_lock_drift_before_invoking_any_tool() {
    let (root, request) = fixture();
    let expected = receipt("0".repeat(64), super::super::sha256(b"toolchain"));
    fs::write(
        root.join("source").join(super::super::RECEIPT_PATH),
        super::super::format::render(&expected).unwrap(),
    )
    .unwrap();
    let error = check_with_tools(&request).unwrap_err();
    assert!(error.contains("actual Cargo.lock"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_controlled_cargo_home_credentials() {
    let (root, request) = fixture();
    fs::write(root.join("cargo-home/credentials.toml"), b"[registry]").unwrap();
    let error = Context::new(&request)
        .err()
        .expect("credentials must be rejected");
    assert!(error.contains("configuration or credentials"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn rejects_hard_linked_tool() {
    let (root, request) = fixture();
    fs::hard_link(
        root.join("toolchain/bin/cargo"),
        root.join("cargo-hard-link"),
    )
    .unwrap();
    let error = Context::new(&request)
        .err()
        .expect("hard-linked Cargo must be rejected");
    assert!(error.contains("hard link"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn rejects_writable_toolchain_path_component() {
    use std::os::unix::fs::PermissionsExt;

    let (root, request) = fixture();
    fs::set_permissions(
        root.join("toolchain/bin"),
        fs::Permissions::from_mode(0o770),
    )
    .unwrap();
    let error = Context::new(&request)
        .err()
        .expect("group-writable toolchain bin must be rejected");
    assert!(error.contains("writable"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn rejects_group_writable_tool() {
    use std::os::unix::fs::PermissionsExt;

    let (root, request) = fixture();
    fs::set_permissions(
        root.join("toolchain/bin/cargo"),
        fs::Permissions::from_mode(0o720),
    )
    .unwrap();
    let error = Context::new(&request)
        .err()
        .expect("group-writable Cargo must be rejected");
    assert!(error.contains("writable"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_tool() {
    use std::os::unix::fs::symlink;

    let (root, request) = fixture();
    fs::remove_file(root.join("toolchain/bin/cargo")).unwrap();
    symlink("rustc", root.join("toolchain/bin/cargo")).unwrap();
    let error = Context::new(&request)
        .err()
        .expect("symlinked Cargo must be rejected");
    assert!(error.contains("canonical"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_toolchain_path_component() {
    use std::os::unix::fs::symlink;

    let (root, request) = fixture();
    fs::rename(root.join("toolchain/bin"), root.join("toolchain/real-bin")).unwrap();
    symlink("real-bin", root.join("toolchain/bin")).unwrap();
    let error = Context::new(&request)
        .err()
        .expect("symlinked toolchain component must be rejected");
    assert!(error.contains("canonical"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn detects_same_byte_tool_replacement() {
    let (root, request) = fixture();
    let context = Context::new(&request).unwrap();
    let before = context.tool_fingerprints().unwrap();
    let cargo = root.join("toolchain/bin/cargo");
    let replacement = root.join("cargo-replacement");
    fs::write(&replacement, fs::read(&cargo).unwrap()).unwrap();
    fs::set_permissions(&replacement, fs::metadata(&cargo).unwrap().permissions()).unwrap();
    fs::rename(&replacement, &cargo).unwrap();
    let after = context.tool_fingerprints().unwrap();
    assert!(
        after != before,
        "same-byte inode replacement was not detected"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn rejects_tool_replacement_during_subprocess() {
    use std::os::unix::fs::PermissionsExt;

    let (root, request) = fixture();
    let cargo = root.join("toolchain/bin/cargo");
    fs::write(
        &cargo,
        b"#!/bin/sh\n/bin/cp \"$RUSTC\" \"$RUSTC.replacement\"\n/bin/mv \"$RUSTC.replacement\" \"$RUSTC\"\nprintf 'cargo fixture\\nhost: x86_64-unknown-linux-gnu\\n'\n",
    )
    .unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700)).unwrap();
    let context = Context::new(&request).unwrap();
    let error = context
        .cargo_command(&["-Vv"])
        .output(
            "mutating Cargo fixture",
            super::super::MAX_TOOL_OUTPUT_BYTES,
            super::super::TOOL_TIMEOUT,
        )
        .unwrap_err();
    assert!(error.contains("changed"), "{error}");
    fs::remove_dir_all(root).unwrap();
}
