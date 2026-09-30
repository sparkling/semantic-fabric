//! Embedding-selected query-shape profile (ADR-0056, SOURCE work only).
//!
//! Status: source-level opt-in. Nothing here provisions a named endpoint or
//! claims closure of ADR-0056. The profile is chosen by the embedding before
//! the config is shared; no request header, claim or serialized identity can
//! select or authorize it.

use super::ServeConfig;

/// Closed set of query-shape profiles for the serving router.
///
/// `Ordinary` retains the existing query behaviour byte for byte.
/// `GeneratedSelectAsk` composes the accepted sealed mapping coverage and
/// runtime binding with subject admission (`QueryAdmission` stays mandatory
/// and runs first). Under it only successful SELECT and ASK are admitted; an
/// uncovered constant is refused instead of narrowing to an empty result.
///
/// Limits that remain unfinished obligations:
/// * Every dataset clause (`FROM`, `FROM NAMED`) is refused. The pinned
///   allowlist is empty; nonempty allowlist semantics are not implemented and
///   no setter for one is exposed.
/// * Lineage (provenance) requests and the bounded federation modes are
///   refused explicitly, never silently stripped or bypassed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum QueryShapeProfile {
    /// Existing behaviour, no issued profile identity.
    #[default]
    Ordinary,
    /// Opt-in generated-query profile: SELECT/ASK only, coverage refusal,
    /// issued attestation header on successful responses.
    GeneratedSelectAsk,
}

impl ServeConfig {
    /// Select the query-shape profile before sharing this config. Changes
    /// require a new server; the profile is immutable once the config is shared.
    pub fn set_query_shape_profile(&mut self, profile: QueryShapeProfile) {
        self.query_shape = profile;
    }

    /// The profile selected by the embedding.
    pub fn query_shape_profile(&self) -> QueryShapeProfile {
        self.query_shape
    }

    pub(crate) fn generated_active(&self) -> bool {
        self.query_shape == QueryShapeProfile::GeneratedSelectAsk
    }
}
