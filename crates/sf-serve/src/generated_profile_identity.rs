//! Private deterministic response-profile identity for the generated-query
//! profile (ADR-0056). A derived identifier, never a credential: possessing or
//! presenting it grants no admission. Parsed wire values are comparison-only
//! claims and can never be turned back into a minted identity.

use std::fmt;

use oxrdf::NamedNode;
use sf_sparql::{MappingDigest, OntologyDigest, SemanticAdmissionDigest};
use sha2::{Digest, Sha256};

const IDENTITY_DOMAIN: &[u8] = b"semantic-fabric/generated-profile-identity/v1";
const WIRE_PREFIX: &str = "sfgp1:";
const HEX_LEN: usize = 64;

pub(crate) const MAX_GRAPHS: usize = 64;
pub(crate) const MAX_GRAPH_IRI_BYTES: usize = 2048;
pub(crate) const MAX_GRAPH_TOTAL_BYTES: usize = 32 * 1024;

/// Typed, redacted failure. Never carries an IRI or any input fragment.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum IdentityError {
    #[error("pinned graph allowlist has too many graphs")]
    TooManyGraphs,
    #[error("pinned graph IRI exceeds its byte limit")]
    GraphTooLong,
    #[error("pinned graph allowlist exceeds its total byte limit")]
    TotalBytesExceeded,
    #[error("pinned graph IRI is not a valid absolute IRI")]
    InvalidGraphIri,
    #[error("pinned graph allowlist contains a duplicate graph")]
    DuplicateGraph,
    #[error("generated profile identity wire value is malformed")]
    MalformedWire,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProfileVersion {
    V1,
    #[cfg(test)]
    TestAlternate,
}

impl ProfileVersion {
    const fn label(self) -> &'static [u8] {
        match self {
            Self::V1 => b"generated-query-profile/v1",
            #[cfg(test)]
            Self::TestAlternate => b"generated-query-profile/test-alternate",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FormRules {
    SelectAskOnly,
    #[cfg(test)]
    TestAlternate,
}

impl FormRules {
    const fn label(self) -> &'static [u8] {
        match self {
            Self::SelectAskOnly => b"forms/select-ask-only/v1",
            #[cfg(test)]
            Self::TestAlternate => b"forms/test-alternate",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ServicePolicy {
    Refuse,
    #[cfg(test)]
    TestAlternate,
}

impl ServicePolicy {
    const fn label(self) -> &'static [u8] {
        match self {
            Self::Refuse => b"service/refuse/v1",
            #[cfg(test)]
            Self::TestAlternate => b"service/test-alternate",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DatasetPolicy {
    PinnedGraphAllowlist,
    SingleDefaultGraphAllowlist,
    #[cfg(test)]
    TestAlternate,
}

impl DatasetPolicy {
    const fn label(self) -> &'static [u8] {
        match self {
            Self::PinnedGraphAllowlist => b"dataset/pinned-graph-allowlist/v1",
            Self::SingleDefaultGraphAllowlist => b"dataset/single-default-graph-allowlist/v1",
            #[cfg(test)]
            Self::TestAlternate => b"dataset/test-alternate",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoveragePolicy {
    PinnedMappingConstants,
    #[cfg(test)]
    TestAlternate,
}

impl CoveragePolicy {
    const fn label(self) -> &'static [u8] {
        match self {
            Self::PinnedMappingConstants => b"coverage/pinned-mapping-constants/v1",
            #[cfg(test)]
            Self::TestAlternate => b"coverage/test-alternate",
        }
    }
}

/// Versioned profile constants. Only crate-owned constructors exist; fields
/// are private and there is no setter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProfileDescriptor {
    version: ProfileVersion,
    form: FormRules,
    service: ServicePolicy,
    dataset: DatasetPolicy,
    coverage: CoveragePolicy,
}

impl ProfileDescriptor {
    /// Fixed source-only dataset policy; constructing it grants no admission.
    pub(crate) const fn single_default_dataset() -> Self {
        Self {
            dataset: DatasetPolicy::SingleDefaultGraphAllowlist,
            ..Self::v1()
        }
    }

    pub(crate) const fn v1() -> Self {
        Self {
            version: ProfileVersion::V1,
            form: FormRules::SelectAskOnly,
            service: ServicePolicy::Refuse,
            dataset: DatasetPolicy::PinnedGraphAllowlist,
            coverage: CoveragePolicy::PinnedMappingConstants,
        }
    }
}

/// Bounded, validated set of pinned named-graph IRIs.
///
/// Contract: every entry must parse as an absolute IRI (`oxrdf::NamedNode`);
/// exact byte-duplicates are refused (no normalisation, so IRIs that are
/// equivalent but spelled differently are distinct); input order is
/// irrelevant because entries are stored sorted bytewise. Count and byte
/// limits are checked before each entry is parsed or stored. An empty set is
/// valid and pins no graph.
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct PinnedGraphAllowlist {
    graphs: Vec<String>,
}

impl PinnedGraphAllowlist {
    pub(crate) fn new<I>(iris: I) -> Result<Self, IdentityError>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let mut graphs: Vec<String> = Vec::new();
        let mut total: usize = 0;
        for item in iris {
            let iri = item.as_ref();
            if graphs.len() >= MAX_GRAPHS {
                return Err(IdentityError::TooManyGraphs);
            }
            if iri.len() > MAX_GRAPH_IRI_BYTES {
                return Err(IdentityError::GraphTooLong);
            }
            total = total
                .checked_add(iri.len())
                .filter(|sum| *sum <= MAX_GRAPH_TOTAL_BYTES)
                .ok_or(IdentityError::TotalBytesExceeded)?;
            NamedNode::new(iri).map_err(|_| IdentityError::InvalidGraphIri)?;
            graphs.push(iri.to_owned());
        }
        graphs.sort_unstable();
        if graphs.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(IdentityError::DuplicateGraph);
        }
        Ok(Self { graphs })
    }
}

impl fmt::Debug for PinnedGraphAllowlist {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PinnedGraphAllowlist")
            .field("graphs", &self.graphs.len())
            .finish_non_exhaustive()
    }
}

/// Every field is preceded by its u64 big-endian byte length, and the graph
/// list by its u64 big-endian count, so no two input tuples share a preimage.
struct FramedHasher(Sha256);

impl FramedHasher {
    fn field(&mut self, bytes: &[u8]) {
        self.count(bytes.len());
        self.0.update(bytes);
    }

    fn count(&mut self, value: usize) {
        let value = u64::try_from(value).unwrap_or(u64::MAX);
        self.0.update(value.to_be_bytes());
    }
}

/// Server-derived identity. Only [`mint`] constructs it. Not a credential.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(crate) struct GeneratedProfileIdentity([u8; 32]);

/// Derive the identity from the validated mapping's own digests, the explicit
/// profile constants and the pinned graph allowlist. No other input exists.
pub(crate) fn mint(
    mapping: MappingDigest,
    ontology: OntologyDigest,
    admission: SemanticAdmissionDigest,
    profile: &ProfileDescriptor,
    allowlist: &PinnedGraphAllowlist,
) -> GeneratedProfileIdentity {
    let mut hasher = FramedHasher(Sha256::new());
    hasher.field(IDENTITY_DOMAIN);
    hasher.field(profile.version.label());
    hasher.field(profile.form.label());
    hasher.field(profile.service.label());
    hasher.field(profile.dataset.label());
    hasher.field(profile.coverage.label());
    hasher.field(mapping.as_bytes());
    hasher.field(ontology.as_bytes());
    hasher.field(admission.as_bytes());
    hasher.count(allowlist.graphs.len());
    for graph in &allowlist.graphs {
        hasher.field(graph.as_bytes());
    }
    GeneratedProfileIdentity(hasher.0.finalize().into())
}

impl GeneratedProfileIdentity {
    /// Stable ASCII wire form: `sfgp1:` followed by 64 lowercase hex digits.
    pub(crate) fn wire(&self) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(WIRE_PREFIX.len() + HEX_LEN);
        out.push_str(WIRE_PREFIX);
        for byte in &self.0 {
            out.push(char::from(DIGITS[usize::from(*byte >> 4)]));
            out.push(char::from(DIGITS[usize::from(*byte & 0x0f)]));
        }
        out
    }

    pub(crate) const fn claim(&self) -> GeneratedProfileClaim {
        GeneratedProfileClaim(self.0)
    }
}

impl fmt::Debug for GeneratedProfileIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeneratedProfileIdentity")
            .finish_non_exhaustive()
    }
}

/// Comparison-only value parsed from the wire. It cannot be converted to a
/// minted identity and confers no authority.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct GeneratedProfileClaim([u8; 32]);

impl GeneratedProfileClaim {
    pub(crate) fn parse(wire: &str) -> Result<Self, IdentityError> {
        let hex = wire
            .strip_prefix(WIRE_PREFIX)
            .ok_or(IdentityError::MalformedWire)?
            .as_bytes();
        if hex.len() != HEX_LEN {
            return Err(IdentityError::MalformedWire);
        }
        let mut out = [0u8; 32];
        for (slot, pair) in out.iter_mut().zip(hex.chunks_exact(2)) {
            let high = nibble(pair[0]).ok_or(IdentityError::MalformedWire)?;
            let low = nibble(pair[1]).ok_or(IdentityError::MalformedWire)?;
            *slot = (high << 4) | low;
        }
        Ok(Self(out))
    }

    pub(crate) fn matches(&self, identity: &GeneratedProfileIdentity) -> bool {
        self.0 == identity.0
    }
}

impl fmt::Debug for GeneratedProfileClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeneratedProfileClaim")
            .finish_non_exhaustive()
    }
}

const fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[path = "generated_profile_identity_tests.rs"]
mod generated_profile_identity_tests;
