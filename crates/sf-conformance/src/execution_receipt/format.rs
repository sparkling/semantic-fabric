use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use super::{ExecutionReceipt, ReceiptCase};
use crate::inventory::CASE_COUNT;
use crate::manifest::Kind;
use crate::sealed_suite::{Backend, OutcomeCode};
use crate::Status;

const HEADER: &str = "semantic-fabric-rdb2rdf-execution-receipt-v3";
const INVENTORY_PATH: &str = "inventory.tsv";
const HASH_ALGORITHM: &str = "sha256";
const METADATA_COUNT: usize = 13;
const MAX_LINE_BYTES: usize = 512;

pub(super) const MAX_RECEIPT_BYTES: u64 = 64 * 1024;

pub(super) fn render(receipt: &ExecutionReceipt) -> String {
    let mut output = String::new();
    writeln!(output, "{HEADER}").expect("String writes cannot fail");
    for (key, value) in metadata(receipt) {
        writeln!(output, "meta\t{key}\t{value}").expect("String writes cannot fail");
    }
    output.push_str(&outcome_records(&receipt.cases));
    output
}

pub(super) fn parse(input: &str) -> Result<ExecutionReceipt, String> {
    for (index, line) in input.lines().enumerate() {
        if line.len() > MAX_LINE_BYTES {
            return Err(format!(
                "execution receipt line {} exceeds {MAX_LINE_BYTES} bytes",
                index + 1
            ));
        }
    }
    let mut lines = input.lines();
    let Some(header) = lines.next() else {
        return Err("execution receipt is empty".to_owned());
    };
    if header != HEADER {
        return Err("invalid execution receipt header".to_owned());
    }

    let mut metadata = BTreeMap::new();
    let mut cases = Vec::with_capacity(CASE_COUNT);
    for (index, line) in lines.enumerate() {
        let number = index + 2;
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["meta", key, value] if cases.is_empty() => {
                if metadata.insert(*key, *value).is_some() {
                    return Err(format!("line {number}: duplicate metadata key {key}"));
                }
                if metadata.len() > METADATA_COUNT {
                    return Err(format!(
                        "line {number}: execution receipt has too many metadata records"
                    ));
                }
            }
            ["case", identifier, kind, status, outcome_code] => {
                if cases.len() == CASE_COUNT {
                    return Err(format!(
                        "line {number}: execution receipt exceeds {CASE_COUNT} cases"
                    ));
                }
                cases.push(ReceiptCase {
                    identifier: parse_identifier(identifier, number)?,
                    kind: parse_kind(kind, number)?,
                    status: parse_status(status, number)?,
                    outcome_code: parse_outcome_code(outcome_code, number)?,
                });
            }
            ["meta", ..] => {
                return Err(format!("line {number}: metadata follows case records"));
            }
            _ => return Err(format!("line {number}: malformed execution receipt record")),
        }
    }

    let backend = parse_backend(take(&mut metadata, "backend")?)?;
    let inventory_sha256 = take(&mut metadata, "inventory-sha256")?.to_owned();
    let recorded_outcomes_sha256 = take(&mut metadata, "outcomes-sha256")?.to_owned();
    expect(&mut metadata, "runner", runner_name(backend))?;
    expect(
        &mut metadata,
        "attestation-scope",
        "sealed-input-and-outcome-baseline-not-runner-toolchain-host-or-provider-provenance",
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
        backend,
        inventory_sha256,
        outcomes_sha256: recorded_outcomes_sha256,
        cases,
    })
}

pub(super) fn outcomes_digest(cases: &[ReceiptCase]) -> String {
    sha256(outcome_records(cases).as_bytes())
}

pub(super) fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::R2rml => "r2rml",
        Kind::DirectMapping => "direct-mapping",
    }
}

pub(super) fn status_name(status: Status) -> &'static str {
    match status {
        Status::Passed => "passed",
        Status::Failed => "failed",
        Status::Skipped => "skipped",
    }
}

fn metadata(receipt: &ExecutionReceipt) -> Vec<(&'static str, String)> {
    vec![
        ("backend", receipt.backend.name().to_owned()),
        ("runner", runner_name(receipt.backend).to_owned()),
        (
            "attestation-scope",
            "sealed-input-and-outcome-baseline-not-runner-toolchain-host-or-provider-provenance"
                .to_owned(),
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

fn parse_backend(value: &str) -> Result<Backend, String> {
    Backend::from_name(value).ok_or_else(|| format!("invalid execution receipt backend {value:?}"))
}

fn runner_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Sqlite => "sf-conformance::runner::run_sealed_suite",
        Backend::Postgres => "sf-conformance::pg::run_sealed_suite_required",
    }
}

fn outcome_records(cases: &[ReceiptCase]) -> String {
    let mut output = String::new();
    for case in cases {
        writeln!(
            output,
            "case\t{}\t{}\t{}\t{}",
            case.identifier,
            kind_name(case.kind),
            status_name(case.status),
            case.outcome_code.name()
        )
        .expect("String writes cannot fail");
    }
    output
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

fn parse_outcome_code(value: &str, line: usize) -> Result<OutcomeCode, String> {
    OutcomeCode::from_name(value)
        .ok_or_else(|| format!("line {line}: invalid outcome code {value:?}"))
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

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
