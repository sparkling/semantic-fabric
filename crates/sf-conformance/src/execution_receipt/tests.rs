use std::cell::Cell;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        loop {
            let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "semantic-fabric-receipt-unit-{}-{serial}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create isolated test directory: {error}"),
            }
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove isolated test directory");
    }
}

fn source_suite() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf")
}

fn source_receipt() -> PathBuf {
    source_suite().join("sqlite-execution-receipt.tsv")
}

fn never_run<'a>(
    called: &'a Cell<bool>,
) -> impl FnOnce(&SealedSuite) -> Result<ClassifiedReport, String> + 'a {
    move |_| {
        called.set(true);
        Err("runner sentinel executed".to_owned())
    }
}

#[test]
fn malformed_receipt_is_rejected_before_runner_execution() {
    let temp = TempDir::new();
    let receipt = temp.0.join("receipt.tsv");
    fs::write(&receipt, "not a receipt\n").expect("write malformed receipt");
    let called = Cell::new(false);

    let error = check_with_runner(
        Path::new("/suite-must-not-be-opened"),
        &receipt,
        never_run(&called),
    )
    .unwrap_err();
    assert!(
        error.contains("invalid execution receipt header"),
        "{error}"
    );
    assert!(!called.get(), "runner must not execute");
}

#[test]
fn invalid_inventory_is_rejected_before_runner_execution() {
    let temp = TempDir::new();
    fs::write(temp.0.join("inventory.tsv"), b"").expect("write invalid inventory");
    let called = Cell::new(false);

    let error = check_with_runner(&temp.0, &source_receipt(), never_run(&called)).unwrap_err();
    assert_eq!(error, "inventory is empty");
    assert!(!called.get(), "runner must not execute");
}

#[test]
fn oversized_receipt_is_rejected_before_suite_or_runner_access() {
    let temp = TempDir::new();
    let receipt = temp.0.join("oversized.tsv");
    let file = File::create(&receipt).expect("create sparse receipt");
    file.set_len(format::MAX_RECEIPT_BYTES + 1)
        .expect("size sparse receipt");
    let called = Cell::new(false);

    let error = check_with_runner(
        Path::new("/suite-must-not-be-opened"),
        &receipt,
        never_run(&called),
    )
    .unwrap_err();
    assert!(error.contains("exceeds 65536 bytes"), "{error}");
    assert!(!called.get(), "runner must not execute");
}

#[test]
fn parser_rejects_the_eighty_eighth_case_immediately() {
    let mut receipt = fs::read_to_string(source_receipt()).expect("read tracked receipt");
    let extra = receipt
        .lines()
        .rev()
        .find(|line| line.starts_with("case\t"))
        .expect("tracked case")
        .to_owned();
    receipt.push_str(&extra);
    receipt.push('\n');

    let error = format::parse(&receipt).unwrap_err();
    assert!(error.contains("exceeds 87 cases"), "{error}");
}

#[test]
fn selected_backend_mismatch_is_rejected_before_suite_or_runner_access() {
    let called = Cell::new(false);
    let error = check_for_with_runner(
        Path::new("/suite-must-not-be-opened"),
        &source_receipt(),
        Backend::Postgres,
        never_run(&called),
    )
    .unwrap_err();

    assert!(error.contains("backend mismatch"), "{error}");
    assert!(error.contains("requested=postgresql"), "{error}");
    assert!(!called.get(), "runner must not execute");
}

#[test]
fn backend_and_runner_must_be_an_exact_registered_pair() {
    let receipt = fs::read_to_string(source_receipt()).expect("read tracked receipt");
    let forged = receipt.replacen("meta\tbackend\tsqlite", "meta\tbackend\tpostgresql", 1);
    let error = format::parse(&forged).unwrap_err();

    assert!(error.contains("metadata runner"), "{error}");
    assert!(error.contains("run_sealed_suite_required"), "{error}");
}
