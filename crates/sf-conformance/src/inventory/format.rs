use std::collections::BTreeMap;
use std::fmt::Write;

use super::{AllowedOutcome, CaseEntry, CaseKind, FileEntry, Inventory};

const HEADER: &str = "semantic-fabric-rdb2rdf-inventory-v1";
const METADATA: &[(&str, &str)] = &[
    ("snapshot-provenance", "local-vendored-copy"),
    (
        "snapshot-source",
        "https://github.com/johardi/jr2rml-test-suite res mirror; recorded by local README",
    ),
    ("upstream-revision", "not-recorded-in-local-snapshot"),
    (
        "evidence-scope",
        "rdb2rdf-mapping-fixtures-and-allowed-backend-outcomes",
    ),
    ("outcome-kind", "allowed-policy-not-execution-receipt"),
    ("hash-algorithm", "sha256"),
    ("scenario-count", "26"),
    ("case-count", "87"),
    ("r2rml-count", "63"),
    ("direct-mapping-count", "24"),
    ("case-tree-file-count", "189"),
    ("suite-manifest-count", "1"),
];

pub(super) fn render(inventory: &Inventory) -> String {
    let mut output = String::new();
    writeln!(output, "{HEADER}").expect("String writes cannot fail");
    for (key, value) in METADATA {
        writeln!(output, "meta\t{key}\t{value}").expect("String writes cannot fail");
    }
    writeln!(
        output,
        "suite-manifest\t{}\t{}",
        inventory.suite_manifest.path, inventory.suite_manifest.sha256
    )
    .expect("String writes cannot fail");
    for scenario in &inventory.scenarios {
        writeln!(output, "scenario\t{scenario}").expect("String writes cannot fail");
    }
    for case in &inventory.cases {
        writeln!(
            output,
            "case\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            case.identifier,
            kind_name(case.kind),
            case.scenario,
            optional_name(case.mapping_document.as_deref()),
            optional_name(case.output.as_deref()),
            case.expected_error,
            outcome_name(case.sqlite),
            outcome_name(case.postgres),
        )
        .expect("String writes cannot fail");
    }
    for file in &inventory.files {
        writeln!(output, "file\t{}\t{}", file.path, file.sha256)
            .expect("String writes cannot fail");
    }
    output
}

pub(super) fn parse(input: &str) -> Result<Inventory, String> {
    let mut lines = input.lines().enumerate();
    let Some((_, header)) = lines.next() else {
        return Err("inventory is empty".to_owned());
    };
    if header != HEADER {
        return Err(format!("invalid inventory header {header:?}"));
    }

    let mut metadata = BTreeMap::new();
    let mut suite_manifest = None;
    let mut scenarios = Vec::new();
    let mut cases = Vec::new();
    let mut files = Vec::new();
    for (index, line) in lines {
        let number = index + 1;
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["meta", key, value] => {
                if metadata
                    .insert((*key).to_owned(), (*value).to_owned())
                    .is_some()
                {
                    return Err(format!("line {number}: duplicate metadata key {key}"));
                }
            }
            ["scenario", scenario] => scenarios.push((*scenario).to_owned()),
            ["suite-manifest", path, digest] => {
                if suite_manifest
                    .replace(FileEntry {
                        path: (*path).to_owned(),
                        sha256: (*digest).to_owned(),
                    })
                    .is_some()
                {
                    return Err(format!("line {number}: duplicate suite manifest record"));
                }
            }
            ["case", id, kind, scenario, mapping, output, error, sqlite, postgres] => {
                cases.push(CaseEntry {
                    identifier: (*id).to_owned(),
                    kind: parse_kind(kind, number)?,
                    scenario: (*scenario).to_owned(),
                    mapping_document: parse_optional(mapping),
                    output: parse_optional(output),
                    expected_error: parse_bool(error, number)?,
                    sqlite: parse_outcome(sqlite, number)?,
                    postgres: parse_outcome(postgres, number)?,
                });
            }
            ["file", path, digest] => files.push(FileEntry {
                path: (*path).to_owned(),
                sha256: (*digest).to_owned(),
            }),
            _ => return Err(format!("line {number}: malformed inventory record")),
        }
    }
    validate_metadata(metadata)?;
    let suite_manifest =
        suite_manifest.ok_or_else(|| "missing suite manifest record".to_owned())?;
    let inventory = Inventory {
        suite_manifest,
        scenarios,
        cases,
        files,
    };
    super::validate(&inventory)?;
    Ok(inventory)
}

fn validate_metadata(mut actual: BTreeMap<String, String>) -> Result<(), String> {
    for (key, expected) in METADATA {
        match actual.remove(*key) {
            Some(value) if value == *expected => {}
            Some(value) => {
                return Err(format!(
                    "metadata {key} is {value:?}, expected {expected:?}"
                ))
            }
            None => return Err(format!("missing metadata key {key}")),
        }
    }
    if let Some(key) = actual.keys().next() {
        return Err(format!("unknown metadata key {key}"));
    }
    Ok(())
}

fn kind_name(kind: CaseKind) -> &'static str {
    match kind {
        CaseKind::R2rml => "r2rml",
        CaseKind::DirectMapping => "direct-mapping",
    }
}

fn parse_kind(value: &str, line: usize) -> Result<CaseKind, String> {
    match value {
        "r2rml" => Ok(CaseKind::R2rml),
        "direct-mapping" => Ok(CaseKind::DirectMapping),
        _ => Err(format!("line {line}: invalid case kind {value:?}")),
    }
}

fn outcome_name(outcome: AllowedOutcome) -> &'static str {
    match outcome {
        AllowedOutcome::Pass => "pass",
        AllowedOutcome::Deviation => "deviation",
        AllowedOutcome::Skip => "skip",
    }
}

fn parse_outcome(value: &str, line: usize) -> Result<AllowedOutcome, String> {
    match value {
        "pass" => Ok(AllowedOutcome::Pass),
        "deviation" => Ok(AllowedOutcome::Deviation),
        "skip" => Ok(AllowedOutcome::Skip),
        _ => Err(format!("line {line}: invalid allowed outcome {value:?}")),
    }
}

fn optional_name(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

fn parse_optional(value: &str) -> Option<String> {
    (value != "-").then(|| value.to_owned())
}

fn parse_bool(value: &str, line: usize) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("line {line}: invalid boolean {value:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_inventory() {
        assert_eq!(parse("").unwrap_err(), "inventory is empty");
    }

    #[test]
    fn rejects_malformed_record() {
        let error = parse(&format!("{HEADER}\nwat\tnot-a-record\n")).unwrap_err();
        assert!(error.contains("malformed inventory record"), "{error}");
    }
}
