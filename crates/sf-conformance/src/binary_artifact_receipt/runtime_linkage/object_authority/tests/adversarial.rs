use super::*;

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixListener;

#[test]
fn binds_every_identity_field_to_exact_source_metadata_and_bytes() {
    let fixture = Fixture::new("exact-identities");
    let held = hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).unwrap();
    let root_bytes = root_fixture();
    let loader_bytes = loader_fixture();
    let libc_bytes = libc_fixture();
    let expected = [
        (
            &fixture.selected,
            root_bytes.as_slice(),
            fixture.selected.to_str().unwrap(),
            RuntimeElfRole::RootPie,
            None,
        ),
        (
            &fixture.loader_terminal,
            loader_bytes.as_slice(),
            INTERPRETER,
            RuntimeElfRole::Loader,
            Some("ld-linux-x86-64.so.2"),
        ),
        (
            &fixture.libc,
            libc_bytes.as_slice(),
            LIBC_PATH,
            RuntimeElfRole::Libc,
            Some("libc.so.6"),
        ),
    ];

    assert_eq!(held.identities().len(), expected.len());
    for (identity, (path, bytes, logical_path, role, soname)) in
        held.identities().iter().zip(expected)
    {
        let metadata = fs::metadata(path).unwrap();
        assert_eq!(
            (identity.device, identity.inode),
            (metadata.dev(), metadata.ino())
        );
        assert_eq!(identity.byte_length, metadata.len());
        assert_eq!(identity.sha256, format!("{:x}", Sha256::digest(bytes)));
        assert_eq!(identity.logical_path, logical_path);
        assert_eq!(identity.role, role);
        assert_eq!(identity.soname.as_deref(), soname);
    }
}

#[test]
fn rejects_duplicate_source_and_byte_identities_independently() {
    let first = synthetic_identity(1, 2, 3, "a");
    let same_source = synthetic_identity(1, 2, 4, "b");
    let error = validate_unique_authorities([&first, &same_source]).unwrap_err();
    assert!(
        error.contains("duplicate source or byte identity"),
        "{error}"
    );

    let same_bytes = synthetic_identity(5, 6, 3, "a");
    let error = validate_unique_authorities([&first, &same_bytes]).unwrap_err();
    assert!(
        error.contains("duplicate source or byte identity"),
        "{error}"
    );
}

#[test]
fn rejects_directory_fifo_and_unix_socket_runtime_leaves_without_blocking() {
    let directory_leaf = Fixture::new("directory-leaf");
    fs::remove_file(&directory_leaf.libc).unwrap();
    directory(&directory_leaf.libc, 0o700);
    assert!(hold_runtime_inputs(
        &directory_leaf.pair,
        &directory_leaf.plan,
        &directory_leaf.view
    )
    .is_err());

    let fifo_leaf = Fixture::new("fifo-leaf");
    fs::remove_file(&fifo_leaf.libc).unwrap();
    let fifo = CString::new(fifo_leaf.libc.as_os_str().as_bytes()).unwrap();
    // SAFETY: the path is NUL terminated and names an absent fixture leaf.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(hold_runtime_inputs(&fifo_leaf.pair, &fifo_leaf.plan, &fifo_leaf.view).is_err());

    let socket_leaf = Fixture::new("socket-leaf");
    fs::remove_file(&socket_leaf.libc).unwrap();
    let _listener = UnixListener::bind(&socket_leaf.libc).unwrap();
    assert!(hold_runtime_inputs(&socket_leaf.pair, &socket_leaf.plan, &socket_leaf.view).is_err());
}

#[test]
fn enforces_zero_aggregate_overflow_and_object_count_boundaries() {
    let mut total = 0;
    assert!(reserve_bytes(&mut total, 0).is_err());
    reserve_bytes(&mut total, MAX_RUNTIME_OBJECT_BYTES).unwrap();
    assert!(reserve_bytes(&mut total, MAX_RUNTIME_OBJECT_BYTES + 1).is_err());
    assert!(reserve_bytes(&mut total, MAX_RUNTIME_OBJECT_BYTES).is_ok());
    assert_eq!(total, MAX_RUNTIME_SET_BYTES);
    assert!(reserve_bytes(&mut total, 1).is_err());

    let mut overflow = u64::MAX;
    assert!(reserve_bytes(&mut overflow, 1).is_err());

    let mut fixture = Fixture::new("object-count");
    fixture
        .view
        .resolved_objects
        .extend(
            (0..MAX_RESOLVED_OBJECTS).map(|index| ResolvedRuntimeObject {
                soname: format!("libbound{index}.so"),
                resolved_path: format!("/lib/x86_64-linux-gnu/libbound{index}.so"),
            }),
        );
    let error = hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).unwrap_err();
    assert!(error.contains("object count exceeds"), "{error}");
}

#[test]
fn detects_same_byte_inode_replacement_and_post_bind_hardlink_addition() {
    let replacement = Fixture::new("same-byte-replacement");
    let held =
        hold_runtime_inputs(&replacement.pair, &replacement.plan, &replacement.view).unwrap();
    fs::rename(&replacement.libc, replacement.libc.with_extension("old")).unwrap();
    regular(&replacement.libc, &libc_fixture(), 0o644);
    assert!(held.assert_current().is_err());
    linux::assert_sealed_current(&held.objects[0].sealed).unwrap();

    let hardlink = Fixture::new("post-bind-hardlink");
    let held = hold_runtime_inputs(&hardlink.pair, &hardlink.plan, &hardlink.view).unwrap();
    fs::hard_link(&hardlink.libc, hardlink.libc.with_extension("copy")).unwrap();
    assert!(held.assert_current().is_err());
}

#[test]
fn final_source_metadata_fence_detects_after_second_read_mutation() {
    let fixture = Fixture::new("final-source-fence");
    let source = fixture.pair.duplicate_selected().unwrap();
    let identity = linux::inspect_artifact(&source).unwrap();
    let error = linux::snapshot_source_with_phase_hook(
        &source,
        identity,
        MAX_RUNTIME_OBJECT_BYTES,
        "phase-fence artifact",
        || {
            fs::set_permissions(&fixture.selected, fs::Permissions::from_mode(0o744)).unwrap();
        },
    )
    .unwrap_err();
    assert!(error.contains("changed while snapshotting"), "{error}");
}

#[test]
fn final_phase_fences_detect_post_object_mount_and_artifact_replacement() {
    let mount = Fixture::new("final-mount-fence");
    let held = hold_runtime_inputs(&mount.pair, &mount.plan, &mount.view).unwrap();
    let error = held
        .assert_current_with_phase_hook(|| {
            let source = mount.root.join("lib");
            fs::rename(&source, mount.root.join("old-lib")).unwrap();
            directory(&source, 0o700);
        })
        .unwrap_err();
    assert!(error.contains("runtime mount"), "{error}");

    let artifact = Fixture::new("final-artifact-fence");
    let held = hold_runtime_inputs(&artifact.pair, &artifact.plan, &artifact.view).unwrap();
    assert!(held
        .assert_current_with_phase_hook(|| {
            let linked = artifact.root.join("linked");
            fs::remove_file(&artifact.selected).unwrap();
            fs::remove_file(&linked).unwrap();
            regular(&artifact.selected, &root_fixture(), 0o755);
            fs::hard_link(&artifact.selected, &linked).unwrap();
        })
        .is_err());
}

#[test]
fn rejects_zero_device_or_inode_before_record_conversion() {
    let zero_device = synthetic_identity(0, 1, 1, "a");
    let zero_inode = synthetic_identity(1, 0, 2, "b");
    assert!(validate_unique_authorities([&zero_device]).is_err());
    assert!(validate_unique_authorities([&zero_inode]).is_err());
}

fn synthetic_identity(
    device: u64,
    inode: u64,
    byte_length: u64,
    sha256: &str,
) -> RuntimeObjectIdentity {
    RuntimeObjectIdentity {
        logical_path: format!("/synthetic/{device}/{inode}"),
        role: RuntimeElfRole::SharedObject,
        soname: Some(format!("lib{device}-{inode}.so")),
        device,
        inode,
        byte_length,
        sha256: sha256.to_owned(),
    }
}
