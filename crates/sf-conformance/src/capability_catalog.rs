//! Strict, evidence-bound capability catalog for M0 architectural truth.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};

use serde::Serialize;
use sha2::{Digest, Sha256};

pub use crate::capability_model::*;

pub const CATALOG_PATH: &str = "tests/capabilities/catalog-v1.json";
pub const SCHEMA_PATH: &str = "tests/capabilities/schema-v1.json";

#[derive(Debug, Clone)]
pub struct LoadedCatalog {
    pub catalog: Catalog,
    pub catalog_sha256: String,
    pub schema_sha256: String,
}

pub fn load(repo_root: &Path) -> Result<LoadedCatalog, String> {
    let catalog_path = repo_root.join(CATALOG_PATH);
    let bytes = fs::read(&catalog_path)
        .map_err(|error| format!("read {}: {error}", catalog_path.display()))?;
    let catalog: Catalog = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {}: {error}", catalog_path.display()))?;
    validate(repo_root, &catalog)?;
    let schema_sha256 = validate_schema(repo_root)?;
    Ok(LoadedCatalog {
        catalog,
        catalog_sha256: sha256(&bytes),
        schema_sha256,
    })
}

pub fn parse_and_validate(repo_root: &Path, bytes: &[u8]) -> Result<Catalog, String> {
    let catalog: Catalog = serde_json::from_slice(bytes)
        .map_err(|error| format!("parse capability catalog: {error}"))?;
    validate(repo_root, &catalog)?;
    Ok(catalog)
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<String, String> {
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|error| format!("serialize capability catalog: {error}"))?;
    text.push('\n');
    Ok(text)
}

pub fn compact_json<T: Serialize>(value: &T) -> Result<String, String> {
    let mut text = serde_json::to_string(value)
        .map_err(|error| format!("serialize capability catalog: {error}"))?;
    text.push('\n');
    Ok(text)
}

pub fn validate(repo_root: &Path, catalog: &Catalog) -> Result<(), String> {
    if catalog.schema_version != 1 {
        return Err("capability catalog schemaVersion must be 1".to_owned());
    }
    validate_date("asOf", &catalog.as_of)?;
    let standards = unique_sorted_ids("standards", &catalog.standards, |item| &item.id)?;
    let profiles = unique_sorted_ids("profiles", &catalog.profiles, |item| &item.id)?;
    let backends = unique_sorted_ids("backends", &catalog.backends, |item| &item.id)?;
    let commands = unique_sorted_ids("commands", &catalog.commands, |item| &item.id)?;
    let evidence = unique_sorted_ids("evidence", &catalog.evidence, |item| &item.id)?;
    let limitations = unique_sorted_ids("limitations", &catalog.limitations, |item| &item.id)?;
    let cells = unique_sorted_ids("cells", &catalog.cells, |item| &item.id)?;
    unique_sorted_ids("claims", &catalog.claims, |item| &item.id)?;
    unique_sorted_ids("scopeExclusions", &catalog.scope_exclusions, |item| {
        &item.id
    })?;

    for standard in &catalog.standards {
        validate_date("standard snapshotDate", &standard.snapshot_date)?;
        validate_sha256("standard", &standard.sha256)?;
        if standard.title.trim().is_empty()
            || standard.status.trim().is_empty()
            || standard.byte_length == 0
            || !standard.url.starts_with("https://www.w3.org/TR/")
        {
            return Err(format!("standard {} metadata is incomplete", standard.id));
        }
    }
    for command in &catalog.commands {
        if command.argv.trim().is_empty() {
            return Err(format!("command {} has empty argv", command.id));
        }
    }
    for profile in &catalog.profiles {
        unique_sorted_strings(
            &format!("profile {} backends", profile.id),
            &profile.backends,
        )?;
        if profile.capability.trim().is_empty() || profile.backends.is_empty() {
            return Err(format!("profile {} is incomplete", profile.id));
        }
        for backend in &profile.backends {
            require_id("backend", backend, &backends)?;
        }
    }
    for item in &catalog.evidence {
        validate_evidence(repo_root, item, &commands)?;
    }
    for limitation in &catalog.limitations {
        if limitation.summary.trim().is_empty() || !limitation.adr.starts_with("ADR-") {
            return Err(format!("limitation {} is incomplete", limitation.id));
        }
    }

    let mut observed_pairs = BTreeSet::new();
    for cell in &catalog.cells {
        let profile = catalog
            .profiles
            .iter()
            .find(|profile| profile.id == cell.profile_id)
            .ok_or_else(|| format!("cell {} references unknown profile", cell.id))?;
        require_id("backend", &cell.backend_id, &backends)?;
        if !profile.backends.contains(&cell.backend_id) {
            return Err(format!(
                "cell {} is outside its profile cross-product",
                cell.id
            ));
        }
        if !observed_pairs.insert((cell.profile_id.as_str(), cell.backend_id.as_str())) {
            return Err(format!(
                "duplicate cell for {}/{}",
                cell.profile_id, cell.backend_id
            ));
        }
        validate_cell(
            cell,
            profile,
            &evidence,
            &limitations,
            &catalog.evidence,
            &catalog.limitations,
        )?;
    }
    let expected_pairs: BTreeSet<_> = catalog
        .profiles
        .iter()
        .flat_map(|profile| {
            profile
                .backends
                .iter()
                .map(move |backend| (profile.id.as_str(), backend.as_str()))
        })
        .collect();
    if observed_pairs != expected_pairs {
        return Err(
            "capability cells do not cover the exact profile/backend cross-product".to_owned(),
        );
    }

    for claim in &catalog.claims {
        unique_sorted_strings(&format!("claim {} cells", claim.id), &claim.cell_ids)?;
        if claim.text.trim().is_empty() || claim.cell_ids.is_empty() {
            return Err(format!("claim {} is incomplete", claim.id));
        }
        for cell_id in &claim.cell_ids {
            require_id("cell", cell_id, &cells)?;
        }
        if claim.kind == ClaimKind::Current
            && claim.cell_ids.iter().any(|id| {
                catalog
                    .cells
                    .iter()
                    .find(|cell| &cell.id == id)
                    .is_none_or(|cell| !cell.advertisable || cell.status != Status::Implemented)
            })
        {
            return Err(format!(
                "current claim {} references a non-advertisable cell",
                claim.id
            ));
        }
    }
    if standards.is_empty() || profiles.is_empty() {
        return Err("capability catalog cannot be empty".to_owned());
    }
    Ok(())
}

fn validate_evidence(
    repo_root: &Path,
    evidence: &Evidence,
    commands: &BTreeSet<String>,
) -> Result<(), String> {
    validate_sha256("evidence", &evidence.sha256)?;
    if evidence.selector.trim().is_empty() {
        return Err(format!("evidence {} has an empty selector", evidence.id));
    }
    if let Some(command) = &evidence.command_id {
        require_id("command", command, commands)?;
    }
    if matches!(
        evidence.verification,
        Verification::CiRequired | Verification::Receipt
    ) && !evidence.required
    {
        return Err(format!(
            "evidence {} has a required grade but required=false",
            evidence.id
        ));
    }
    let relative = Path::new(&evidence.path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("evidence {} path is not normalized", evidence.id));
    }
    let path = repo_root.join(relative);
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "read evidence {} at {}: {error}",
            evidence.id,
            path.display()
        )
    })?;
    let observed = sha256(&bytes);
    if evidence.sha256 != observed {
        return Err(format!(
            "evidence {} digest mismatch: recorded={}, actual={observed}",
            evidence.id, evidence.sha256
        ));
    }
    Ok(())
}

fn validate_cell(
    cell: &Cell,
    profile: &Profile,
    evidence_ids: &BTreeSet<String>,
    limitation_ids: &BTreeSet<String>,
    all_evidence: &[Evidence],
    all_limitations: &[Limitation],
) -> Result<(), String> {
    unique_sorted_strings(&format!("cell {} evidence", cell.id), &cell.evidence_ids)?;
    unique_sorted_strings(
        &format!("cell {} limitations", cell.id),
        &cell.limitation_ids,
    )?;
    if cell.qualification.trim().is_empty() {
        return Err(format!("cell {} has no qualification", cell.id));
    }
    for id in &cell.evidence_ids {
        require_id("evidence", id, evidence_ids)?;
    }
    for id in &cell.limitation_ids {
        require_id("limitation", id, limitation_ids)?;
    }
    let selected_evidence: Vec<_> = all_evidence
        .iter()
        .filter(|item| cell.evidence_ids.contains(&item.id))
        .collect();
    let selected_limitations: Vec<_> = all_limitations
        .iter()
        .filter(|item| cell.limitation_ids.contains(&item.id))
        .collect();
    if (!cell.semantic_exact || !cell.bounded) && cell.advertisable {
        return Err(format!(
            "cell {} cannot advertise inexact or unbounded behavior",
            cell.id
        ));
    }
    match cell.status {
        Status::Implemented => {
            if selected_evidence.is_empty()
                || !selected_evidence
                    .iter()
                    .any(|item| item.verification == cell.verification)
                || selected_evidence
                    .iter()
                    .all(|item| item.domain == EvidenceDomain::ArchitecturePlan)
            {
                return Err(format!(
                    "implemented cell {} lacks direct evidence at its grade",
                    cell.id
                ));
            }
        }
        Status::Admitted => {
            if !cell.semantic_exact
                || !cell.bounded
                || !cell.advertisable
                || !matches!(
                    cell.verification,
                    Verification::CiRequired | Verification::Receipt
                )
                || selected_limitations
                    .iter()
                    .any(|item| item.release_blocking)
            {
                return Err(format!(
                    "admitted cell {} does not satisfy production law",
                    cell.id
                ));
            }
        }
        Status::Planned => {
            if cell.advertisable
                || !selected_limitations
                    .iter()
                    .any(|item| item.release_blocking)
                || !selected_evidence
                    .iter()
                    .any(|item| item.domain == EvidenceDomain::ArchitecturePlan)
            {
                return Err(format!(
                    "planned cell {} lacks a blocking plan record",
                    cell.id
                ));
            }
        }
        Status::Unsupported => {
            if cell.advertisable
                || !selected_evidence
                    .iter()
                    .any(|item| item.domain == EvidenceDomain::NegativeRejection)
            {
                return Err(format!(
                    "unsupported cell {} lacks exact rejection evidence",
                    cell.id
                ));
            }
        }
    }
    if matches!(
        profile.surface,
        Surface::SparqlQuery | Surface::SparqlProtocol
    ) && selected_evidence.iter().any(|item| {
        matches!(
            item.domain,
            EvidenceDomain::MappingExecution | EvidenceDomain::MappingFixture
        )
    }) {
        return Err(format!(
            "cell {} illegally promotes mapping evidence",
            cell.id
        ));
    }
    if profile.surface == Surface::MappingExecution
        && selected_evidence.iter().any(|item| {
            matches!(
                item.domain,
                EvidenceDomain::SparqlQuery | EvidenceDomain::SparqlProtocol
            )
        })
    {
        return Err(format!(
            "cell {} illegally mixes mapping and query evidence",
            cell.id
        ));
    }
    Ok(())
}

fn validate_schema(repo_root: &Path) -> Result<String, String> {
    let path = repo_root.join(SCHEMA_PATH);
    let bytes = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {}: {error}", path.display()))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("{} root must be an object", SCHEMA_PATH))?;
    if object.get("type").and_then(|value| value.as_str()) != Some("object")
        || object
            .get("additionalProperties")
            .and_then(|value| value.as_bool())
            != Some(false)
        || object.get("$schema").and_then(|value| value.as_str())
            != Some("https://json-schema.org/draft/2020-12/schema")
    {
        return Err(format!(
            "{} does not declare the strict v1 object contract",
            SCHEMA_PATH
        ));
    }
    Ok(sha256(&bytes))
}

fn unique_sorted_ids<T>(
    label: &str,
    values: &[T],
    id: impl Fn(&T) -> &String,
) -> Result<BTreeSet<String>, String> {
    let ids: Vec<_> = values.iter().map(id).cloned().collect();
    unique_sorted_strings(label, &ids)?;
    Ok(ids.into_iter().collect())
}

fn unique_sorted_strings(label: &str, values: &[String]) -> Result<(), String> {
    if values.iter().any(|value| !valid_id(value)) {
        return Err(format!("{label} contains an identifier outside [a-z0-9-]"));
    }
    let mut sorted = values.to_vec();
    sorted.sort();
    sorted.dedup();
    if sorted != values {
        return Err(format!("{label} must be lexically sorted and unique"));
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    value
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn require_id(label: &str, id: &str, known: &BTreeSet<String>) -> Result<(), String> {
    if known.contains(id) {
        Ok(())
    } else {
        Err(format!("unknown {label} identifier {id}"))
    }
}

fn validate_date(label: &str, value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    if bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        Ok(())
    } else {
        Err(format!("{label} must be YYYY-MM-DD"))
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
        Err(format!("{label} SHA-256 is invalid"))
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn status_counts(catalog: &Catalog) -> BTreeMap<Status, usize> {
    let mut counts = BTreeMap::new();
    for cell in &catalog.cells {
        *counts.entry(cell.status).or_insert(0) += 1;
    }
    counts
}
