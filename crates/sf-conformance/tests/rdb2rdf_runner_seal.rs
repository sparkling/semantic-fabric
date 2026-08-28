use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use sf_conformance::inventory::{AllowedOutcome, CaseKind};
use sf_conformance::pg::{self, LiveMode};
use sf_conformance::sealed_suite::{Backend, SealedSuite};
use sf_conformance::{CaseResult, Kind, Report, Status};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

fn source_suite() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf")
}

struct TempSuite(PathBuf);

impl TempSuite {
    fn copy() -> Self {
        let path = loop {
            let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let candidate = std::env::temp_dir().join(format!(
                "semantic-fabric-rdb2rdf-runner-{}-{serial}",
                std::process::id()
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create isolated suite directory: {error}"),
            }
        };
        copy_tree_contents(&source_suite(), &path);
        Self(path)
    }
}

fn copy_tree_contents(source: &Path, target: &Path) {
    for entry in fs::read_dir(source).expect("read suite directory") {
        let entry = entry.expect("read suite entry");
        let destination = target.join(entry.file_name());
        let kind = entry.file_type().expect("read entry type");
        if kind.is_dir() {
            fs::create_dir(&destination).expect("create suite subdirectory");
            copy_tree_contents(&entry.path(), &destination);
        } else if kind.is_file() {
            fs::copy(entry.path(), destination).expect("copy suite file");
        } else {
            panic!(
                "suite copy refuses non-file entry {}",
                entry.path().display()
            );
        }
    }
}

impl Drop for TempSuite {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove suite copy");
    }
}

fn manifest_kind(kind: CaseKind) -> Kind {
    match kind {
        CaseKind::R2rml => Kind::R2rml,
        CaseKind::DirectMapping => Kind::DirectMapping,
    }
}

fn policy_report(sealed: &SealedSuite, backend: Backend) -> Report {
    let cases = sealed
        .inventory()
        .cases
        .iter()
        .map(|case| {
            let policy = match backend {
                Backend::Sqlite => case.sqlite,
                Backend::Postgres => case.postgres,
            };
            let status = match policy {
                AllowedOutcome::Pass => Status::Passed,
                AllowedOutcome::Deviation => Status::Failed,
                AllowedOutcome::Skip => Status::Skipped,
            };
            CaseResult {
                id: case.identifier.clone(),
                kind: manifest_kind(case.kind),
                status,
                reason: "policy fixture".to_owned(),
            }
        })
        .collect();
    Report { cases }
}

#[test]
fn sealed_cases_materialize_in_canonical_inventory_order() {
    let sealed = SealedSuite::load(&source_suite()).expect("load sealed suite");
    let expected: Vec<_> = sealed
        .inventory()
        .cases
        .iter()
        .map(|case| case.identifier.as_str())
        .collect();
    let actual: Vec<_> = sealed
        .cases()
        .iter()
        .map(|case| case.case.identifier.as_str())
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 87);
}

#[test]
fn count_neutral_outcome_swap_fails_per_id_policy() {
    let sealed = SealedSuite::load(&source_suite()).expect("load sealed suite");
    let mut report = policy_report(&sealed, Backend::Sqlite);
    let pass = report
        .cases
        .iter()
        .position(|case| case.id == "DirectGraphTC0001")
        .unwrap();
    let skip = report
        .cases
        .iter()
        .position(|case| case.id == "DirectGraphTC0021")
        .unwrap();
    report.cases[pass].status = Status::Skipped;
    report.cases[skip].status = Status::Passed;
    let error = sealed
        .validate_report(Backend::Sqlite, &report)
        .unwrap_err();
    assert!(error.contains("DirectGraphTC0001"), "{error}");
    assert!(error.contains("outcome mismatch"), "{error}");
}

#[test]
fn report_identity_order_swap_fails_closed() {
    let sealed = SealedSuite::load(&source_suite()).expect("load sealed suite");
    let mut report = policy_report(&sealed, Backend::Postgres);
    report.cases.swap(0, 1);
    let error = sealed
        .validate_report(Backend::Postgres, &report)
        .unwrap_err();
    assert!(error.contains("order/identity mismatch"), "{error}");
}

#[test]
fn documented_nonpassing_cases_may_improve_to_pass() {
    let sealed = SealedSuite::load(&source_suite()).expect("load sealed suite");
    for backend in [Backend::Sqlite, Backend::Postgres] {
        let mut report = policy_report(&sealed, backend);
        for case in &mut report.cases {
            case.status = Status::Passed;
        }
        sealed
            .validate_report(backend, &report)
            .expect("passing is always allowed");
    }
}

#[test]
fn sqlite_entrypoint_rejects_drift_before_execution() {
    let suite = TempSuite::copy();
    let path = suite.0.join("cases/D000-1table1column0rows/manifest.ttl");
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    let error = sf_conformance::run_suite(&suite.0).unwrap_err();
    assert!(error.contains("digest mismatch"), "{error}");
}

#[test]
fn sealed_sqlite_execution_uses_captured_bytes_after_source_mutation() {
    let suite = TempSuite::copy();
    let sealed = SealedSuite::load(&suite.0).expect("capture sealed suite");
    fs::write(
        suite.0.join("cases/D000-1table1column0rows/create.sql"),
        "this is not valid SQLite",
    )
    .expect("mutate source after the sealing barrier");

    let report = sf_conformance::runner::run_sealed_suite(&sealed)
        .expect("execution uses the immutable captured snapshot");
    assert_eq!(report.cases.len(), 87);
    let mutated_source_case = report
        .cases
        .iter()
        .find(|case| case.id == "DirectGraphTC0000")
        .expect("mutated source case remains in the classified report");
    assert_eq!(mutated_source_case.status, Status::Passed);
}

#[test]
fn postgres_entrypoint_rejects_bad_inventory_before_provider_probe() {
    let suite = TempSuite::copy();
    fs::write(suite.0.join("inventory.tsv"), b"").expect("empty inventory");
    let error = pg::run(&suite.0, LiveMode::LocalOptional).unwrap_err();
    assert_eq!(error, "inventory is empty");
}
