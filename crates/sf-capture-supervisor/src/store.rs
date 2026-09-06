// SPDX-License-Identifier: MIT OR Apache-2.0

use async_trait::async_trait;
use thiserror::Error;

use crate::{
    AuthorityRequest, Digest, EventProposal, ExactResult, StoredEvent, MAX_EVENT_BYTES_V2,
    MAX_RESULT_BYTES_V2,
};

#[derive(Debug, Error)]
pub enum AuthorityError {
    #[error("invalid authority input: {0}")]
    InvalidInput(&'static str),
    #[error("authority state is malformed or inconsistent: {0}")]
    CorruptState(&'static str),
    #[error("authority is not provisioned")]
    AuthorityNotProvisioned,
    #[error("run is unknown")]
    UnknownRun,
    #[error("a changed request conflicts with an immutable request slot")]
    ChangedRequest,
    #[error("transition is not valid from the current run state")]
    InvalidTransition,
    #[error("one or more overlapping resources are held or quarantined")]
    ResourceUnavailable,
    #[error("lease has expired")]
    LeaseExpired,
    #[error("lease has not expired")]
    LeaseNotExpired,
    #[error("exact committed result is absent")]
    ExactResultMissing,
    #[error("external event materialization failed: {0}")]
    Materialization(String),
    #[error("materialized event does not bind the exact proposal: {0}")]
    MaterializationMismatch(&'static str),
    #[error("transaction commit resolution is unknown; exact recovery only")]
    CommitOutcomeUnknown,
    #[error("injected crash at {0}")]
    InjectedCrash(&'static str),
    #[error("PostgreSQL authority-store failure: {0}")]
    Postgres(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplyOutcome {
    Committed(ExactResult),
    Recovered(ExactResult),
}

impl ApplyOutcome {
    pub fn result(&self) -> &ExactResult {
        match self {
            Self::Committed(value) | Self::Recovered(value) => value,
        }
    }

    pub fn was_recovered(&self) -> bool {
        matches!(self, Self::Recovered(_))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedEvent {
    pub proposal_digest: Digest,
    pub event_digest: Digest,
    pub event_envelope: Vec<u8>,
    pub response_bytes: Vec<u8>,
}

/// Future signer/envelope boundary. This crate intentionally provides no
/// production implementation and makes no signature-validity claim.
#[async_trait]
pub trait EventMaterializerPort: Send + Sync {
    async fn materialize(
        &self,
        proposal: &EventProposal,
    ) -> Result<MaterializedEvent, AuthorityError>;
}

/// Service-authoritative time. Production PostgreSQL uses the transaction's
/// database time; this port exists only for the deterministic reference store.
pub trait ServiceClock: Send + Sync {
    fn now_unix_millis(&self) -> Result<i64, AuthorityError>;
}

#[async_trait]
pub trait AuthorityStore: Send + Sync {
    async fn apply(
        &self,
        request: &AuthorityRequest,
        materializer: &dyn EventMaterializerPort,
    ) -> Result<ApplyOutcome, AuthorityError>;

    /// Exact-only recovery performs no transition and consults no current run,
    /// resource, policy or authority-head state.
    async fn recover_exact(
        &self,
        request: &AuthorityRequest,
    ) -> Result<ExactResult, AuthorityError>;
}

pub(crate) fn validate_materialized(
    request: &AuthorityRequest,
    proposal: &EventProposal,
    value: MaterializedEvent,
) -> Result<StoredEvent, AuthorityError> {
    let proposal_digest = proposal.proposal_digest()?;
    if value.proposal_digest != proposal_digest {
        return Err(AuthorityError::MaterializationMismatch("proposal digest"));
    }
    if value.event_envelope.is_empty() || value.event_envelope.len() > MAX_EVENT_BYTES_V2 {
        return Err(AuthorityError::MaterializationMismatch(
            "event envelope bounds",
        ));
    }
    if value.response_bytes.is_empty() || value.response_bytes.len() > MAX_RESULT_BYTES_V2 {
        return Err(AuthorityError::MaterializationMismatch("response bounds"));
    }
    if std::str::from_utf8(&value.event_envelope).is_err()
        || std::str::from_utf8(&value.response_bytes).is_err()
    {
        return Err(AuthorityError::MaterializationMismatch(
            "canonical UTF-8 bytes",
        ));
    }
    Ok(StoredEvent {
        project_authority_digest: request.project_authority_digest.clone(),
        run_id: request.run_id.clone(),
        semantic_request_digest: request.semantic_request_digest.clone(),
        canonical_request: request.canonical_request.clone(),
        canonical_request_sha256: request.canonical_request_sha256.clone(),
        operation_digest: request.operation_digest.clone(),
        proposal_digest,
        event_kind: proposal.event_kind,
        event_digest: value.event_digest,
        global_sequence: proposal.global_sequence,
        run_sequence: proposal.run_sequence,
        event_envelope: value.event_envelope,
        response_sha256: Digest::sha256(&value.response_bytes),
        response_bytes: value.response_bytes,
    })
}

pub(crate) fn exact_match(
    stored: &StoredEvent,
    request: &AuthorityRequest,
) -> Result<ExactResult, AuthorityError> {
    stored.validate()?;
    if stored.project_authority_digest != request.project_authority_digest
        || stored.run_id != request.run_id
        || stored.semantic_request_digest != request.semantic_request_digest
        || stored.canonical_request_sha256 != request.canonical_request_sha256
        || stored.canonical_request != request.canonical_request
        || stored.operation_digest != request.operation_digest
        || stored.event_kind != request.operation.event_kind()
    {
        return Err(AuthorityError::ChangedRequest);
    }
    Ok(stored.exact_result())
}
