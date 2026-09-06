use std::collections::BTreeSet;

pub const HEADER: &str = "semantic-fabric-supported-surface-manifest-v1";
pub const HASH_ALGORITHM: &str = "sha256";
pub const REFERENCE_BINDING: &str = "retrieved-reference-metadata";

const QUERY_SCOPE: &str = "sqlite-public-query-supported-surface-not-mapping-protocol-full-w3c-runtime-provenance-or-backend-admission";
const PROTOCOL_SCOPE: &str = "sqlite-public-protocol-supported-surface-not-mapping-query-full-w3c-runtime-provenance-or-backend-admission";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    SparqlQuery,
    SparqlProtocol,
}

impl Surface {
    pub fn name(self) -> &'static str {
        match self {
            Self::SparqlQuery => "sparql-query",
            Self::SparqlProtocol => "sparql-protocol",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "sparql-query" => Some(Self::SparqlQuery),
            "sparql-protocol" => Some(Self::SparqlProtocol),
            _ => None,
        }
    }

    pub fn attestation_scope(self) -> &'static str {
        match self {
            Self::SparqlQuery => QUERY_SCOPE,
            Self::SparqlProtocol => PROTOCOL_SCOPE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardReference {
    pub id: String,
    pub status: String,
    pub snapshot_date: String,
    pub byte_length: u64,
    pub sha256: String,
}

impl StandardReference {
    pub fn for_surface(surface: Surface) -> Self {
        match surface {
            Surface::SparqlQuery => Self {
                id: "sparql12-query-2026".to_owned(),
                status: "w3c-working-draft".to_owned(),
                snapshot_date: "2026-06-25".to_owned(),
                byte_length: 1_021_305,
                sha256: "85cf4270bdc0cc3191ed84803fa1a91fe258776448ea0c82fc91685c577fb4c1"
                    .to_owned(),
            },
            Surface::SparqlProtocol => Self {
                id: "sparql12-protocol-2026".to_owned(),
                status: "w3c-working-draft".to_owned(),
                snapshot_date: "2026-07-08".to_owned(),
                byte_length: 160_577,
                sha256: "4056a2bb3fbb383d3067eee060ee725a8cf3a689ec2420629dda28b9939cfe07"
                    .to_owned(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedStatus {
    Supported,
    Unsupported,
    Rejected,
}

impl ExpectedStatus {
    pub fn name(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
            Self::Rejected => "rejected",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "supported" => Some(Self::Supported),
            "unsupported" => Some(Self::Unsupported),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Case {
    pub id: String,
    pub scenario: String,
    pub expected_status: ExpectedStatus,
    pub http_status: u16,
    pub response_media_type: String,
    pub cause: String,
    pub result_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub profile_id: String,
    pub surface: Surface,
    pub backend: String,
    pub standard: StandardReference,
    pub cases: Vec<Case>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestSeal {
    pub profile_id: String,
    pub surface: Surface,
    pub case_count: usize,
    pub manifest_sha256: String,
}

pub(crate) fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    validate_token("profile id", &manifest.profile_id)?;
    if manifest.backend != "sqlite" {
        return Err("supported-surface manifests are SQLite-only".to_owned());
    }
    if manifest.standard != StandardReference::for_surface(manifest.surface) {
        return Err("standard reference does not match the fixed surface snapshot".to_owned());
    }
    if manifest.cases.is_empty() {
        return Err("supported-surface manifest has no cases".to_owned());
    }
    let mut previous = None;
    let mut scenarios = BTreeSet::new();
    for case in &manifest.cases {
        validate_token("case id", &case.id)?;
        validate_token("case scenario", &case.scenario)?;
        validate_token("case cause", &case.cause)?;
        validate_token("response media type", &case.response_media_type)?;
        validate_sha256("case result", &case.result_sha256)?;
        if previous.is_some_and(|prior| prior >= case.id.as_str()) {
            return Err("case records are not strictly ordered".to_owned());
        }
        previous = Some(case.id.as_str());
        if !scenarios.insert(case.scenario.as_str()) {
            return Err("case scenarios must be unique".to_owned());
        }
        validate_outcome(case)?;
    }
    Ok(())
}

fn validate_outcome(case: &Case) -> Result<(), String> {
    let valid = match case.expected_status {
        ExpectedStatus::Supported => {
            case.http_status == 200
                && supported_cause_matches_media(&case.cause, &case.response_media_type)
        }
        ExpectedStatus::Unsupported => {
            case.http_status == 501
                && case.response_media_type == "application/problem+json"
                && case.cause == "unsupported-query"
        }
        ExpectedStatus::Rejected => {
            case.response_media_type == "application/problem+json"
                && matches!(
                    (case.http_status, case.cause.as_str()),
                    (400, "invalid-request")
                        | (405, "method-not-allowed")
                        | (406, "not-acceptable")
                        | (413, "payload-too-large")
                        | (415, "unsupported-media-type")
                )
        }
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "case {} has an invalid status/code/media/cause combination",
            case.id
        ))
    }
}

fn supported_cause_matches_media(cause: &str, media_type: &str) -> bool {
    let query_results = matches!(
        media_type,
        "application/sparql-results+json"
            | "application/sparql-results+xml"
            | "text/csv"
            | "text/tab-separated-values"
    );
    let rdf_graph = matches!(
        media_type,
        "text/turtle" | "application/n-triples" | "application/ld+json"
    );
    match cause {
        "ask-boolean" | "select-bindings" | "transport-query" | "representation-query-results" => {
            query_results
        }
        "construct-graph" | "describe-graph" | "representation-rdf-graph" => rdf_graph,
        "service-description" => media_type == "text/turtle",
        _ => false,
    }
}

pub(crate) fn validate_token(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 200
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.' | b'/' | b'+')
        })
    {
        Err(format!("invalid {label}"))
    } else {
        Ok(())
    }
}

pub(crate) fn validate_sha256(label: &str, value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!("invalid SHA-256 for {label}"))
    }
}
