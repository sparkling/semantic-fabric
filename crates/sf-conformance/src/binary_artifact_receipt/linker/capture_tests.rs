use super::*;
use crate::binary_artifact_receipt::{producer_paths::SandboxPathMap, sandbox};
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};

const OUTPUT: &str =
    "/target/x86_64-unknown-linux-gnu/release/deps/semantic_fabric-0123456789abcdef";

fn fixture() -> (PathBuf, SandboxPathMap) {
    let root = std::env::temp_dir().join(format!(
        "semantic-fabric-link-capture-{}",
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
        "target/x86_64-unknown-linux-gnu",
        "target/x86_64-unknown-linux-gnu/release",
        "target/x86_64-unknown-linux-gnu/release/deps",
        "system",
    ] {
        let path = root.join(directory);
        fs::create_dir_all(&path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mounts = [sandbox::Mount {
        source: root.join("system"),
        destination: "/usr/lib",
    }];
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

fn depfile(inputs: &[&str]) -> Vec<u8> {
    let mut value = format!("{OUTPUT}: \\\n");
    for (index, input) in inputs.iter().enumerate() {
        let continuation = if index + 1 == inputs.len() {
            ""
        } else {
            " \\\n"
        };
        value.push_str(&format!("  {input}{continuation}"));
    }
    value.push_str("\n\n");
    for (index, input) in inputs.iter().enumerate() {
        value.push_str(&format!("{input}:\n"));
        if index + 1 != inputs.len() {
            value.push('\n');
        }
    }
    value.into_bytes()
}

#[test]
fn canonicalizes_alias_and_direct_inputs_and_rechecks_them() {
    let (root, map) = fixture();
    write_private(&root.join("system/terminal.so"), b"terminal");
    symlink("terminal.so", root.join("system/alias.so")).unwrap();
    write_private(
        &root.join("target/x86_64-unknown-linux-gnu/release/deps/semantic_fabric-0123456789abcdef"),
        b"output",
    );
    let inputs = [
        "/usr/lib/alias.so",
        "/usr/lib/terminal.so",
        "/usr/lib/alias.so",
    ];
    let depfile_path = root.join("target/final-link.d");
    write_private(&depfile_path, &depfile(&inputs));

    let captured = capture(&depfile_path, &map).unwrap();
    assert_eq!(captured.raw_input_count, 3);
    assert_eq!(captured.inputs.len(), 1);
    assert_eq!(captured.aliases.len(), 1);
    assert_eq!(
        captured.inputs[0].receipt_path,
        "host-system/usr/lib/terminal.so"
    );
    captured.assert_current(&map).unwrap();

    fs::write(root.join("system/terminal.so"), b"changed!").unwrap();
    assert!(captured.assert_current(&map).is_err());
    drop(captured);
    fs::remove_dir_all(root).unwrap();
}
