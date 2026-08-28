//! Deterministic seal for the locally vendored W3C RDB2RDF case tree.
//!
//! This is mapping-fixture evidence plus an allowed backend-outcome policy. It
//! is not a SPARQL query/protocol inventory and is not an execution receipt.

mod format;
mod policy;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::manifest::{self, Kind};
use policy::{allowed_outcomes, expected_error_ids, expected_ids, expected_scenarios, SCENARIOS};
use sha2::{Digest, Sha256};

pub const SCENARIO_COUNT: usize = 26;
pub const CASE_COUNT: usize = 87;
pub const R2RML_COUNT: usize = 63;
pub const DIRECT_COUNT: usize = 24;
pub const CASE_TREE_FILE_COUNT: usize = 189;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseKind {
    R2rml,
    DirectMapping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowedOutcome {
    Pass,
    Deviation,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseEntry {
    pub identifier: String,
    pub kind: CaseKind,
    pub scenario: String,
    pub mapping_document: Option<String>,
    pub output: Option<String>,
    pub expected_error: bool,
    pub sqlite: AllowedOutcome,
    pub postgres: AllowedOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    pub suite_manifest: FileEntry,
    pub scenarios: Vec<String>,
    pub cases: Vec<CaseEntry>,
    pub files: Vec<FileEntry>,
}

/// Build and validate the canonical inventory from the current local snapshot.
pub fn build(suite_root: &Path) -> Result<Inventory, String> {
    let suite_manifest_path = suite_root.join("manifest-evaluation.ttl");
    let suite_manifest_bytes = fs::read(&suite_manifest_path)
        .map_err(|e| format!("read {}: {e}", suite_manifest_path.display()))?;
    let suite_manifest_text = std::str::from_utf8(&suite_manifest_bytes)
        .map_err(|e| format!("{} is not UTF-8: {e}", suite_manifest_path.display()))?;
    let includes = manifest::parse_suite_manifest(suite_manifest_text)
        .map_err(|e| format!("{}: {e}", suite_manifest_path.display()))?;
    let expected_includes: Vec<_> = SCENARIOS
        .iter()
        .map(|scenario| format!("{scenario}/manifest.ttl"))
        .collect();
    if includes != expected_includes {
        return Err(format!(
            "suite manifest scenario list differs; expected={expected_includes:?}; actual={includes:?}"
        ));
    }
    let suite_manifest = FileEntry {
        path: "manifest-evaluation.ttl".to_owned(),
        sha256: format!("{:x}", Sha256::digest(&suite_manifest_bytes)),
    };

    let cases_root = suite_root.join("cases");
    let mut scenarios = directory_names(&cases_root, true)?;
    scenarios.sort();
    exact_set("scenarios", expected_scenarios(), string_set(&scenarios))?;

    let mut cases = Vec::new();
    let mut actual_paths = BTreeSet::new();
    for scenario in &scenarios {
        let scenario_dir = cases_root.join(scenario);
        for name in directory_names(&scenario_dir, false)? {
            actual_paths.insert(format!("cases/{scenario}/{name}"));
        }
        let manifest_path = scenario_dir.join("manifest.ttl");
        let text = fs::read_to_string(&manifest_path)
            .map_err(|e| format!("read {}: {e}", manifest_path.display()))?;
        let parsed = manifest::parse_strict(&text)
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
        for case in parsed {
            let kind = match case.kind {
                Kind::R2rml => CaseKind::R2rml,
                Kind::DirectMapping => CaseKind::DirectMapping,
            };
            let mapping_document = case
                .mapping_document
                .as_deref()
                .map(|name| case_path(scenario, name))
                .transpose()?;
            let output = case
                .output
                .as_deref()
                .map(|name| case_path(scenario, name))
                .transpose()?;
            let (sqlite, postgres) = allowed_outcomes(&case.identifier);
            cases.push(CaseEntry {
                identifier: case.identifier,
                kind,
                scenario: scenario.clone(),
                mapping_document,
                output,
                expected_error: !case.has_expected_output,
                sqlite,
                postgres,
            });
        }
    }
    cases.sort_by(|a, b| a.identifier.cmp(&b.identifier));

    let required_paths = required_paths(&scenarios, &cases)?;
    exact_set("case-tree files", required_paths, actual_paths.clone())?;
    let mut files = Vec::with_capacity(actual_paths.len());
    for path in actual_paths {
        let bytes = fs::read(suite_root.join(&path))
            .map_err(|e| format!("read {}: {e}", suite_root.join(&path).display()))?;
        files.push(FileEntry {
            path,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        });
    }

    let inventory = Inventory {
        suite_manifest,
        scenarios,
        cases,
        files,
    };
    validate(&inventory)?;
    Ok(inventory)
}

/// Render a freshly validated inventory without writing it.
pub fn generate(suite_root: &Path) -> Result<String, String> {
    Ok(format::render(&build(suite_root)?))
}

/// Mechanically replace an inventory after the local snapshot passes all fixed
/// identity and structure checks. Normal verification should use [`check`].
pub fn write_generated(suite_root: &Path, inventory_path: &Path) -> Result<(), String> {
    let rendered = generate(suite_root)?;
    fs::write(inventory_path, rendered)
        .map_err(|e| format!("write {}: {e}", inventory_path.display()))
}

/// Fail closed unless the tracked inventory is canonical and exactly matches
/// the current local case tree. This function performs no writes.
pub fn check(suite_root: &Path, inventory_path: &Path) -> Result<Inventory, String> {
    let text = fs::read_to_string(inventory_path)
        .map_err(|e| format!("read {}: {e}", inventory_path.display()))?;
    let sealed = format::parse(&text)?;
    if format::render(&sealed) != text {
        return Err("inventory is valid but not in canonical generated form".to_owned());
    }
    let current = build(suite_root)?;
    compare(&sealed, &current)?;
    Ok(current)
}

pub(crate) fn validate(inventory: &Inventory) -> Result<(), String> {
    if inventory.suite_manifest.path != "manifest-evaluation.ttl" {
        return Err(format!(
            "unexpected suite manifest path {}",
            inventory.suite_manifest.path
        ));
    }
    validate_digest(&inventory.suite_manifest)?;
    if inventory.scenarios.len() != SCENARIO_COUNT {
        return Err(format!(
            "expected {SCENARIO_COUNT} scenarios, found {}",
            inventory.scenarios.len()
        ));
    }
    exact_set(
        "scenarios",
        expected_scenarios(),
        string_set(&inventory.scenarios),
    )?;
    if inventory.cases.len() != CASE_COUNT {
        return Err(format!(
            "expected {CASE_COUNT} cases, found {}",
            inventory.cases.len()
        ));
    }

    let mut ids = BTreeSet::new();
    let mut errors = BTreeSet::new();
    let mut r2rml = 0;
    let mut direct = 0;
    for case in &inventory.cases {
        if !ids.insert(case.identifier.clone()) {
            return Err(format!("duplicate case identifier {}", case.identifier));
        }
        if case.expected_error {
            errors.insert(case.identifier.clone());
        }
        validate_case(case)?;
        match case.kind {
            CaseKind::R2rml => r2rml += 1,
            CaseKind::DirectMapping => direct += 1,
        }
    }
    if r2rml != R2RML_COUNT || direct != DIRECT_COUNT {
        return Err(format!(
            "expected {R2RML_COUNT} R2RML/{DIRECT_COUNT} Direct cases, found {r2rml}/{direct}"
        ));
    }
    exact_set("case identifiers", expected_ids(), ids)?;
    exact_set("expected-error cases", expected_error_ids(), errors)?;

    if inventory.files.len() != CASE_TREE_FILE_COUNT {
        return Err(format!(
            "expected {CASE_TREE_FILE_COUNT} case-tree files, found {}",
            inventory.files.len()
        ));
    }
    let mut file_paths = BTreeSet::new();
    for file in &inventory.files {
        validate_stored_path(&file.path)?;
        if !file_paths.insert(file.path.clone()) {
            return Err(format!("duplicate file path {}", file.path));
        }
        validate_digest(file)?;
    }
    exact_set(
        "inventory required paths",
        required_paths(&inventory.scenarios, &inventory.cases)?,
        file_paths,
    )
}

fn validate_digest(file: &FileEntry) -> Result<(), String> {
    if file.sha256.len() != 64
        || !file
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("invalid SHA-256 for {}", file.path));
    }
    Ok(())
}

fn validate_case(case: &CaseEntry) -> Result<(), String> {
    if !SCENARIOS.contains(&case.scenario.as_str()) {
        return Err(format!(
            "case {} names unknown scenario {}",
            case.identifier, case.scenario
        ));
    }
    let scenario_number = format!("0{}", &case.scenario[1..4]);
    let (suffix, expected_kind) = if let Some(suffix) = case.identifier.strip_prefix("R2RMLTC") {
        (suffix, CaseKind::R2rml)
    } else if let Some(suffix) = case.identifier.strip_prefix("DirectGraphTC") {
        (suffix, CaseKind::DirectMapping)
    } else {
        return Err(format!("malformed case identifier {}", case.identifier));
    };
    if case.kind != expected_kind {
        return Err(format!(
            "case {} kind does not match its identifier",
            case.identifier
        ));
    }
    let id_number = suffix
        .get(..4)
        .ok_or_else(|| format!("malformed case identifier {}", case.identifier))?;
    if id_number != scenario_number {
        return Err(format!(
            "case {} does not belong to scenario {}",
            case.identifier, case.scenario
        ));
    }
    match case.kind {
        CaseKind::R2rml if case.mapping_document.is_none() => {
            return Err(format!("case {} has no mapping path", case.identifier))
        }
        CaseKind::DirectMapping if case.mapping_document.is_some() => {
            return Err(format!(
                "direct case {} has a mapping path",
                case.identifier
            ))
        }
        _ => {}
    }
    match (case.expected_error, case.output.is_some()) {
        (true, true) => return Err(format!("error case {} has an output", case.identifier)),
        (false, false) => return Err(format!("positive case {} has no output", case.identifier)),
        _ => {}
    }
    for path in case.mapping_document.iter().chain(case.output.iter()) {
        validate_case_path(&case.scenario, path)?;
    }
    let expected = allowed_outcomes(&case.identifier);
    if (case.sqlite, case.postgres) != expected {
        return Err(format!(
            "allowed outcomes for {} do not match pinned backend policy",
            case.identifier
        ));
    }
    Ok(())
}

fn required_paths(scenarios: &[String], cases: &[CaseEntry]) -> Result<BTreeSet<String>, String> {
    let mut paths = BTreeSet::new();
    for scenario in scenarios {
        paths.insert(format!("cases/{scenario}/create.sql"));
        paths.insert(format!("cases/{scenario}/manifest.ttl"));
    }
    for case in cases {
        for path in case.mapping_document.iter().chain(case.output.iter()) {
            validate_case_path(&case.scenario, path)?;
            paths.insert(path.clone());
        }
    }
    Ok(paths)
}

fn case_path(scenario: &str, filename: &str) -> Result<String, String> {
    if filename.is_empty() || filename == "-" || filename.contains(['/', '\\', '\t', '\r', '\n']) {
        return Err(format!("unsafe case filename {filename:?}"));
    }
    Ok(format!("cases/{scenario}/{filename}"))
}

fn validate_case_path(scenario: &str, path: &str) -> Result<(), String> {
    let prefix = format!("cases/{scenario}/");
    let Some(filename) = path.strip_prefix(&prefix) else {
        return Err(format!("case path {path:?} is outside {prefix}"));
    };
    if case_path(scenario, filename)? != path {
        return Err(format!("non-canonical case path {path:?}"));
    }
    Ok(())
}

fn validate_stored_path(path: &str) -> Result<(), String> {
    let Some(rest) = path.strip_prefix("cases/") else {
        return Err(format!("file path {path:?} is outside cases/"));
    };
    let Some((scenario, filename)) = rest.split_once('/') else {
        return Err(format!("malformed case-tree path {path:?}"));
    };
    if !SCENARIOS.contains(&scenario) || case_path(scenario, filename)? != path {
        return Err(format!("non-canonical case-tree path {path:?}"));
    }
    Ok(())
}

fn directory_names(path: &Path, directories: bool) -> Result<Vec<String>, String> {
    let entries = fs::read_dir(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("read {} entry: {e}", path.display()))?;
        let kind = entry
            .file_type()
            .map_err(|e| format!("inspect {}: {e}", entry.path().display()))?;
        if kind.is_dir() != directories || (!directories && !kind.is_file()) {
            return Err(format!(
                "unexpected {} in case tree",
                entry.path().display()
            ));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| format!("non-UTF-8 name below {}", path.display()))?;
        if name.contains(['/', '\\', '\t', '\r', '\n']) {
            return Err(format!("unsafe case-tree name {name:?}"));
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}

fn string_set(values: &[String]) -> BTreeSet<String> {
    values.iter().cloned().collect()
}

fn exact_set(
    label: &str,
    expected: BTreeSet<String>,
    actual: BTreeSet<String>,
) -> Result<(), String> {
    if expected == actual {
        return Ok(());
    }
    let missing: Vec<_> = expected.difference(&actual).cloned().collect();
    let extra: Vec<_> = actual.difference(&expected).cloned().collect();
    Err(format!(
        "{label} differ; missing={missing:?}; extra={extra:?}"
    ))
}

fn compare(sealed: &Inventory, current: &Inventory) -> Result<(), String> {
    if sealed.suite_manifest != current.suite_manifest {
        return Err("digest mismatch for manifest-evaluation.ttl".to_owned());
    }
    if sealed.scenarios != current.scenarios {
        return Err("scenario inventory differs from current case tree".to_owned());
    }
    let sealed_cases: BTreeMap<_, _> = sealed
        .cases
        .iter()
        .map(|case| (&case.identifier, case))
        .collect();
    let current_cases: BTreeMap<_, _> = current
        .cases
        .iter()
        .map(|case| (&case.identifier, case))
        .collect();
    if sealed_cases != current_cases {
        return Err("case identity, paths, error flags, or backend outcomes differ".to_owned());
    }
    let sealed_files: BTreeMap<_, _> = sealed
        .files
        .iter()
        .map(|file| (&file.path, &file.sha256))
        .collect();
    let current_files: BTreeMap<_, _> = current
        .files
        .iter()
        .map(|file| (&file.path, &file.sha256))
        .collect();
    if sealed_files.keys().ne(current_files.keys()) {
        return Err("sealed and current file paths differ".to_owned());
    }
    for (path, digest) in sealed_files {
        if current_files[path] != digest {
            return Err(format!("digest mismatch for {path}"));
        }
    }
    Ok(())
}
