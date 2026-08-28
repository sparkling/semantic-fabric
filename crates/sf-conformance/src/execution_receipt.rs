//! Deterministic SQLite outcome baseline for the sealed W3C RDB2RDF suite.
//!
//! The receipt binds canonical inventory bytes to ordered
//! `(identifier, kind, status, outcome-code)` records. Human-readable reasons,
//! timestamps, hosts, runner source, dependency locks, and toolchains remain
//! outside this baseline and require a higher-level provenance envelope.

mod format;
#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::inventory::CaseKind;
use crate::manifest::Kind;
use crate::runner;
use crate::sealed_suite::{
    Backend, ClassifiedCaseResult, ClassifiedReport, OutcomeCode, SealedSuite,
};
use crate::Status;

/// One exact, stable case outcome. Free-form diagnostic wording is deliberately
/// excluded; the typed code preserves the adjudication cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptCase {
    pub identifier: String,
    pub kind: Kind,
    pub status: Status,
    pub outcome_code: OutcomeCode,
}

/// Parsed, canonical, and self-verified outcome receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionReceipt {
    inventory_sha256: String,
    outcomes_sha256: String,
    cases: Vec<ReceiptCase>,
}

impl ExecutionReceipt {
    pub fn inventory_sha256(&self) -> &str {
        &self.inventory_sha256
    }

    pub fn outcomes_sha256(&self) -> &str {
        &self.outcomes_sha256
    }

    pub fn cases(&self) -> &[ReceiptCase] {
        &self.cases
    }

    pub fn count(&self, status: Status) -> usize {
        self.cases
            .iter()
            .filter(|case| case.status == status)
            .count()
    }
}

/// Capture one immutable suite, execute exactly that snapshot, and render it.
pub fn generate(suite_root: &Path) -> Result<String, String> {
    let sealed = SealedSuite::load(suite_root)?;
    let report = runner::run_sealed_suite(&sealed)?;
    Ok(format::render(&from_report(&sealed, &report)?))
}

/// Validate the expected baseline and inventory before executing that same
/// captured snapshot. This path performs no writes.
pub fn check(suite_root: &Path, receipt_path: &Path) -> Result<ExecutionReceipt, String> {
    check_with_runner(suite_root, receipt_path, runner::run_sealed_suite)
}

fn check_with_runner<F>(
    suite_root: &Path,
    receipt_path: &Path,
    run: F,
) -> Result<ExecutionReceipt, String>
where
    F: FnOnce(&SealedSuite) -> Result<ClassifiedReport, String>,
{
    let expected = load_receipt(receipt_path)?;
    let sealed = SealedSuite::load(suite_root)?;
    validate_inventory_binding(&sealed, &expected)?;
    let observed = from_report(&sealed, &run(&sealed)?)?;
    compare(&expected, &observed)?;
    Ok(expected)
}

fn load_receipt(receipt_path: &Path) -> Result<ExecutionReceipt, String> {
    let text = read_bounded(receipt_path)?;
    let expected = format::parse(&text)?;
    if format::render(&expected) != text {
        return Err("execution receipt is valid but not in canonical generated form".to_owned());
    }
    Ok(expected)
}

fn read_bounded(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(format::MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    if bytes.len() as u64 > format::MAX_RECEIPT_BYTES {
        return Err(format!(
            "execution receipt exceeds {} bytes",
            format::MAX_RECEIPT_BYTES
        ));
    }
    String::from_utf8(bytes)
        .map_err(|error| format!("execution receipt {} is not UTF-8: {error}", path.display()))
}

fn from_report(
    sealed: &SealedSuite,
    report: &ClassifiedReport,
) -> Result<ExecutionReceipt, String> {
    sealed.validate_classified_report(Backend::Sqlite, report)?;
    let cases: Vec<_> = report.cases.iter().map(receipt_case).collect();
    Ok(ExecutionReceipt {
        inventory_sha256: sealed.inventory_sha256().to_owned(),
        outcomes_sha256: format::outcomes_digest(&cases),
        cases,
    })
}

fn receipt_case(case: &ClassifiedCaseResult) -> ReceiptCase {
    ReceiptCase {
        identifier: case.id.clone(),
        kind: case.kind,
        status: case.status,
        outcome_code: case.outcome_code,
    }
}

fn validate_inventory_binding(
    sealed: &SealedSuite,
    receipt: &ExecutionReceipt,
) -> Result<(), String> {
    if receipt.inventory_sha256 != sealed.inventory_sha256() {
        return Err(format!(
            "execution receipt inventory digest mismatch: recorded={}, actual={}",
            receipt.inventory_sha256,
            sealed.inventory_sha256()
        ));
    }
    if receipt.cases.len() != sealed.inventory().cases.len() {
        return Err(format!(
            "execution receipt has {} cases; sealed inventory requires {}",
            receipt.cases.len(),
            sealed.inventory().cases.len()
        ));
    }
    for (inventory_case, receipt_case) in sealed.inventory().cases.iter().zip(&receipt.cases) {
        if inventory_case.identifier != receipt_case.identifier {
            return Err(format!(
                "execution receipt order/identity mismatch: expected {}, observed {}",
                inventory_case.identifier, receipt_case.identifier
            ));
        }
        if inventory_kind(inventory_case.kind) != receipt_case.kind {
            return Err(format!(
                "execution receipt kind mismatch for {}",
                inventory_case.identifier
            ));
        }
    }
    let classified = ClassifiedReport {
        cases: receipt
            .cases
            .iter()
            .map(|case| ClassifiedCaseResult {
                id: case.identifier.clone(),
                kind: case.kind,
                status: case.status,
                outcome_code: case.outcome_code,
                reason: "tracked execution outcome".to_owned(),
            })
            .collect(),
    };
    sealed.validate_classified_report(Backend::Sqlite, &classified)
}

fn compare(expected: &ExecutionReceipt, observed: &ExecutionReceipt) -> Result<(), String> {
    if expected.inventory_sha256 != observed.inventory_sha256 {
        return Err("execution receipt inventory digest changed during replay".to_owned());
    }
    if expected.cases.len() != observed.cases.len() {
        return Err(format!(
            "execution receipt replay has {} cases; expected {}",
            observed.cases.len(),
            expected.cases.len()
        ));
    }
    for (expected_case, observed_case) in expected.cases.iter().zip(&observed.cases) {
        if expected_case != observed_case {
            return Err(format!(
                "execution receipt outcome mismatch for {}: expected {} {} {}, observed {} {} {}",
                expected_case.identifier,
                format::kind_name(expected_case.kind),
                format::status_name(expected_case.status),
                expected_case.outcome_code.name(),
                format::kind_name(observed_case.kind),
                format::status_name(observed_case.status),
                observed_case.outcome_code.name(),
            ));
        }
    }
    if expected.outcomes_sha256 != observed.outcomes_sha256 {
        return Err(format!(
            "execution receipt outcomes digest mismatch after replay: expected={}, actual={}",
            expected.outcomes_sha256, observed.outcomes_sha256
        ));
    }
    Ok(())
}

fn inventory_kind(kind: CaseKind) -> Kind {
    match kind {
        CaseKind::R2rml => Kind::R2rml,
        CaseKind::DirectMapping => Kind::DirectMapping,
    }
}
