//! Deterministic SQLite execution receipt for the sealed W3C RDB2RDF suite.
//!
//! The receipt deliberately excludes timestamps, host details, reasons, and
//! other unstable text. It binds the exact canonical inventory bytes to the
//! ordered `(identifier, kind, status)` result of every case. It is an outcome
//! baseline, not standalone provenance: it does not attest runner source,
//! dependency locks, compiler, or host. Normal CI uses [`check`] without writes;
//! the companion CLI owns explicit, path-constrained regeneration.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::inventory::CaseKind;
use crate::sealed_suite::{Backend, SealedSuite};
use crate::{run_suite, CaseResult, Kind, Report, Status};

const HEADER: &str = "semantic-fabric-rdb2rdf-execution-receipt-v1";
const BACKEND: &str = "sqlite";
const RUNNER: &str = "sf-conformance::run_suite";
const INVENTORY_PATH: &str = "inventory.tsv";
const HASH_ALGORITHM: &str = "sha256";

/// One deterministic case outcome. Human-readable reasons remain in EARL and
/// console reports because provider/library wording is not a stable baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptCase {
    pub identifier: String,
    pub kind: Kind,
    pub status: Status,
}

/// Parsed and self-verified execution receipt.
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
        status_count(&self.cases, status)
    }
}

/// Execute the sealed SQLite suite and return its canonical receipt bytes.
pub fn generate(suite_root: &Path) -> Result<String, String> {
    let report = run_suite(suite_root)?;
    generate_from_report(suite_root, &report)
}

/// Build canonical receipt bytes from an already executed report.
///
/// This is public so mutation tests and higher-level harnesses can prove the
/// receipt comparison independently from database execution.
pub fn generate_from_report(suite_root: &Path, report: &Report) -> Result<String, String> {
    let sealed = SealedSuite::load(suite_root)?;
    let receipt = from_report(suite_root, &sealed, report)?;
    Ok(render(&receipt))
}

/// Re-execute SQLite and compare the result to a tracked receipt without
/// writing the receipt, inventory, EARL, or any suite file.
pub fn check(suite_root: &Path, receipt_path: &Path) -> Result<ExecutionReceipt, String> {
    let (sealed, expected) = load_expected(suite_root, receipt_path)?;
    let report = run_suite(suite_root)?;
    let observed = from_report(suite_root, &sealed, &report)?;
    compare(&expected, &observed)?;
    Ok(expected)
}

/// Compare an already executed report to a tracked receipt without writes.
pub fn check_report(
    suite_root: &Path,
    receipt_path: &Path,
    report: &Report,
) -> Result<ExecutionReceipt, String> {
    let (sealed, expected) = load_expected(suite_root, receipt_path)?;
    let observed = from_report(suite_root, &sealed, report)?;
    compare(&expected, &observed)?;
    Ok(expected)
}

fn load_expected(
    suite_root: &Path,
    receipt_path: &Path,
) -> Result<(SealedSuite, ExecutionReceipt), String> {
    let text = fs::read_to_string(receipt_path)
        .map_err(|error| format!("read {}: {error}", receipt_path.display()))?;
    let expected = parse(&text)?;
    if render(&expected) != text {
        return Err("execution receipt is valid but not in canonical generated form".to_owned());
    }

    let sealed = SealedSuite::load(suite_root)?;
    validate_inventory_binding(suite_root, &sealed, &expected)?;
    Ok((sealed, expected))
}

fn from_report(
    suite_root: &Path,
    sealed: &SealedSuite,
    report: &Report,
) -> Result<ExecutionReceipt, String> {
    sealed.validate_report(Backend::Sqlite, report)?;
    let inventory_sha256 = inventory_digest(suite_root)?;
    let cases: Vec<_> = report.cases.iter().map(receipt_case).collect();
    let outcomes_sha256 = outcomes_digest(&cases);
    Ok(ExecutionReceipt {
        inventory_sha256,
        outcomes_sha256,
        cases,
    })
}

fn receipt_case(case: &CaseResult) -> ReceiptCase {
    ReceiptCase {
        identifier: case.id.clone(),
        kind: case.kind,
        status: case.status,
    }
}

fn inventory_digest(suite_root: &Path) -> Result<String, String> {
    let path = suite_root.join(INVENTORY_PATH);
    let bytes = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    Ok(sha256(&bytes))
}

fn render(receipt: &ExecutionReceipt) -> String {
    let mut output = String::new();
    writeln!(output, "{HEADER}").expect("String writes cannot fail");
    for (key, value) in metadata(receipt) {
        writeln!(output, "meta\t{key}\t{value}").expect("String writes cannot fail");
    }
    output.push_str(&outcome_records(&receipt.cases));
    output
}

fn metadata(receipt: &ExecutionReceipt) -> Vec<(&'static str, String)> {
    vec![
        ("backend", BACKEND.to_owned()),
        ("runner", RUNNER.to_owned()),
        (
            "attestation-scope",
            "sealed-input-and-outcome-baseline-not-runner-provenance".to_owned(),
        ),
        ("inventory-path", INVENTORY_PATH.to_owned()),
        ("inventory-sha256", receipt.inventory_sha256.clone()),
        ("hash-algorithm", HASH_ALGORITHM.to_owned()),
        ("case-count", receipt.cases.len().to_string()),
        (
            "r2rml-count",
            kind_count(&receipt.cases, Kind::R2rml).to_string(),
        ),
        (
            "direct-mapping-count",
            kind_count(&receipt.cases, Kind::DirectMapping).to_string(),
        ),
        ("passed-count", receipt.count(Status::Passed).to_string()),
        ("failed-count", receipt.count(Status::Failed).to_string()),
        ("skipped-count", receipt.count(Status::Skipped).to_string()),
        ("outcomes-sha256", receipt.outcomes_sha256.clone()),
    ]
}

fn parse(input: &str) -> Result<ExecutionReceipt, String> {
    let mut lines = input.lines().enumerate();
    let Some((_, header)) = lines.next() else {
        return Err("execution receipt is empty".to_owned());
    };
    if header != HEADER {
        return Err(format!("invalid execution receipt header {header:?}"));
    }

    let mut metadata = BTreeMap::new();
    let mut cases = Vec::new();
    for (index, line) in lines {
        let number = index + 1;
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["meta", key, value] if cases.is_empty() => {
                if metadata.insert(*key, *value).is_some() {
                    return Err(format!("line {number}: duplicate metadata key {key}"));
                }
            }
            ["case", identifier, kind, status] => cases.push(ReceiptCase {
                identifier: parse_identifier(identifier, number)?,
                kind: parse_kind(kind, number)?,
                status: parse_status(status, number)?,
            }),
            ["meta", ..] => {
                return Err(format!("line {number}: metadata follows case records"));
            }
            _ => return Err(format!("line {number}: malformed execution receipt record")),
        }
    }

    let inventory_sha256 = take(&mut metadata, "inventory-sha256")?.to_owned();
    let recorded_outcomes_sha256 = take(&mut metadata, "outcomes-sha256")?.to_owned();
    expect(&mut metadata, "backend", BACKEND)?;
    expect(&mut metadata, "runner", RUNNER)?;
    expect(
        &mut metadata,
        "attestation-scope",
        "sealed-input-and-outcome-baseline-not-runner-provenance",
    )?;
    expect(&mut metadata, "inventory-path", INVENTORY_PATH)?;
    expect(&mut metadata, "hash-algorithm", HASH_ALGORITHM)?;
    expect_count(&mut metadata, "case-count", cases.len())?;
    expect_count(
        &mut metadata,
        "r2rml-count",
        kind_count(&cases, Kind::R2rml),
    )?;
    expect_count(
        &mut metadata,
        "direct-mapping-count",
        kind_count(&cases, Kind::DirectMapping),
    )?;
    expect_count(
        &mut metadata,
        "passed-count",
        status_count(&cases, Status::Passed),
    )?;
    expect_count(
        &mut metadata,
        "failed-count",
        status_count(&cases, Status::Failed),
    )?;
    expect_count(
        &mut metadata,
        "skipped-count",
        status_count(&cases, Status::Skipped),
    )?;
    if let Some(key) = metadata.keys().next() {
        return Err(format!("unknown execution receipt metadata key {key}"));
    }
    validate_sha256("inventory-sha256", &inventory_sha256)?;
    validate_sha256("outcomes-sha256", &recorded_outcomes_sha256)?;
    validate_case_shape(&cases)?;

    let actual_outcomes_sha256 = outcomes_digest(&cases);
    if recorded_outcomes_sha256 != actual_outcomes_sha256 {
        return Err(format!(
            "execution receipt outcomes digest mismatch: recorded={recorded_outcomes_sha256}, actual={actual_outcomes_sha256}"
        ));
    }
    Ok(ExecutionReceipt {
        inventory_sha256,
        outcomes_sha256: recorded_outcomes_sha256,
        cases,
    })
}

fn validate_inventory_binding(
    suite_root: &Path,
    sealed: &SealedSuite,
    receipt: &ExecutionReceipt,
) -> Result<(), String> {
    let observed_inventory_sha256 = inventory_digest(suite_root)?;
    if receipt.inventory_sha256 != observed_inventory_sha256 {
        return Err(format!(
            "execution receipt inventory digest mismatch: recorded={}, actual={observed_inventory_sha256}",
            receipt.inventory_sha256
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
    let receipt_report = Report {
        cases: receipt
            .cases
            .iter()
            .map(|case| CaseResult {
                id: case.identifier.clone(),
                kind: case.kind,
                status: case.status,
                reason: "execution receipt".to_owned(),
            })
            .collect(),
    };
    sealed.validate_report(Backend::Sqlite, &receipt_report)
}

fn compare(expected: &ExecutionReceipt, observed: &ExecutionReceipt) -> Result<(), String> {
    if expected.inventory_sha256 != observed.inventory_sha256 {
        return Err("execution receipt inventory digest changed during replay".to_owned());
    }
    for (expected_case, observed_case) in expected.cases.iter().zip(&observed.cases) {
        if expected_case != observed_case {
            return Err(format!(
                "execution receipt outcome mismatch for {}: expected {} {}, observed {} {}",
                expected_case.identifier,
                kind_name(expected_case.kind),
                status_name(expected_case.status),
                kind_name(observed_case.kind),
                status_name(observed_case.status),
            ));
        }
    }
    if expected.cases.len() != observed.cases.len() {
        return Err(format!(
            "execution receipt replay has {} cases; expected {}",
            observed.cases.len(),
            expected.cases.len()
        ));
    }
    if expected.outcomes_sha256 != observed.outcomes_sha256 {
        return Err(format!(
            "execution receipt outcomes digest mismatch after replay: expected={}, actual={}",
            expected.outcomes_sha256, observed.outcomes_sha256
        ));
    }
    Ok(())
}

fn outcome_records(cases: &[ReceiptCase]) -> String {
    let mut output = String::new();
    for case in cases {
        writeln!(
            output,
            "case\t{}\t{}\t{}",
            case.identifier,
            kind_name(case.kind),
            status_name(case.status)
        )
        .expect("String writes cannot fail");
    }
    output
}

fn outcomes_digest(cases: &[ReceiptCase]) -> String {
    sha256(outcome_records(cases).as_bytes())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_case_shape(cases: &[ReceiptCase]) -> Result<(), String> {
    let mut identifiers = BTreeSet::new();
    for case in cases {
        if !identifiers.insert(&case.identifier) {
            return Err(format!(
                "duplicate execution receipt identifier {}",
                case.identifier
            ));
        }
    }
    Ok(())
}

fn parse_identifier(value: &str, line: usize) -> Result<String, String> {
    if value.is_empty() || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(format!("line {line}: invalid case identifier {value:?}"));
    }
    Ok(value.to_owned())
}

fn parse_kind(value: &str, line: usize) -> Result<Kind, String> {
    match value {
        "r2rml" => Ok(Kind::R2rml),
        "direct-mapping" => Ok(Kind::DirectMapping),
        _ => Err(format!("line {line}: invalid case kind {value:?}")),
    }
}

fn parse_status(value: &str, line: usize) -> Result<Status, String> {
    match value {
        "passed" => Ok(Status::Passed),
        "failed" => Ok(Status::Failed),
        "skipped" => Ok(Status::Skipped),
        _ => Err(format!("line {line}: invalid case status {value:?}")),
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::R2rml => "r2rml",
        Kind::DirectMapping => "direct-mapping",
    }
}

fn inventory_kind(kind: CaseKind) -> Kind {
    match kind {
        CaseKind::R2rml => Kind::R2rml,
        CaseKind::DirectMapping => Kind::DirectMapping,
    }
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Passed => "passed",
        Status::Failed => "failed",
        Status::Skipped => "skipped",
    }
}

fn kind_count(cases: &[ReceiptCase], kind: Kind) -> usize {
    cases.iter().filter(|case| case.kind == kind).count()
}

fn status_count(cases: &[ReceiptCase], status: Status) -> usize {
    cases.iter().filter(|case| case.status == status).count()
}

fn take<'a>(metadata: &mut BTreeMap<&'a str, &'a str>, key: &str) -> Result<&'a str, String> {
    metadata
        .remove(key)
        .ok_or_else(|| format!("missing execution receipt metadata key {key}"))
}

fn expect<'a>(
    metadata: &mut BTreeMap<&'a str, &'a str>,
    key: &str,
    expected: &str,
) -> Result<(), String> {
    let actual = take(metadata, key)?;
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "execution receipt metadata {key} is {actual:?}, expected {expected:?}"
        ))
    }
}

fn expect_count<'a>(
    metadata: &mut BTreeMap<&'a str, &'a str>,
    key: &str,
    expected: usize,
) -> Result<(), String> {
    let actual = take(metadata, key)?;
    if actual == expected.to_string() {
        Ok(())
    } else {
        Err(format!(
            "execution receipt metadata {key} is {actual:?}, expected {expected}"
        ))
    }
}

fn validate_sha256(label: &str, value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!("invalid SHA-256 in execution receipt {label}"))
    }
}
