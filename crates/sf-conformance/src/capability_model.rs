//! Serialized model for the strict M0 capability catalog.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Catalog {
    pub schema_version: u32,
    pub as_of: String,
    pub standards: Vec<Standard>,
    pub profiles: Vec<Profile>,
    pub backends: Vec<Backend>,
    pub commands: Vec<Command>,
    pub evidence: Vec<Evidence>,
    pub limitations: Vec<Limitation>,
    pub cells: Vec<Cell>,
    pub claims: Vec<Claim>,
    pub scope_exclusions: Vec<ScopeExclusion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Standard {
    pub id: String,
    pub title: String,
    pub status: String,
    pub url: String,
    pub snapshot_date: String,
    pub sha256: String,
    pub byte_length: u64,
    pub classification: StandardClassification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StandardClassification {
    RetrievedReferenceMetadata,
    ProjectSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub surface: Surface,
    pub capability: String,
    pub backends: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Surface {
    MappingExecution,
    SparqlQuery,
    SparqlProtocol,
    RuntimeSourcePath,
    ProductionSourceAdmission,
    RuntimeArchitecture,
    ResourceGovernance,
    Operations,
    Release,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backend {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Command {
    pub id: String,
    pub argv: String,
    pub mode: CommandMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandMode {
    Required,
    Diagnostic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Evidence {
    pub id: String,
    pub domain: EvidenceDomain,
    pub path: String,
    pub sha256: String,
    pub selector: String,
    pub command_id: Option<String>,
    pub verification: Verification,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceDomain {
    ArchitecturePlan,
    BackendAdmission,
    BackendLive,
    Benchmark,
    MappingExecution,
    MappingFixture,
    NegativeRejection,
    SparqlProtocol,
    SparqlQuery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verification {
    SourceOnly,
    Mock,
    LiveOptional,
    CiRequired,
    Receipt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Limitation {
    pub id: String,
    pub summary: String,
    pub release_blocking: bool,
    pub adr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Cell {
    pub id: String,
    pub profile_id: String,
    pub backend_id: String,
    pub status: Status,
    pub verification: Verification,
    pub evidence_ids: Vec<String>,
    pub limitation_ids: Vec<String>,
    pub semantic_exact: bool,
    pub bounded: bool,
    pub advertisable: bool,
    pub qualification: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Admitted,
    Implemented,
    Planned,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Claim {
    pub id: String,
    pub kind: ClaimKind,
    pub text: String,
    pub cell_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaimKind {
    Current,
    Qualification,
    Limitation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeExclusion {
    pub id: String,
    pub text: String,
}
