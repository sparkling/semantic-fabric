//! Fail-closed, immutable execution view of the canonical W3C RDB2RDF inventory.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::inventory::{self, AllowedOutcome, CaseEntry, CaseKind, FileEntry, Inventory};
use crate::manifest::{Case, Kind};
use crate::{CaseResult, Report, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Sqlite,
    Postgres,
}

/// Stable adjudication cause. Free-form reason text remains diagnostic only;
/// receipts and the non-passing policy bind this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeCode {
    GraphMatched,
    DatasetMatched,
    GraphMismatch,
    DatasetMismatch,
    UnexpectedOutput,
    FixtureLoadError,
    IntrospectionError,
    MappingError,
    SourceValidationError,
    TranslationUnsupported,
    TranslationError,
    ExecutionUnsupported,
    ExecutionError,
    QuadDumpUnsupported,
    QuadDumpError,
    DirectMappingError,
}

impl OutcomeCode {
    pub fn name(self) -> &'static str {
        match self {
            Self::GraphMatched => "graph-matched",
            Self::DatasetMatched => "dataset-matched",
            Self::GraphMismatch => "graph-mismatch",
            Self::DatasetMismatch => "dataset-mismatch",
            Self::UnexpectedOutput => "unexpected-output",
            Self::FixtureLoadError => "fixture-load-error",
            Self::IntrospectionError => "introspection-error",
            Self::MappingError => "mapping-error",
            Self::SourceValidationError => "source-validation-error",
            Self::TranslationUnsupported => "translation-unsupported",
            Self::TranslationError => "translation-error",
            Self::ExecutionUnsupported => "execution-unsupported",
            Self::ExecutionError => "execution-error",
            Self::QuadDumpUnsupported => "quad-dump-unsupported",
            Self::QuadDumpError => "quad-dump-error",
            Self::DirectMappingError => "direct-mapping-error",
        }
    }

    pub fn from_name(value: &str) -> Option<Self> {
        Some(match value {
            "graph-matched" => Self::GraphMatched,
            "dataset-matched" => Self::DatasetMatched,
            "graph-mismatch" => Self::GraphMismatch,
            "dataset-mismatch" => Self::DatasetMismatch,
            "unexpected-output" => Self::UnexpectedOutput,
            "fixture-load-error" => Self::FixtureLoadError,
            "introspection-error" => Self::IntrospectionError,
            "mapping-error" => Self::MappingError,
            "source-validation-error" => Self::SourceValidationError,
            "translation-unsupported" => Self::TranslationUnsupported,
            "translation-error" => Self::TranslationError,
            "execution-unsupported" => Self::ExecutionUnsupported,
            "execution-error" => Self::ExecutionError,
            "quad-dump-unsupported" => Self::QuadDumpUnsupported,
            "quad-dump-error" => Self::QuadDumpError,
            "direct-mapping-error" => Self::DirectMappingError,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ClassifiedCaseResult {
    pub id: String,
    pub kind: Kind,
    pub status: Status,
    pub outcome_code: OutcomeCode,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct ClassifiedReport {
    pub cases: Vec<ClassifiedCaseResult>,
}

#[derive(Debug, Clone)]
pub struct SealedCase {
    /// Retained for legacy runners. Receipt execution uses only the captured
    /// text fields and performs no path rereads after `SealedSuite::load`.
    pub directory: PathBuf,
    pub case: Case,
    create_sql: String,
    mapping_text: Option<String>,
    output_text: Option<String>,
}

impl SealedCase {
    pub(crate) fn create_sql(&self) -> &str {
        &self.create_sql
    }

    pub(crate) fn mapping_text(&self) -> Option<&str> {
        self.mapping_text.as_deref()
    }

    pub(crate) fn output_text(&self) -> Option<&str> {
        self.output_text.as_deref()
    }
}

#[derive(Debug, Clone)]
pub struct SealedSuite {
    inventory: Inventory,
    inventory_sha256: String,
    cases: Vec<SealedCase>,
}

impl SealedSuite {
    /// Validate the tree, capture every inventory file by value, and verify the
    /// captured bytes before exposing an execution case.
    pub fn load(suite_root: &Path) -> Result<Self, String> {
        let inventory_path = suite_root.join("inventory.tsv");
        let inventory = inventory::check(suite_root, &inventory_path)?;
        let inventory_text = fs::read_to_string(&inventory_path)
            .map_err(|error| format!("read {}: {error}", inventory_path.display()))?;
        if inventory::canonical_text(&inventory) != inventory_text {
            return Err("inventory changed while creating the sealed snapshot".to_owned());
        }

        capture_text(suite_root, &inventory.suite_manifest)?;
        let files = capture_files(suite_root, &inventory.files)?;
        let cases = inventory
            .cases
            .iter()
            .map(|entry| materialize_case(suite_root, entry, &files))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            inventory,
            inventory_sha256: sha256(inventory_text.as_bytes()),
            cases,
        })
    }

    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }

    pub fn inventory_sha256(&self) -> &str {
        &self.inventory_sha256
    }

    /// Cases in canonical inventory order, with all execution inputs captured.
    pub fn cases(&self) -> &[SealedCase] {
        &self.cases
    }

    /// Enforce identity, kind, order, and the backend's per-case status policy.
    pub fn validate_report(&self, backend: Backend, report: &Report) -> Result<(), String> {
        if report.cases.len() != self.inventory.cases.len() {
            return Err(format!(
                "{backend:?} report has {} cases; sealed inventory requires {}",
                report.cases.len(),
                self.inventory.cases.len()
            ));
        }
        for (expected, actual) in self.inventory.cases.iter().zip(&report.cases) {
            validate_identity(expected, actual, backend)?;
            let allowed = backend_policy(expected, backend);
            if !status_allowed(allowed, actual.status) {
                return Err(format!(
                    "{backend:?} outcome mismatch for {}: policy={allowed:?}, observed={:?}",
                    expected.identifier, actual.status
                ));
            }
        }
        Ok(())
    }

    /// Apply the status policy plus stable typed-cause rules used by receipts.
    pub fn validate_classified_report(
        &self,
        backend: Backend,
        report: &ClassifiedReport,
    ) -> Result<(), String> {
        let plain = Report {
            cases: report
                .cases
                .iter()
                .map(|case| CaseResult {
                    id: case.id.clone(),
                    kind: case.kind,
                    status: case.status,
                    reason: case.reason.clone(),
                })
                .collect(),
        };
        self.validate_report(backend, &plain)?;
        for (expected, actual) in self.inventory.cases.iter().zip(&report.cases) {
            if !code_matches_case(expected, actual) {
                return Err(format!(
                    "{backend:?} outcome code mismatch for {}: status={:?}, code={}",
                    expected.identifier,
                    actual.status,
                    actual.outcome_code.name()
                ));
            }
            let allowed = backend_policy(expected, backend);
            if !code_matches_nonpassing_policy(allowed, actual) {
                return Err(format!(
                    "{backend:?} nonpassing cause mismatch for {}: policy={allowed:?}, code={}",
                    expected.identifier,
                    actual.outcome_code.name()
                ));
            }
        }
        Ok(())
    }
}

fn capture_files(
    suite_root: &Path,
    entries: &[FileEntry],
) -> Result<BTreeMap<String, String>, String> {
    entries
        .iter()
        .map(|entry| Ok((entry.path.clone(), capture_text(suite_root, entry)?)))
        .collect()
}

fn capture_text(suite_root: &Path, entry: &FileEntry) -> Result<String, String> {
    let path = suite_root.join(&entry.path);
    let bytes = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    if sha256(&bytes) != entry.sha256 {
        return Err(format!("digest mismatch while capturing {}", entry.path));
    }
    String::from_utf8(bytes).map_err(|error| format!("{} is not UTF-8: {error}", entry.path))
}

fn materialize_case(
    suite_root: &Path,
    entry: &CaseEntry,
    files: &BTreeMap<String, String>,
) -> Result<SealedCase, String> {
    let mapping_document = relative_filename(entry, entry.mapping_document.as_deref())?;
    let output = relative_filename(entry, entry.output.as_deref())?;
    let required = |path: &str| {
        files
            .get(path)
            .cloned()
            .ok_or_else(|| format!("sealed snapshot omits {path}"))
    };
    Ok(SealedCase {
        directory: suite_root.join("cases").join(&entry.scenario),
        create_sql: required(&format!("cases/{}/create.sql", entry.scenario))?,
        mapping_text: entry
            .mapping_document
            .as_deref()
            .map(required)
            .transpose()?,
        output_text: entry.output.as_deref().map(required).transpose()?,
        case: Case {
            identifier: entry.identifier.clone(),
            kind: manifest_kind(entry.kind),
            mapping_document,
            output,
            has_expected_output: !entry.expected_error,
        },
    })
}

fn relative_filename(entry: &CaseEntry, path: Option<&str>) -> Result<Option<String>, String> {
    let Some(path) = path else {
        return Ok(None);
    };
    let prefix = format!("cases/{}/", entry.scenario);
    path.strip_prefix(&prefix)
        .map(|filename| Some(filename.to_owned()))
        .ok_or_else(|| {
            format!(
                "sealed path {path:?} for {} is outside {prefix}",
                entry.identifier
            )
        })
}

fn validate_identity(
    expected: &CaseEntry,
    actual: &CaseResult,
    backend: Backend,
) -> Result<(), String> {
    if actual.id != expected.identifier {
        return Err(format!(
            "{backend:?} report order/identity mismatch: expected {}, observed {}",
            expected.identifier, actual.id
        ));
    }
    if actual.kind != manifest_kind(expected.kind) {
        return Err(format!(
            "{backend:?} report kind mismatch for {}",
            expected.identifier
        ));
    }
    Ok(())
}

fn code_matches_case(expected: &CaseEntry, actual: &ClassifiedCaseResult) -> bool {
    use OutcomeCode as Code;
    match actual.outcome_code {
        Code::GraphMatched | Code::DatasetMatched => {
            actual.status == Status::Passed && !expected.expected_error
        }
        Code::GraphMismatch | Code::DatasetMismatch => {
            actual.status == Status::Failed && !expected.expected_error
        }
        Code::UnexpectedOutput => actual.status == Status::Failed && expected.expected_error,
        Code::FixtureLoadError
        | Code::IntrospectionError
        | Code::TranslationUnsupported
        | Code::ExecutionUnsupported
        | Code::QuadDumpUnsupported => actual.status == Status::Skipped,
        Code::MappingError
        | Code::SourceValidationError
        | Code::TranslationError
        | Code::ExecutionError
        | Code::QuadDumpError => {
            actual.status
                == if expected.expected_error {
                    Status::Passed
                } else {
                    Status::Failed
                }
        }
        Code::DirectMappingError => {
            expected.kind == CaseKind::DirectMapping && actual.status == Status::Failed
        }
    }
}

fn code_matches_nonpassing_policy(allowed: AllowedOutcome, actual: &ClassifiedCaseResult) -> bool {
    match (allowed, actual.status) {
        (AllowedOutcome::Deviation, Status::Failed) => {
            actual.outcome_code == OutcomeCode::UnexpectedOutput
        }
        (AllowedOutcome::Skip, Status::Skipped) => {
            actual.outcome_code == OutcomeCode::FixtureLoadError
        }
        _ => true,
    }
}

fn backend_policy(entry: &CaseEntry, backend: Backend) -> AllowedOutcome {
    match backend {
        Backend::Sqlite => entry.sqlite,
        Backend::Postgres => entry.postgres,
    }
}

fn manifest_kind(kind: CaseKind) -> Kind {
    match kind {
        CaseKind::R2rml => Kind::R2rml,
        CaseKind::DirectMapping => Kind::DirectMapping,
    }
}

fn status_allowed(policy: AllowedOutcome, observed: Status) -> bool {
    match policy {
        AllowedOutcome::Pass => observed == Status::Passed,
        AllowedOutcome::Deviation => matches!(observed, Status::Passed | Status::Failed),
        AllowedOutcome::Skip => matches!(observed, Status::Passed | Status::Skipped),
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
