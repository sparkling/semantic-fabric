// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::{Deserialize, Serialize};

use crate::{
    request_json::validate_request_bytes, AttemptOutcome, AuthorityError, Digest,
    EgressDisposition, EventKind, LeaseDisposition, OpaqueId, ProcessDisposition,
    ResourceDisposition, RunTerminalOutcome, RunTerminalStage,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewPair {
    pub codex_receipt_digest: Digest,
    pub claude_receipt_digest: Digest,
}

impl ReviewPair {
    fn validate(&self) -> Result<(), AuthorityError> {
        if self.codex_receipt_digest == self.claude_receipt_digest {
            return Err(AuthorityError::InvalidInput(
                "distinct native review receipts",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerIdentity {
    pub runner_id: OpaqueId,
    pub enrollment_record_digest: Digest,
    pub session_id: OpaqueId,
    pub boot_id: OpaqueId,
    #[serde(with = "crate::decimal::u64_string")]
    pub key_epoch: u64,
    pub key_fingerprint: Digest,
    pub possession_proof_digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterClaim {
    pub claim_key_digest: Digest,
    pub claim_digest: Digest,
    pub rooted_claim_validation_digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GrantLease {
    pub registration_event_digest: Digest,
    pub claim_digest: Digest,
    pub admission_challenge_digest: Digest,
    pub admission_evidence_digest: Digest,
    pub runner: RunnerIdentity,
    pub host_evidence_digest: Digest,
    pub runner_profile_digest: Digest,
    pub control_policy_digest: Digest,
    pub pre_review: ReviewPair,
    pub lease_id: OpaqueId,
    pub lease_duration_millis: u64,
    pub physical_parent_id: OpaqueId,
    pub resource_ids: Vec<OpaqueId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartAttempt {
    pub lease_event_digest: Digest,
    pub lease_id: OpaqueId,
    #[serde(with = "crate::decimal::u64_string")]
    pub fence: u64,
    pub runner_id: OpaqueId,
    pub session_id: OpaqueId,
    pub boot_id: OpaqueId,
    pub resource_conflict_set_digest: Digest,
    pub quiescence_digest: Digest,
    pub fresh_host_preflight_digest: Digest,
    pub held_source_digest: Digest,
    pub producer_agreement_digest: Digest,
    pub producer_artifact_digest: Digest,
    pub producer_runtime_closure_digest: Digest,
    pub command_digest: Digest,
    pub environment_digest: Digest,
    pub output_slot_digest: Digest,
    pub capture_nonce_digest: Digest,
    pub attempt_id: OpaqueId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreStartTerminal {
    pub terminal_stage: RunTerminalStage,
    pub outcome_code: RunTerminalOutcome,
    pub registration_event_digest: Digest,
    pub outcome_evidence_digest: Digest,
    pub resource_disposition: Option<ResourceDispositionEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DispositionEvidence<T> {
    pub kind: T,
    pub evidence_digest: Digest,
}

pub type ResourceDispositionEvidence = DispositionEvidence<ResourceDisposition>;
pub type ProcessDispositionEvidence = DispositionEvidence<ProcessDisposition>;
pub type EgressDispositionEvidence = DispositionEvidence<EgressDisposition>;
pub type LeaseDispositionEvidence = DispositionEvidence<LeaseDisposition>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupEvidence {
    pub process_cleanup_digest: Option<Digest>,
    pub egress_cleanup_digest: Option<Digest>,
    pub resource_cleanup_digest: Option<Digest>,
}

impl CleanupEvidence {
    pub fn complete(&self) -> bool {
        self.process_cleanup_digest.is_some()
            && self.egress_cleanup_digest.is_some()
            && self.resource_cleanup_digest.is_some()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptTerminal {
    pub start_event_digest: Digest,
    pub lease_event_digest: Digest,
    pub lease_id: OpaqueId,
    #[serde(with = "crate::decimal::u64_string")]
    pub fence: u64,
    pub attempt_id: OpaqueId,
    pub outcome_code: AttemptOutcome,
    pub outcome_evidence_digest: Digest,
    pub process_disposition: ProcessDispositionEvidence,
    pub egress_disposition: EgressDispositionEvidence,
    pub lease_disposition: LeaseDispositionEvidence,
    pub resource_disposition: ResourceDispositionEvidence,
    pub cleanup: CleanupEvidence,
    pub output_envelope_digest: Option<Digest>,
    pub capture_record_digest: Option<Digest>,
    pub final_state_digest: Option<Digest>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalWitness {
    pub attempt_terminal_event_digest: Digest,
    pub lease_event_digest: Digest,
    pub lease_id: OpaqueId,
    #[serde(with = "crate::decimal::u64_string")]
    pub fence: u64,
    pub attempt_id: OpaqueId,
    pub frozen_envelope_digest: Digest,
    pub capture_record_digest: Digest,
    pub final_state_digest: Digest,
    pub replay_validation_digest: Digest,
    pub post_review: ReviewPair,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "eventKind", content = "body", deny_unknown_fields)]
pub enum Operation {
    #[serde(rename = "claim-registered-v2")]
    Register(RegisterClaim),
    #[serde(rename = "runner-lease-granted-v2")]
    GrantLease(GrantLease),
    #[serde(rename = "capture-attempt-start-committed-v2")]
    StartAttempt(StartAttempt),
    #[serde(rename = "capture-run-terminal-v2")]
    PreStartTerminal(PreStartTerminal),
    #[serde(rename = "capture-attempt-terminal-v2")]
    AttemptTerminal(AttemptTerminal),
    #[serde(rename = "capture-final-witness-v2")]
    FinalWitness(FinalWitness),
}

impl Operation {
    pub fn event_kind(&self) -> EventKind {
        match self {
            Self::Register(_) => EventKind::ClaimRegistered,
            Self::GrantLease(_) => EventKind::RunnerLeaseGranted,
            Self::StartAttempt(_) => EventKind::AttemptStartCommitted,
            Self::PreStartTerminal(_) => EventKind::RunTerminal,
            Self::AttemptTerminal(_) => EventKind::AttemptTerminal,
            Self::FinalWitness(_) => EventKind::FinalWitness,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), AuthorityError> {
        match self {
            Self::Register(_) => Ok(()),
            Self::GrantLease(value) => {
                value.pre_review.validate()?;
                if value.runner.key_epoch == 0
                    || value.lease_duration_millis == 0
                    || value.lease_duration_millis > 86_400_000
                {
                    return Err(AuthorityError::InvalidInput("lease policy"));
                }
                if value.resource_ids.is_empty()
                    || value.resource_ids.len() > crate::MAX_RESOURCE_MEMBERS_V2
                    || !value.resource_ids.contains(&value.physical_parent_id)
                    || value.resource_ids.windows(2).any(|pair| pair[0] >= pair[1])
                {
                    return Err(AuthorityError::InvalidInput("sorted resource conflict set"));
                }
                Ok(())
            }
            Self::StartAttempt(value) => {
                if value.fence == 0 {
                    return Err(AuthorityError::InvalidInput("non-zero resource fence"));
                }
                Ok(())
            }
            Self::PreStartTerminal(value) => value.validate(),
            Self::AttemptTerminal(value) => value.validate(),
            Self::FinalWitness(value) => {
                value.post_review.validate()?;
                if value.fence == 0 {
                    return Err(AuthorityError::InvalidInput("non-zero resource fence"));
                }
                Ok(())
            }
        }
    }
}

impl PreStartTerminal {
    fn validate(&self) -> Result<(), AuthorityError> {
        let stage_matches = matches!(
            (self.terminal_stage, self.outcome_code),
            (
                RunTerminalStage::Registration,
                RunTerminalOutcome::RegistrationChangedReplay
            ) | (
                RunTerminalStage::Registration,
                RunTerminalOutcome::RegistrationAuthenticatedDenial
            ) | (
                RunTerminalStage::PreLease,
                RunTerminalOutcome::PreLeaseAdmissionFailed
            ) | (
                RunTerminalStage::PreLease,
                RunTerminalOutcome::PreLeasePreReviewFailed
            ) | (
                RunTerminalStage::PreLease,
                RunTerminalOutcome::PreLeaseRunnerUnavailable
            ) | (
                RunTerminalStage::PreLease,
                RunTerminalOutcome::PreLeasePolicyFailed
            ) | (
                RunTerminalStage::PreLease,
                RunTerminalOutcome::PreLeaseInternalFailure
            ) | (
                RunTerminalStage::LeasedPreStart,
                RunTerminalOutcome::LeasedPreStartExpired
            ) | (
                RunTerminalStage::LeasedPreStart,
                RunTerminalOutcome::LeasedPreStartAdmissionRevoked
            ) | (
                RunTerminalStage::LeasedPreStart,
                RunTerminalOutcome::LeasedPreStartPreflightFailed
            ) | (
                RunTerminalStage::LeasedPreStart,
                RunTerminalOutcome::LeasedPreStartInternalFailure
            )
        );
        let disposition_matches = match self.terminal_stage {
            RunTerminalStage::LeasedPreStart => matches!(
                self.resource_disposition.as_ref().map(|value| value.kind),
                Some(ResourceDisposition::ReleasedUnstarted | ResourceDisposition::Quarantined)
            ),
            _ => self.resource_disposition.is_none(),
        };
        if !stage_matches || !disposition_matches {
            return Err(AuthorityError::InvalidInput("pre-start terminal outcome"));
        }
        Ok(())
    }
}

impl AttemptTerminal {
    fn validate(&self) -> Result<(), AuthorityError> {
        if self.fence == 0 {
            return Err(AuthorityError::InvalidInput("non-zero resource fence"));
        }
        let output_count = [
            self.output_envelope_digest.is_some(),
            self.capture_record_digest.is_some(),
            self.final_state_digest.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        if output_count != 0 && output_count != 3 {
            return Err(AuthorityError::InvalidInput("complete attempt output set"));
        }
        let released = self.resource_disposition.kind == ResourceDisposition::ReleasedAfterCleanup;
        if released
            && (!self.cleanup.complete()
                || self.process_disposition.kind == ProcessDisposition::RunnerLost
                || self.process_disposition.kind == ProcessDisposition::Unknown
                || self.egress_disposition.kind != EgressDisposition::IsolatedNoViolation)
        {
            return Err(AuthorityError::InvalidInput("safe resource release"));
        }
        let valid = match self.outcome_code {
            AttemptOutcome::CandidateComplete => {
                output_count == 3
                    && self.process_disposition.kind == ProcessDisposition::ExitedZero
                    && self.egress_disposition.kind == EgressDisposition::IsolatedNoViolation
                    && released
            }
            AttemptOutcome::ProcessFailed => {
                matches!(
                    self.process_disposition.kind,
                    ProcessDisposition::ExitedNonzero | ProcessDisposition::Terminated
                ) && output_count == 0
            }
            AttemptOutcome::Timeout => {
                output_count == 0
                    && matches!(
                        self.process_disposition.kind,
                        ProcessDisposition::Terminated | ProcessDisposition::Unknown
                    )
            }
            AttemptOutcome::RunnerLost => {
                output_count == 0
                    && self.process_disposition.kind == ProcessDisposition::RunnerLost
                    && self.egress_disposition.kind == EgressDisposition::Unknown
                    && self.resource_disposition.kind == ResourceDisposition::Quarantined
            }
            AttemptOutcome::OutputMissing => output_count == 0,
            AttemptOutcome::CleanupFailed => {
                !self.cleanup.complete()
                    && self.resource_disposition.kind == ResourceDisposition::Quarantined
            }
            AttemptOutcome::EgressViolation => {
                self.egress_disposition.kind == EgressDisposition::ViolationDetected
                    && self.resource_disposition.kind == ResourceDisposition::Quarantined
            }
            AttemptOutcome::FenceInvalidated => {
                output_count == 0
                    && self.resource_disposition.kind == ResourceDisposition::Quarantined
            }
            AttemptOutcome::OutputInvalid => output_count == 0,
            AttemptOutcome::InternalFailure => {
                output_count == 0
                    && self.resource_disposition.kind == ResourceDisposition::Quarantined
            }
        };
        if !valid {
            return Err(AuthorityError::InvalidInput("attempt outcome disposition"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorityRequest {
    pub project_authority_digest: Digest,
    pub run_id: OpaqueId,
    pub semantic_request_digest: Digest,
    pub canonical_request: Vec<u8>,
    pub canonical_request_sha256: Digest,
    pub operation_digest: Digest,
    pub operation: Operation,
}

impl AuthorityRequest {
    pub fn new(
        project_authority_digest: Digest,
        run_id: OpaqueId,
        semantic_request_digest: Digest,
        canonical_request: Vec<u8>,
        operation: Operation,
    ) -> Result<Self, AuthorityError> {
        operation.validate()?;
        validate_request_bytes(&canonical_request)?;
        let canonical_request_sha256 = Digest::sha256(&canonical_request);
        let operation_bytes = serde_json::to_vec(&operation)
            .map_err(|_| AuthorityError::InvalidInput("operation"))?;
        let request = Self {
            project_authority_digest,
            run_id,
            semantic_request_digest,
            canonical_request,
            canonical_request_sha256,
            operation_digest: Digest::sha256(operation_bytes),
            operation,
        };
        request.validate()?;
        Ok(request)
    }

    pub(crate) fn validate(&self) -> Result<(), AuthorityError> {
        self.operation.validate()?;
        validate_request_bytes(&self.canonical_request)?;
        let operation_bytes = serde_json::to_vec(&self.operation)
            .map_err(|_| AuthorityError::InvalidInput("operation"))?;
        if Digest::sha256(&self.canonical_request) != self.canonical_request_sha256
            || Digest::sha256(operation_bytes) != self.operation_digest
        {
            return Err(AuthorityError::InvalidInput("request binding"));
        }
        Ok(())
    }
}
