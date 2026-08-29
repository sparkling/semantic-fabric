use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::binary_artifact_receipt::runtime_elf::{
    runtime_elf_policy_identity, tests::root_fixture_with_needed,
};
use crate::binary_artifact_receipt::runtime_linkage::object_authority::prepared_probe::{
    seccomp_identity_for_test, ExpectedBwrapIdentity, ExpectedPreparedSeccompPolicy,
    ExpectedRuntimeElfPolicy,
};
use crate::binary_artifact_receipt::runtime_linkage::RuntimeReadOnlyMount;

pub(super) const FIXTURE_BWRAP_NEEDED: [&str; 3] = ["libselinux.so.1", "libcap.so.2", "libc.so.6"];

pub(super) fn fixture_bwrap_bytes() -> Vec<u8> {
    root_fixture_with_needed(&FIXTURE_BWRAP_NEEDED)
}

pub(super) fn fixture_bwrap_identity(path: &Path) -> ExpectedBwrapIdentity {
    let bytes = fixture_bwrap_bytes();
    ExpectedBwrapIdentity::new(
        path,
        &format!("{:x}", Sha256::digest(&bytes)),
        bytes.len() as u64,
        "test-fixture-root-pie-v1",
    )
    .unwrap()
}

pub(super) fn fixture_runtime_elf_policy() -> ExpectedRuntimeElfPolicy {
    let actual = runtime_elf_policy_identity();
    ExpectedRuntimeElfPolicy::new(actual.id(), actual.sha256()).unwrap()
}

pub(super) fn fixture_seccomp_policy() -> ExpectedPreparedSeccompPolicy {
    let actual = seccomp_identity_for_test();
    ExpectedPreparedSeccompPolicy::new(actual.id(), actual.sha256(), actual.byte_length()).unwrap()
}

pub(super) fn mount(source: &Path, destination: &str) -> RuntimeReadOnlyMount {
    RuntimeReadOnlyMount {
        source: source.to_path_buf(),
        destination: destination.to_owned(),
    }
}

pub(super) fn directory(path: &Path, mode: u32) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

pub(super) fn regular(path: &Path, bytes: &[u8], mode: u32) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}
