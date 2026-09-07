#![cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]

use std::path::PathBuf;

use sf_conformance::rust_closure_receipt::{
    self, LEGACY_RECEIPT_PATH, PARSER_WORKER_QUALIFICATION_INPUTS_RECEIPT_PATH, RECEIPT_PATH,
};
use sha2::{Digest, Sha256};

const DEFAULT_RECEIPT_BYTES: usize = 167_811;
const DEFAULT_RECEIPT_SHA256: &str =
    "962acd1718322a2ec00486cb207327167718689fc4ff43d3ed312e35673cb17f";

#[test]
fn tracked_default_receipt_is_a_frozen_legacy_contract() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_ne!(LEGACY_RECEIPT_PATH, RECEIPT_PATH);
    let bytes = std::fs::read(root.join(LEGACY_RECEIPT_PATH)).expect("read frozen legacy receipt");

    assert_eq!(bytes.len(), DEFAULT_RECEIPT_BYTES);
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        DEFAULT_RECEIPT_SHA256
    );
}

#[test]
fn tracked_rust_dependency_and_closure_receipt_matches_current_workspace() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let receipt = root.join(RECEIPT_PATH);

    let verified = rust_closure_receipt::check(&root, &receipt)
        .unwrap_or_else(|error| panic!("tracked Rust closure receipt must verify: {error}"));

    assert!(verified.package_count() > 0);
}

#[test]
fn tracked_parser_worker_qualification_inputs_match_current_workspace() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let receipt = root.join(PARSER_WORKER_QUALIFICATION_INPUTS_RECEIPT_PATH);

    let verified = rust_closure_receipt::check_parser_worker_qualification_inputs(&root, &receipt)
        .unwrap_or_else(|error| {
            panic!("tracked parser-worker qualification inputs must verify: {error}")
        });

    assert!(verified.package_count() > 0);
    assert!(verified.context_count() >= verified.package_count());
    assert!(verified.edge_count() > 0);
}
