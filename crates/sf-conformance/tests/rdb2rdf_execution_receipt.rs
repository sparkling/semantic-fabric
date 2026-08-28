use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use sf_conformance::execution_receipt;
use sf_conformance::{Report, Status};
use sha2::{Digest, Sha256};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);
static ACTUAL_REPORT: OnceLock<Report> = OnceLock::new();

fn source_suite() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf")
}

fn source_receipt() -> PathBuf {
    source_suite().join("sqlite-execution-receipt.tsv")
}

fn actual_report() -> &'static Report {
    ACTUAL_REPORT.get_or_init(|| sf_conformance::run_suite(&source_suite()).expect("suite runs"))
}

struct TempReceipt(PathBuf);

impl TempReceipt {
    fn copy() -> Self {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "semantic-fabric-rdb2rdf-receipt-{}-{serial}.tsv",
            std::process::id()
        ));
        fs::copy(source_receipt(), &path).expect("copy execution receipt");
        Self(path)
    }
}

impl Drop for TempReceipt {
    fn drop(&mut self) {
        fs::remove_file(&self.0).expect("remove temporary receipt");
    }
}

fn replace_once(path: &Path, from: &str, to: &str) {
    let original = fs::read_to_string(path).expect("read mutation target");
    assert_eq!(original.matches(from).count(), 1, "unique mutation token");
    fs::write(path, original.replacen(from, to, 1)).expect("write mutation target");
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
        assert_eq!(fields.len(), 4, "canonical case record");
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

#[test]
fn tracked_receipt_replays_exact_sqlite_outcomes_without_writes() {
    let receipt_before = fs::read(source_receipt()).expect("read tracked receipt");
    let inventory_path = source_suite().join("inventory.tsv");
    let inventory_before = fs::read(&inventory_path).expect("read inventory");

    let receipt =
        execution_receipt::check_report(&source_suite(), &source_receipt(), actual_report())
            .expect("receipt matches replay");

    assert_eq!(receipt.cases().len(), 87);
    assert_eq!(receipt.count(Status::Passed), 81);
    assert_eq!(receipt.count(Status::Failed), 1);
    assert_eq!(receipt.count(Status::Skipped), 5);
    assert_eq!(
        receipt.inventory_sha256(),
        format!("{:x}", Sha256::digest(&inventory_before))
    );
    assert_eq!(
        fs::read(source_receipt()).expect("reread tracked receipt"),
        receipt_before,
        "check path must not rewrite the receipt"
    );
    assert_eq!(
        fs::read(inventory_path).expect("reread inventory"),
        inventory_before,
        "check path must not rewrite the inventory"
    );
}

#[test]
fn inventory_digest_substitution_fails_closed() {
    let receipt = TempReceipt::copy();
    let inventory_sha256 =
        execution_receipt::check_report(&source_suite(), &receipt.0, actual_report())
            .expect("baseline receipt")
            .inventory_sha256()
            .to_owned();
    let replacement = if inventory_sha256.starts_with('0') {
        "f".repeat(64)
    } else {
        "0".repeat(64)
    };
    replace_once(&receipt.0, &inventory_sha256, &replacement);

    let error =
        execution_receipt::check_report(&source_suite(), &receipt.0, actual_report()).unwrap_err();
    assert!(error.contains("inventory digest mismatch"), "{error}");
}

#[test]
fn count_neutral_per_id_outcome_mutation_fails_self_verification() {
    let receipt = TempReceipt::copy();
    replace_once(
        &receipt.0,
        "case\tR2RMLTC0002f\tr2rml\tfailed",
        "case\tR2RMLTC0002f\tr2rml\tpassed",
    );
    replace_once(
        &receipt.0,
        "case\tR2RMLTC0002g\tr2rml\tpassed",
        "case\tR2RMLTC0002g\tr2rml\tfailed",
    );

    let error =
        execution_receipt::check_report(&source_suite(), &receipt.0, actual_report()).unwrap_err();
    assert!(error.contains("outcomes digest mismatch"), "{error}");
}

#[test]
fn self_consistent_allowed_improvement_still_requires_receipt_regeneration() {
    let receipt = TempReceipt::copy();
    replace_once(
        &receipt.0,
        "case\tR2RMLTC0002f\tr2rml\tfailed",
        "case\tR2RMLTC0002f\tr2rml\tpassed",
    );
    make_metadata_self_consistent(&receipt.0);

    let error =
        execution_receipt::check_report(&source_suite(), &receipt.0, actual_report()).unwrap_err();
    assert!(error.contains("R2RMLTC0002f"), "{error}");
    assert!(error.contains("outcome mismatch"), "{error}");
}

#[test]
fn count_neutral_case_order_mutation_fails_inventory_binding() {
    let receipt = TempReceipt::copy();
    let text = fs::read_to_string(&receipt.0).expect("read receipt");
    let mut lines: Vec<_> = text.lines().map(str::to_owned).collect();
    let first = lines
        .iter()
        .position(|line| line.starts_with("case\t"))
        .expect("first case");
    lines.swap(first, first + 1);
    fs::write(&receipt.0, format!("{}\n", lines.join("\n"))).expect("write swapped receipt");
    make_metadata_self_consistent(&receipt.0);

    let error =
        execution_receipt::check_report(&source_suite(), &receipt.0, actual_report()).unwrap_err();
    assert!(error.contains("order/identity mismatch"), "{error}");
}

#[test]
fn report_with_allowed_status_drift_is_rejected_per_identifier() {
    let mut improved = actual_report().clone();
    let case = improved
        .cases
        .iter_mut()
        .find(|case| case.id == "R2RMLTC0002f")
        .expect("documented deviation");
    assert_eq!(case.status, Status::Failed);
    case.status = Status::Passed;

    let error =
        execution_receipt::check_report(&source_suite(), &source_receipt(), &improved).unwrap_err();
    assert!(error.contains("R2RMLTC0002f"), "{error}");
    assert!(error.contains("expected r2rml failed"), "{error}");
    assert!(error.contains("observed r2rml passed"), "{error}");
}
