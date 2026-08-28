#![cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]

use std::path::PathBuf;

use sf_conformance::rust_closure_receipt::{self, RECEIPT_PATH};

#[test]
fn tracked_rust_dependency_and_closure_receipt_matches_current_workspace() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let receipt = root.join(RECEIPT_PATH);

    let verified = rust_closure_receipt::check(&root, &receipt)
        .unwrap_or_else(|error| panic!("tracked Rust closure receipt must verify: {error}"));

    assert!(verified.package_count() > 0);
}
