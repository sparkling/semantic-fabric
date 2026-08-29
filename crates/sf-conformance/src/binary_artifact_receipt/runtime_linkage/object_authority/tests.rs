use super::*;
mod adversarial;
mod prepared_probe;
mod prepared_receipt;
mod prepared_seccomp;
mod prepared_seccomp_canary;
mod runtime_elf_policy;
mod support;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::binary_artifact_receipt::runtime_elf::tests::{
    libc_fixture, loader_fixture, root_fixture, root_fixture_with_needed, shared_fixture,
};
use crate::binary_artifact_receipt::runtime_linkage::{
    build_plan, ResolvedRuntimeObject, VirtualRuntimeObject,
};
use support::{directory, fixture_runtime_elf_policy, fixture_seccomp_policy, mount, regular};

const INTERPRETER: &str = "/lib64/ld-linux-x86-64.so.2";
const LIBC_PATH: &str = "/lib/x86_64-linux-gnu/libc.so.6";
const LOADER_TERMINAL: &str = "/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2";

static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    selected: PathBuf,
    loader_alias: PathBuf,
    loader_terminal: PathBuf,
    libc: PathBuf,
    pair: ArtifactPair,
    plan: RuntimeLoaderPlan,
    view: RuntimeLinkageView,
}

impl Fixture {
    fn new(name: &str) -> Self {
        Self::with_root(name, root_fixture(), vec!["libc.so.6".to_owned()])
    }

    fn with_root(name: &str, root_bytes: Vec<u8>, direct_needed: Vec<String>) -> Self {
        let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "semantic-fabric-runtime-authority-{name}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        directory(&root, 0o700);
        let selected = root.join("selected");
        let linked = root.join("linked");
        regular(&selected, &root_bytes, 0o755);
        fs::hard_link(&selected, &linked).unwrap();
        let pair = ArtifactPair::bind(&selected, &linked).unwrap();

        let lib = root.join("lib");
        let lib64 = root.join("lib64");
        directory(&lib, 0o700);
        directory(&lib64, 0o700);
        let architecture = lib.join("x86_64-linux-gnu");
        directory(&architecture, 0o700);
        let loader_terminal = architecture.join("ld-linux-x86-64.so.2");
        let libc = architecture.join("libc.so.6");
        regular(&loader_terminal, &loader_fixture(), 0o755);
        regular(&libc, &libc_fixture(), 0o644);
        let loader_alias = lib64.join("ld-linux-x86-64.so.2");
        symlink(
            "../lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
            &loader_alias,
        )
        .unwrap();
        let bwrap = root.join("test-bwrap");
        regular(&bwrap, b"fixture bubblewrap executable", 0o755);

        let mounts = vec![
            mount(&lib, "/lib"),
            mount(&lib64, "/lib64"),
            mount(&lib, "/usr/lib"),
            mount(&lib64, "/usr/lib64"),
        ];
        let plan = build_plan(&bwrap, &selected, INTERPRETER, mounts).unwrap();
        let artifact_sha256 = format!("{:x}", Sha256::digest(&root_bytes));
        let view = RuntimeLinkageView {
            artifact_sha256,
            elf_interpreter: INTERPRETER.to_owned(),
            direct_needed,
            loader_path: INTERPRETER.to_owned(),
            resolved_objects: vec![ResolvedRuntimeObject {
                soname: "libc.so.6".to_owned(),
                resolved_path: LIBC_PATH.to_owned(),
            }],
            virtual_objects: vec![VirtualRuntimeObject {
                name: "linux-vdso.so.1".to_owned(),
            }],
        };
        Self {
            root,
            selected,
            loader_alias,
            loader_terminal,
            libc,
            pair,
            plan,
            view,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn holds_static_needed_set_equality_as_sealed_bytes() {
    let fixture = Fixture::new("complete");
    let held = hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).unwrap();

    assert_eq!(held.identities().len(), 3);
    assert_eq!(held.identities()[0].role, RuntimeElfRole::RootPie);
    assert_eq!(held.identities()[1].role, RuntimeElfRole::Loader);
    assert_eq!(held.identities()[2].role, RuntimeElfRole::Libc);
    assert_eq!(held.identities()[2].soname.as_deref(), Some("libc.so.6"));
    assert_eq!(
        held.identities()[0].logical_path,
        fixture.selected.to_str().unwrap()
    );
    assert_eq!(held.identities()[1].logical_path, INTERPRETER);
    assert_eq!(held.identities()[2].logical_path, LIBC_PATH);
    assert!(held
        .identities()
        .iter()
        .all(|identity| identity.device != 0 && identity.inode != 0));
    assert_eq!(
        held.identities()[2].sha256,
        format!("{:x}", Sha256::digest(libc_fixture()))
    );
    assert_eq!(
        held.total_bytes(),
        held.identities()
            .iter()
            .map(|identity| identity.byte_length)
            .sum::<u64>()
    );
    held.assert_current().unwrap();

    for object in [&held.artifact, &held.loader, &held.objects[0]] {
        // SAFETY: the descriptor and buffer are valid; the write must be denied
        // by the already-verified immutable seal set.
        let result =
            unsafe { libc::pwrite(object.sealed.file.as_raw_fd(), b"x".as_ptr().cast(), 1, 0) };
        assert_eq!(result, -1);
        // SAFETY: ftruncate operates on a live owned descriptor and both
        // shrinking and growth must be denied by the seal set.
        assert_eq!(
            unsafe { libc::ftruncate(object.sealed.file.as_raw_fd(), 0) },
            -1
        );
        assert_eq!(
            unsafe {
                libc::ftruncate(
                    object.sealed.file.as_raw_fd(),
                    object.sealed.byte_length as libc::off_t + 1,
                )
            },
            -1
        );
        // SAFETY: fcntl reads flags from a live owned descriptor.
        let flags = unsafe { libc::fcntl(object.sealed.file.as_raw_fd(), libc::F_GETFD) };
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
        // SAFETY: fcntl reads seals from a live memfd descriptor.
        let seals = unsafe { libc::fcntl(object.sealed.file.as_raw_fd(), libc::F_GET_SEALS) };
        assert_eq!(
            seals,
            libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL
        );
    }
}

#[test]
fn rejects_digest_interpreter_and_needed_drift() {
    let mut digest = Fixture::new("digest-drift");
    digest.view.artifact_sha256 = "0".repeat(64);
    assert!(hold_runtime_inputs(&digest.pair, &digest.plan, &digest.view).is_err());

    let mut interpreter = Fixture::new("interpreter-drift");
    interpreter.view.elf_interpreter = "/lib64/not-the-loader.so".to_owned();
    assert!(hold_runtime_inputs(&interpreter.pair, &interpreter.plan, &interpreter.view).is_err());

    let mut needed = Fixture::new("needed-drift");
    needed.view.direct_needed = vec!["libz.so.1".to_owned()];
    assert!(hold_runtime_inputs(&needed.pair, &needed.plan, &needed.view).is_err());
}

#[test]
fn rejects_non_loader_aliases_absolute_aliases_and_multihop_aliases() {
    let ordinary = Fixture::new("ordinary-alias");
    let real_libc = ordinary.libc.with_extension("real");
    fs::rename(&ordinary.libc, &real_libc).unwrap();
    symlink("libc.real", &ordinary.libc).unwrap();
    assert!(hold_runtime_inputs(&ordinary.pair, &ordinary.plan, &ordinary.view).is_err());

    let absolute = Fixture::new("absolute-alias");
    fs::remove_file(&absolute.loader_alias).unwrap();
    symlink(LOADER_TERMINAL, &absolute.loader_alias).unwrap();
    assert!(hold_runtime_inputs(&absolute.pair, &absolute.plan, &absolute.view).is_err());

    let multihop = Fixture::new("multihop-alias");
    fs::remove_file(&multihop.loader_alias).unwrap();
    symlink("next-loader", &multihop.loader_alias).unwrap();
    symlink(
        "../lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
        multihop.root.join("lib64/next-loader"),
    )
    .unwrap();
    assert!(hold_runtime_inputs(&multihop.pair, &multihop.plan, &multihop.view).is_err());
}

#[test]
fn rejects_noncanonical_loader_alias_topology() {
    for (name, target) in [
        (
            "dot-component",
            "../lib/./x86_64-linux-gnu/ld-linux-x86-64.so.2",
        ),
        (
            "repeated-separator",
            "../lib//x86_64-linux-gnu/ld-linux-x86-64.so.2",
        ),
        (
            "trailing-separator",
            "../lib/x86_64-linux-gnu/ld-linux-x86-64.so.2/",
        ),
    ] {
        let fixture = Fixture::new(name);
        fs::remove_file(&fixture.loader_alias).unwrap();
        symlink(target, &fixture.loader_alias).unwrap();
        assert!(hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).is_err());
    }
}

#[test]
fn rejects_symlink_ancestors_loader_mode_and_hardlinked_objects() {
    let ancestor = Fixture::new("ancestor-alias");
    let real_architecture = ancestor.root.join("real-architecture");
    fs::rename(
        ancestor.root.join("lib/x86_64-linux-gnu"),
        &real_architecture,
    )
    .unwrap();
    symlink(
        "../real-architecture",
        ancestor.root.join("lib/x86_64-linux-gnu"),
    )
    .unwrap();
    assert!(hold_runtime_inputs(&ancestor.pair, &ancestor.plan, &ancestor.view).is_err());

    let loader_mode = Fixture::new("loader-mode");
    fs::set_permissions(
        &loader_mode.loader_terminal,
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(hold_runtime_inputs(&loader_mode.pair, &loader_mode.plan, &loader_mode.view).is_err());

    let linked = Fixture::new("object-hardlink");
    fs::hard_link(&linked.libc, linked.root.join("lib/libc-copy")).unwrap();
    assert!(hold_runtime_inputs(&linked.pair, &linked.plan, &linked.view).is_err());
}

#[test]
fn rejects_a_writable_intermediate_runtime_directory() {
    let fixture = Fixture::new("writable-intermediate");
    fs::set_permissions(
        fixture.root.join("lib/x86_64-linux-gnu"),
        fs::Permissions::from_mode(0o777),
    )
    .unwrap();

    assert!(hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).is_err());
}

#[test]
fn held_parent_descriptor_defeats_a_rename_between_resolution_and_open() {
    let fixture = Fixture::new("parent-rename");
    let roots = linux::hold_mounts(&fixture.plan.mounts).unwrap();
    let mount = roots
        .iter()
        .find(|mount| mount.destination == "/lib")
        .unwrap();
    let leaf = linux::component::open_leaf(mount, Path::new("x86_64-linux-gnu/libc.so.6")).unwrap();
    let original = leaf.handle.metadata().unwrap();
    let architecture = fixture.root.join("lib/x86_64-linux-gnu");
    fs::rename(&architecture, fixture.root.join("old-architecture")).unwrap();
    directory(&architecture, 0o700);
    regular(&architecture.join("libc.so.6"), &loader_fixture(), 0o644);

    let opened = linux::component::open_regular(&leaf).unwrap();
    let metadata = opened.metadata().unwrap();
    assert_eq!(
        (metadata.dev(), metadata.ino()),
        (original.dev(), original.ino())
    );
    assert_ne!(
        metadata.ino(),
        fs::metadata(architecture.join("libc.so.6")).unwrap().ino()
    );
}

#[test]
fn object_resolution_forbids_nested_mount_crossing() {
    assert!(Path::new("/proc/version").is_file());
    let error = linux::open_object_beneath_filesystem_root(Path::new("proc/version")).unwrap_err();
    assert!(error.contains("openat2 failed"), "{error}");
}

#[test]
fn rejects_an_unreachable_but_well_formed_extra_provider() {
    let mut fixture = Fixture::new("unreachable-provider");
    let extra = fixture.root.join("lib/x86_64-linux-gnu/libextra.so.1");
    regular(
        &extra,
        &shared_fixture(&["libc.so.6"], "libextra.so.1"),
        0o644,
    );
    fixture.view.resolved_objects.push(ResolvedRuntimeObject {
        soname: "libextra.so.1".to_owned(),
        resolved_path: "/lib/x86_64-linux-gnu/libextra.so.1".to_owned(),
    });

    assert!(hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).is_err());
}

#[test]
fn accepts_a_reachable_transitive_cycle_and_rejects_a_missing_provider() {
    let direct = vec!["libc.so.6".to_owned(), "libentry.so.1".to_owned()];
    let mut cycle = Fixture::with_root(
        "reachable-cycle",
        root_fixture_with_needed(&["libc.so.6", "libentry.so.1"]),
        direct.clone(),
    );
    let entry = cycle.root.join("lib/x86_64-linux-gnu/libentry.so.1");
    let cycle_object = cycle.root.join("lib/x86_64-linux-gnu/libcycle.so.1");
    regular(
        &entry,
        &shared_fixture(&["libcycle.so.1"], "libentry.so.1"),
        0o644,
    );
    regular(
        &cycle_object,
        &shared_fixture(&["libentry.so.1"], "libcycle.so.1"),
        0o644,
    );
    cycle.view.resolved_objects.extend([
        ResolvedRuntimeObject {
            soname: "libentry.so.1".to_owned(),
            resolved_path: "/lib/x86_64-linux-gnu/libentry.so.1".to_owned(),
        },
        ResolvedRuntimeObject {
            soname: "libcycle.so.1".to_owned(),
            resolved_path: "/lib/x86_64-linux-gnu/libcycle.so.1".to_owned(),
        },
    ]);
    let held = hold_runtime_inputs(&cycle.pair, &cycle.plan, &cycle.view).unwrap();
    assert_eq!(
        held.identities()
            .iter()
            .map(|identity| (identity.role, identity.soname.as_deref()))
            .collect::<Vec<_>>(),
        vec![
            (RuntimeElfRole::RootPie, None),
            (RuntimeElfRole::Loader, Some("ld-linux-x86-64.so.2")),
            (RuntimeElfRole::Libc, Some("libc.so.6")),
            (RuntimeElfRole::SharedObject, Some("libcycle.so.1")),
            (RuntimeElfRole::SharedObject, Some("libentry.so.1")),
        ]
    );

    let mut missing = Fixture::with_root(
        "missing-provider",
        root_fixture_with_needed(&["libc.so.6", "libentry.so.1"]),
        direct,
    );
    let entry = missing.root.join("lib/x86_64-linux-gnu/libentry.so.1");
    regular(
        &entry,
        &shared_fixture(&["libmissing.so.1"], "libentry.so.1"),
        0o644,
    );
    missing.view.resolved_objects.push(ResolvedRuntimeObject {
        soname: "libentry.so.1".to_owned(),
        resolved_path: "/lib/x86_64-linux-gnu/libentry.so.1".to_owned(),
    });
    let error = hold_runtime_inputs(&missing.pair, &missing.plan, &missing.view).unwrap_err();
    assert!(error.contains("no provider for libmissing.so.1"), "{error}");
}

#[test]
fn sealed_bytes_survive_source_mutation_while_currentness_fails() {
    let fixture = Fixture::new("source-mutation");
    let held = hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).unwrap();
    let sealed_before = held.objects[0].sealed.bytes.clone();

    let mut changed = libc_fixture();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    regular(&fixture.libc, &changed, 0o644);

    assert!(held.assert_current().is_err());
    linux::assert_sealed_current(&held.objects[0].sealed).unwrap();
    assert_eq!(held.objects[0].sealed.bytes, sealed_before);
}

#[test]
fn detects_loader_alias_and_artifact_path_replacement_after_holding() {
    let alias = Fixture::new("alias-replacement");
    let held_alias = hold_runtime_inputs(&alias.pair, &alias.plan, &alias.view).unwrap();
    fs::remove_file(&alias.loader_alias).unwrap();
    symlink("../lib/x86_64-linux-gnu/libc.so.6", &alias.loader_alias).unwrap();
    assert!(held_alias.assert_current().is_err());
    drop(held_alias);

    let artifact = Fixture::new("artifact-replacement");
    let held_artifact =
        hold_runtime_inputs(&artifact.pair, &artifact.plan, &artifact.view).unwrap();
    let linked = artifact.root.join("linked");
    fs::remove_file(&artifact.selected).unwrap();
    fs::remove_file(&linked).unwrap();
    regular(&artifact.selected, &root_fixture(), 0o755);
    fs::hard_link(&artifact.selected, &linked).unwrap();
    assert!(held_artifact.assert_current().is_err());
}

#[test]
fn detects_runtime_mount_path_replacement_after_holding() {
    let fixture = Fixture::new("mount-replacement");
    let held = hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).unwrap();
    let mount = fixture.root.join("lib");
    fs::rename(&mount, fixture.root.join("old-lib")).unwrap();
    directory(&mount, 0o700);

    assert!(held.assert_current().is_err());
    linux::assert_sealed_current(&held.loader.sealed).unwrap();
    linux::assert_sealed_current(&held.objects[0].sealed).unwrap();
}

#[test]
fn dropping_the_holder_closes_every_sealed_descriptor() {
    let fixture = Fixture::new("drop-closure");
    let held = hold_runtime_inputs(&fixture.pair, &fixture.plan, &fixture.view).unwrap();
    let descriptors = [&held.artifact, &held.loader, &held.objects[0]].map(|object| {
        let descriptor = object.sealed.file.as_raw_fd();
        // SAFETY: the source descriptor is live and F_DUPFD_CLOEXEC returns a
        // new independently owned descriptor on success.
        let sentinel = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 0) };
        assert!(sentinel >= 0);
        // SAFETY: ownership of the new descriptor transfers to this File.
        (descriptor, unsafe { File::from_raw_fd(sentinel) })
    });
    drop(held);

    for (descriptor, sentinel) in descriptors {
        let expected = sentinel.metadata().unwrap();
        let mut current = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: `current` is writable and fstat either initializes it or
        // reports that the closed number has not been reused.
        let status = unsafe { libc::fstat(descriptor, current.as_mut_ptr()) };
        if status == 0 {
            // SAFETY: successful fstat initialized the structure.
            let current = unsafe { current.assume_init() };
            assert_ne!(
                (current.st_dev, current.st_ino),
                (expected.dev(), expected.ino())
            );
        } else {
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EBADF)
            );
        }
    }
}
