use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use sf_conformance::inventory::{self, AllowedOutcome, CaseKind};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

fn source_suite() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf")
}

struct TempSuite(PathBuf);

impl TempSuite {
    fn copy() -> Self {
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "semantic-fabric-rdb2rdf-inventory-{}-{serial}",
            std::process::id()
        ));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale temporary suite");
        }
        copy_tree(&source_suite(), &path);
        Self(path)
    }

    fn inventory(&self) -> PathBuf {
        self.0.join("inventory.tsv")
    }
}

impl Drop for TempSuite {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove temporary suite");
    }
}

fn copy_tree(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("create temporary suite directory");
    for entry in fs::read_dir(source).expect("read suite directory") {
        let entry = entry.expect("read suite entry");
        let destination = target.join(entry.file_name());
        if entry.file_type().expect("read entry type").is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).expect("copy suite file");
        }
    }
}

fn replace_once(path: &Path, from: &str, to: &str) {
    let original = fs::read_to_string(path).expect("read mutation target");
    assert_eq!(original.matches(from).count(), 1, "unique mutation token");
    fs::write(path, original.replacen(from, to, 1)).expect("write mutation target");
}

fn check_temp(suite: &TempSuite) -> String {
    inventory::check(&suite.0, &suite.inventory()).unwrap_err()
}

#[test]
fn tracked_inventory_seals_exact_corpus_and_backend_policy() {
    let path = source_suite().join("inventory.tsv");
    let before = fs::read(&path).expect("read tracked inventory");
    let sealed = inventory::check(&source_suite(), &path).expect("inventory is current");
    let after = fs::read(&path).expect("read inventory after check");
    assert_eq!(before, after, "--check path must not write");
    assert_eq!(sealed.scenarios.len(), 26);
    assert_eq!(sealed.cases.len(), 87);
    assert_eq!(sealed.files.len(), 189);
    assert_eq!(sealed.suite_manifest.path, "manifest-evaluation.ttl");
    assert_eq!(
        sealed
            .cases
            .iter()
            .filter(|case| case.kind == CaseKind::R2rml)
            .count(),
        63
    );
    assert_eq!(
        sealed
            .cases
            .iter()
            .filter(|case| case.kind == CaseKind::DirectMapping)
            .count(),
        24
    );
    let sqlite = outcome_counts(&sealed.cases, |case| case.sqlite);
    let postgres = outcome_counts(&sealed.cases, |case| case.postgres);
    assert_eq!(sqlite, (81, 1, 5));
    assert_eq!(postgres, (80, 1, 6));
}

#[test]
fn deletion_fails_closed() {
    let suite = TempSuite::copy();
    fs::remove_file(suite.0.join("cases/D000-1table1column0rows/mapped.nq"))
        .expect("delete fixture");
    let error = check_temp(&suite);
    assert!(error.contains("case-tree files differ"), "{error}");
    assert!(error.contains("mapped.nq"), "{error}");
}

#[test]
fn extra_input_fails_closed() {
    let suite = TempSuite::copy();
    fs::write(
        suite.0.join("cases/D000-1table1column0rows/untracked.nq"),
        b"<extra> <extra> <extra> .\n",
    )
    .expect("add fixture");
    let error = check_temp(&suite);
    assert!(error.contains("case-tree files differ"), "{error}");
    assert!(error.contains("untracked.nq"), "{error}");
}

#[test]
fn byte_mutation_reports_digest_mismatch() {
    let suite = TempSuite::copy();
    let path = suite.0.join("cases/D000-1table1column0rows/mapped.nq");
    let mut output = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open fixture");
    output.write_all(b"\n").expect("mutate fixture bytes");
    let error = check_temp(&suite);
    assert!(error.contains("digest mismatch"), "{error}");
    assert!(error.contains("mapped.nq"), "{error}");
}

#[test]
fn suite_manifest_byte_mutation_reports_digest_mismatch() {
    let suite = TempSuite::copy();
    let mut manifest = fs::OpenOptions::new()
        .append(true)
        .open(suite.0.join("manifest-evaluation.ttl"))
        .expect("open suite manifest");
    manifest
        .write_all(b"\n")
        .expect("mutate suite manifest bytes");
    let error = check_temp(&suite);
    assert!(
        error.contains("digest mismatch for manifest-evaluation.ttl"),
        "{error}"
    );
}

#[test]
fn duplicate_manifest_identifier_fails_closed() {
    let suite = TempSuite::copy();
    let path = suite.0.join("cases/D001-1table1column1row/manifest.ttl");
    replace_once(&path, "R2RMLTC0001b", "R2RMLTC0001a");
    let error = check_temp(&suite);
    assert!(error.contains("duplicate case identifier"), "{error}");
}

#[test]
fn count_neutral_identity_substitution_fails_closed() {
    let suite = TempSuite::copy();
    let d15 = suite
        .0
        .join("cases/D015-1table3columns1composityeprimarykey3rows2languages/manifest.ttl");
    let d16 = suite
        .0
        .join("cases/D016-1table1primarykey10columns3rowsSQLdatatypes/manifest.ttl");
    replace_once(&d15, "DirectGraphTC0015", "DirectGraphTC0016");
    replace_once(&d16, "DirectGraphTC0016", "DirectGraphTC0015");
    let error = check_temp(&suite);
    assert!(error.contains("does not belong to scenario"), "{error}");
}

#[test]
fn count_neutral_status_substitution_fails_closed() {
    let suite = TempSuite::copy();
    let text = fs::read_to_string(suite.inventory()).expect("read inventory");
    let mut changed = Vec::new();
    for line in text.lines() {
        let mut fields: Vec<_> = line.split('\t').collect();
        if fields.first() == Some(&"case") && fields.get(1) == Some(&"DirectGraphTC0016") {
            fields[7] = "skip";
            fields[8] = "pass";
        } else if fields.first() == Some(&"case") && fields.get(1) == Some(&"DirectGraphTC0021") {
            fields[7] = "pass";
            fields[8] = "skip";
        }
        changed.push(fields.join("\t"));
    }
    fs::write(suite.inventory(), format!("{}\n", changed.join("\n")))
        .expect("write substituted inventory");
    let error = check_temp(&suite);
    assert!(error.contains("pinned backend policy"), "{error}");
}

#[test]
fn duplicate_inventory_identifier_fails_closed() {
    let suite = TempSuite::copy();
    replace_once(&suite.inventory(), "DirectGraphTC0002", "DirectGraphTC0001");
    let error = check_temp(&suite);
    assert!(error.contains("duplicate case identifier"), "{error}");
}

#[test]
fn malformed_inventory_fails_closed() {
    let suite = TempSuite::copy();
    fs::write(
        suite.inventory(),
        b"semantic-fabric-rdb2rdf-inventory-v1\nmalformed\n",
    )
    .expect("write malformed inventory");
    let error = check_temp(&suite);
    assert!(error.contains("malformed inventory record"), "{error}");
}

#[test]
fn empty_inventory_fails_closed() {
    let suite = TempSuite::copy();
    fs::write(suite.inventory(), b"").expect("empty inventory");
    assert_eq!(check_temp(&suite), "inventory is empty");
}

fn outcome_counts(
    cases: &[inventory::CaseEntry],
    select: impl Fn(&inventory::CaseEntry) -> AllowedOutcome,
) -> (usize, usize, usize) {
    let mut counts = (0, 0, 0);
    for case in cases {
        match select(case) {
            AllowedOutcome::Pass => counts.0 += 1,
            AllowedOutcome::Deviation => counts.1 += 1,
            AllowedOutcome::Skip => counts.2 += 1,
        }
    }
    counts
}
