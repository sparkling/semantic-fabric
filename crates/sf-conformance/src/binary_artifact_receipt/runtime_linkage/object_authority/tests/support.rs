use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::binary_artifact_receipt::runtime_elf::runtime_elf_policy_identity;
use crate::binary_artifact_receipt::runtime_linkage::object_authority::prepared_probe::{
    seccomp_identity_for_test, ExpectedPreparedSeccompPolicy, ExpectedRuntimeElfPolicy,
};
use crate::binary_artifact_receipt::runtime_linkage::RuntimeReadOnlyMount;

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
