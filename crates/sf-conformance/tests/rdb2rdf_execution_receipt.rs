use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use sf_conformance::execution_receipt;
use sf_conformance::Status;
use sha2::{Digest, Sha256};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

fn source_suite() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf")
}

fn source_receipt() -> PathBuf {
    source_suite().join("sqlite-execution-receipt.tsv")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        loop {
            let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "semantic-fabric-rdb2rdf-receipt-{}-{serial}",
                std::process::id()
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create isolated receipt directory: {error}"),
            }
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove isolated receipt directory");
    }
}

struct TempReceipt {
    path: PathBuf,
    _directory: TempDir,
}

impl TempReceipt {
    fn copy() -> Self {
        let directory = TempDir::new();
        let path = directory.0.join("receipt.tsv");
        fs::copy(source_receipt(), &path).expect("copy execution receipt");
        Self {
            path,
            _directory: directory,
        }
    }
}

fn replace_once(path: &Path, from: &str, to: &str) {
    let original = fs::read_to_string(path).expect("read mutation target");
    assert_eq!(original.matches(from).count(), 1, "unique mutation token");
    fs::write(path, original.replacen(from, to, 1)).expect("write mutation target");
}

fn metadata_value(path: &Path, key: &str) -> String {
    let prefix = format!("meta\t{key}\t");
    fs::read_to_string(path)
        .expect("read receipt metadata")
        .lines()
        .find_map(|line| line.strip_prefix(&prefix).map(str::to_owned))
        .unwrap_or_else(|| panic!("missing receipt metadata {key}"))
}

fn make_metadata_self_consistent(path: &Path) {
    let text = fs::read_to_string(path).expect("read mutated receipt");
    let mut case_count = 0;
    let mut r2rml_count = 0;
    let mut direct_count = 0;
    let mut passed_count = 0;
    let mut failed_count = 0;
    let mut skipped_count = 0;
    let mut outcome_records = String::new();
    for line in text.lines().filter(|line| line.starts_with("case\t")) {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 5, "canonical typed case record");
        case_count += 1;
        match fields[2] {
            "r2rml" => r2rml_count += 1,
            "direct-mapping" => direct_count += 1,
            other => panic!("unknown test kind {other}"),
        }
        match fields[3] {
            "passed" => passed_count += 1,
            "failed" => failed_count += 1,
            "skipped" => skipped_count += 1,
            other => panic!("unknown test status {other}"),
        }
        outcome_records.push_str(line);
        outcome_records.push('\n');
    }
    let outcomes_sha256 = format!("{:x}", Sha256::digest(outcome_records.as_bytes()));
    let replacements = [
        ("case-count", case_count.to_string()),
        ("r2rml-count", r2rml_count.to_string()),
        ("direct-mapping-count", direct_count.to_string()),
        ("passed-count", passed_count.to_string()),
        ("failed-count", failed_count.to_string()),
        ("skipped-count", skipped_count.to_string()),
        ("outcomes-sha256", outcomes_sha256),
    ];
    let mut lines: Vec<_> = text.lines().map(str::to_owned).collect();
    for (key, value) in replacements {
        let prefix = format!("meta\t{key}\t");
        let matches: Vec<_> = lines
            .iter_mut()
            .filter(|line| line.starts_with(&prefix))
            .collect();
        assert_eq!(matches.len(), 1, "unique {key} metadata");
        *matches.into_iter().next().unwrap() = format!("{prefix}{value}");
    }
    fs::write(path, format!("{}\n", lines.join("\n"))).expect("write repaired metadata");
}

#[derive(Debug, PartialEq, Eq)]
enum TreeNode {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
}

fn suite_snapshot(root: &Path) -> BTreeMap<PathBuf, TreeNode> {
    let mut snapshot = BTreeMap::new();
    snapshot_directory(root, root, &mut snapshot);
    snapshot
}

fn snapshot_directory(root: &Path, directory: &Path, snapshot: &mut BTreeMap<PathBuf, TreeNode>) {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .expect("read snapshot directory")
        .map(|entry| entry.expect("read snapshot entry"))
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap().to_owned();
        let kind = entry.file_type().expect("read snapshot entry type");
        if kind.is_dir() {
            snapshot.insert(relative, TreeNode::Directory);
            snapshot_directory(root, &path, snapshot);
        } else if kind.is_file() {
            snapshot.insert(relative, TreeNode::File(fs::read(path).unwrap()));
        } else if kind.is_symlink() {
            snapshot.insert(relative, TreeNode::Symlink(fs::read_link(path).unwrap()));
        } else {
            panic!("unsupported suite node {}", path.display());
        }
    }
}

#[test]
fn production_check_replays_exact_outcomes_and_is_whole_suite_neutral() {
    let before = suite_snapshot(&source_suite());
    let receipt = execution_receipt::check(&source_suite(), &source_receipt())
        .expect("production check matches replay");

    assert_eq!(receipt.cases().len(), 87);
    assert_eq!(receipt.count(Status::Passed), 81);
    assert_eq!(receipt.count(Status::Failed), 1);
    assert_eq!(receipt.count(Status::Skipped), 5);
    assert_eq!(
        receipt.inventory_sha256(),
        format!(
            "{:x}",
            Sha256::digest(fs::read(source_suite().join("inventory.tsv")).unwrap())
        )
    );
    assert_eq!(suite_snapshot(&source_suite()), before);
}

#[test]
fn inventory_digest_substitution_fails_closed() {
    let receipt = TempReceipt::copy();
    let inventory_sha256 = metadata_value(&receipt.path, "inventory-sha256");
    let replacement = if inventory_sha256.starts_with('0') {
        "f".repeat(64)
    } else {
        "0".repeat(64)
    };
    replace_once(&receipt.path, &inventory_sha256, &replacement);

    let error = execution_receipt::check(&source_suite(), &receipt.path).unwrap_err();
    assert!(error.contains("inventory digest mismatch"), "{error}");
}

#[test]
fn count_neutral_per_id_status_mutation_fails_self_verification() {
    let receipt = TempReceipt::copy();
    replace_once(
        &receipt.path,
        "case\tR2RMLTC0002f\tr2rml\tfailed\tunexpected-output",
        "case\tR2RMLTC0002f\tr2rml\tpassed\texecution-error",
    );
    replace_once(
        &receipt.path,
        "case\tR2RMLTC0002g\tr2rml\tpassed\texecution-error",
        "case\tR2RMLTC0002g\tr2rml\tfailed\tunexpected-output",
    );

    let error = execution_receipt::check(&source_suite(), &receipt.path).unwrap_err();
    assert!(error.contains("outcomes digest mismatch"), "{error}");
}

#[test]
fn self_consistent_allowed_improvement_requires_receipt_regeneration() {
    let receipt = TempReceipt::copy();
    replace_once(
        &receipt.path,
        "case\tR2RMLTC0002f\tr2rml\tfailed\tunexpected-output",
        "case\tR2RMLTC0002f\tr2rml\tpassed\texecution-error",
    );
    make_metadata_self_consistent(&receipt.path);

    let error = execution_receipt::check(&source_suite(), &receipt.path).unwrap_err();
    assert!(error.contains("R2RMLTC0002f"), "{error}");
    assert!(error.contains("outcome mismatch"), "{error}");
}

#[test]
fn self_consistent_same_status_outcome_code_drift_is_rejected() {
    let receipt = TempReceipt::copy();
    replace_once(
        &receipt.path,
        "case\tDirectGraphTC0001\tdirect-mapping\tpassed\tgraph-matched",
        "case\tDirectGraphTC0001\tdirect-mapping\tpassed\tdataset-matched",
    );
    make_metadata_self_consistent(&receipt.path);

    let error = execution_receipt::check(&source_suite(), &receipt.path).unwrap_err();
    assert!(error.contains("DirectGraphTC0001"), "{error}");
    assert!(error.contains("graph-matched"), "{error}");
    assert!(error.contains("dataset-matched"), "{error}");
}

#[test]
fn self_consistent_known_deviation_cause_drift_fails_policy() {
    let receipt = TempReceipt::copy();
    replace_once(
        &receipt.path,
        "case\tR2RMLTC0002f\tr2rml\tfailed\tunexpected-output",
        "case\tR2RMLTC0002f\tr2rml\tfailed\tgraph-mismatch",
    );
    make_metadata_self_consistent(&receipt.path);

    let error = execution_receipt::check(&source_suite(), &receipt.path).unwrap_err();
    assert!(error.contains("R2RMLTC0002f"), "{error}");
    assert!(error.contains("outcome code mismatch"), "{error}");
}

#[test]
fn count_neutral_case_order_mutation_fails_inventory_binding() {
    let receipt = TempReceipt::copy();
    let text = fs::read_to_string(&receipt.path).expect("read receipt");
    let mut lines: Vec<_> = text.lines().map(str::to_owned).collect();
    let first = lines
        .iter()
        .position(|line| line.starts_with("case\t"))
        .expect("first case");
    lines.swap(first, first + 1);
    fs::write(&receipt.path, format!("{}\n", lines.join("\n"))).unwrap();
    make_metadata_self_consistent(&receipt.path);

    let error = execution_receipt::check(&source_suite(), &receipt.path).unwrap_err();
    assert!(error.contains("order/identity mismatch"), "{error}");
}
