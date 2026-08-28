use std::path::{Component, Path};

pub const PROFILE_HEADER: &str = "semantic-fabric-regression-profile-v1";
pub const BASELINE_HEADER: &str = "semantic-fabric-expected-regression-baseline-v1";
pub const HASH_ALGORITHM: &str = "sha256";
pub const ATTESTATION_SCOPE: &str =
    "expected-local-test-outcomes-not-runtime-provenance-w3c-conformance-or-backend-admission";

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Include,
    Exclude,
}

impl Disposition {
    pub fn name(self) -> &'static str {
        match self {
            Self::Include => "include",
            Self::Exclude => "exclude",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "include" => Some(Self::Include),
            "exclude" => Some(Self::Exclude),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputBinding {
    pub path: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suite {
    pub id: String,
    pub package: String,
    pub target: String,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestIdentity {
    pub suite_id: String,
    pub name: String,
    pub disposition: Disposition,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub profile_id: String,
    pub surface: Surface,
    pub backend: String,
    pub inputs: Vec<InputBinding>,
    pub suites: Vec<Suite>,
    pub tests: Vec<TestIdentity>,
}

/// One exact required test identity and its only admissible baseline outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineRow {
    pub suite_id: String,
    pub test_name: String,
}

/// Expected deterministic local regression outcomes. This is not runtime
/// provenance: it deliberately contains no host, time, executable, or runner hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedBaseline {
    pub profile_id: String,
    pub surface: Surface,
    pub backend: String,
    pub inventory_path: String,
    pub inventory_sha256: String,
    pub outcomes_sha256: String,
    pub suite_count: usize,
    pub excluded_count: usize,
    pub rows: Vec<BaselineRow>,
}

pub fn validate_token(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 200
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_:./".contains(&byte))
    {
        return Err(format!("invalid {label} {value:?}"));
    }
    Ok(())
}

pub fn validate_relative_path(value: &str) -> Result<(), String> {
    validate_token("relative path", value)?;
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "path must be a normalized relative path: {value:?}"
        ));
    }
    Ok(())
}

pub fn validate_sha256(label: &str, value: &str) -> Result<(), String> {
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
