//! Fail-closed execution view of the canonical W3C RDB2RDF inventory.

use std::path::{Path, PathBuf};

use crate::inventory::{self, AllowedOutcome, CaseEntry, CaseKind, Inventory};
use crate::manifest::{Case, Kind};
use crate::{Report, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone)]
pub struct SealedCase {
    pub directory: PathBuf,
    pub case: Case,
}

#[derive(Debug, Clone)]
pub struct SealedSuite {
    inventory: Inventory,
    cases: Vec<SealedCase>,
}

impl SealedSuite {
    /// Validate the complete snapshot before materialising any execution case.
    pub fn load(suite_root: &Path) -> Result<Self, String> {
        let inventory = inventory::check(suite_root, &suite_root.join("inventory.tsv"))?;
        let cases = inventory
            .cases
            .iter()
            .map(|entry| materialize_case(suite_root, entry))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { inventory, cases })
    }

    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }

    /// Cases in the canonical inventory order, not filesystem discovery order.
    pub fn cases(&self) -> &[SealedCase] {
        &self.cases
    }

    /// Enforce identity, kind, order, and the backend's per-case allowed policy.
    pub fn validate_report(&self, backend: Backend, report: &Report) -> Result<(), String> {
        if report.cases.len() != self.inventory.cases.len() {
            return Err(format!(
                "{backend:?} report has {} cases; sealed inventory requires {}",
                report.cases.len(),
                self.inventory.cases.len()
            ));
        }
        for (expected, actual) in self.inventory.cases.iter().zip(&report.cases) {
            if actual.id != expected.identifier {
                return Err(format!(
                    "{backend:?} report order/identity mismatch: expected {}, observed {}",
                    expected.identifier, actual.id
                ));
            }
            let expected_kind = manifest_kind(expected.kind);
            if actual.kind != expected_kind {
                return Err(format!(
                    "{backend:?} report kind mismatch for {}",
                    expected.identifier
                ));
            }
            let allowed = match backend {
                Backend::Sqlite => expected.sqlite,
                Backend::Postgres => expected.postgres,
            };
            if !status_allowed(allowed, actual.status) {
                return Err(format!(
                    "{backend:?} outcome mismatch for {}: policy={allowed:?}, observed={:?}",
                    expected.identifier, actual.status
                ));
            }
        }
        Ok(())
    }
}

fn materialize_case(suite_root: &Path, entry: &CaseEntry) -> Result<SealedCase, String> {
    let mapping_document = relative_filename(entry, entry.mapping_document.as_deref())?;
    let output = relative_filename(entry, entry.output.as_deref())?;
    Ok(SealedCase {
        directory: suite_root.join("cases").join(&entry.scenario),
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
