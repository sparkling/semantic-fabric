// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::{Deserialize, Serialize};

use crate::{
    AttemptOutcome, AttemptTerminal, AuthorityError, CanonicalInstant, Digest, EventKind,
    FinalWitness, GrantLease, OpaqueId, PreStartTerminal, RegisterClaim, ResourceDisposition,
    ReviewPair, RunPhase, RunTerminalStage, StartAttempt, KERNEL_PROPOSAL_DIGEST_DOMAIN_V1,
    RUN_EVENT_RECORD_KIND_V2, SCHEMA_VERSION_V2, TRANSACTION_KIND_V2,
};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunKey {
    pub project_authority_digest: Digest,
    pub run_id: OpaqueId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseState {
    pub event_digest: Digest,
    pub lease_id: OpaqueId,
    pub runner_id: OpaqueId,
    pub session_id: OpaqueId,
    pub boot_id: OpaqueId,
    pub runner_enrollment_record_digest: Digest,
    pub physical_parent_id: OpaqueId,
    pub resource_ids: Vec<OpaqueId>,
    pub conflict_set_digest: Digest,
    #[serde(with = "crate::decimal::u64_string")]
    pub fence: u64,
    pub service_issued_at: CanonicalInstant,
    pub not_after: CanonicalInstant,
    pub pre_review: ReviewPair,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptState {
    pub event_digest: Digest,
    pub attempt_id: OpaqueId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "terminalKind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TerminalState {
    PreStart {
        event_digest: Digest,
        stage: RunTerminalStage,
    },
    Attempt {
        event_digest: Digest,
        outcome: AttemptOutcome,
        output_envelope_digest: Option<Digest>,
        capture_record_digest: Option<Digest>,
        final_state_digest: Option<Digest>,
    },
}

impl TerminalState {
    pub fn event_digest(&self) -> &Digest {
        match self {
            Self::PreStart { event_digest, .. } | Self::Attempt { event_digest, .. } => {
                event_digest
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunState {
    pub key: RunKey,
    pub phase: RunPhase,
    pub claim_digest: Digest,
    pub registration_event_digest: Digest,
    pub lease: Option<LeaseState>,
    pub attempt: Option<AttemptState>,
    pub terminal: Option<TerminalState>,
    pub final_witness_event_digest: Option<Digest>,
    pub last_event_digest: Digest,
    pub last_run_sequence: u64,
}

impl RunState {
    pub(crate) fn validate(&self) -> Result<(), AuthorityError> {
        let valid = match self.phase {
            RunPhase::Registered => {
                self.last_run_sequence == 0
                    && self.lease.is_none()
                    && self.attempt.is_none()
                    && self.terminal.is_none()
                    && self.final_witness_event_digest.is_none()
            }
            RunPhase::Leased => {
                self.last_run_sequence == 1
                    && self.lease.is_some()
                    && self.attempt.is_none()
                    && self.terminal.is_none()
                    && self.final_witness_event_digest.is_none()
            }
            RunPhase::AttemptStarted => {
                self.last_run_sequence == 2
                    && self.lease.is_some()
                    && self.attempt.is_some()
                    && self.terminal.is_none()
                    && self.final_witness_event_digest.is_none()
            }
            RunPhase::PreStartTerminal => {
                matches!(
                    (&self.lease, &self.terminal, self.last_run_sequence),
                    (
                        None,
                        Some(TerminalState::PreStart {
                            stage: RunTerminalStage::Registration | RunTerminalStage::PreLease,
                            ..
                        }),
                        1,
                    ) | (
                        Some(_),
                        Some(TerminalState::PreStart {
                            stage: RunTerminalStage::LeasedPreStart,
                            ..
                        }),
                        2,
                    )
                ) && self.attempt.is_none()
                    && self.final_witness_event_digest.is_none()
            }
            RunPhase::CandidateSuccessAwaitingFinal => {
                self.last_run_sequence == 3
                    && self.lease.is_some()
                    && self.attempt.is_some()
                    && matches!(
                        self.terminal,
                        Some(TerminalState::Attempt {
                            outcome: AttemptOutcome::CandidateComplete,
                            output_envelope_digest: Some(_),
                            capture_record_digest: Some(_),
                            final_state_digest: Some(_),
                            ..
                        })
                    )
                    && self.final_witness_event_digest.is_none()
            }
            RunPhase::FailedFinalOptional => {
                self.last_run_sequence == 3
                    && self.lease.is_some()
                    && self.attempt.is_some()
                    && matches!(
                        self.terminal.as_ref(),
                        Some(TerminalState::Attempt { outcome, .. })
                            if *outcome != AttemptOutcome::CandidateComplete
                    )
                    && self.final_witness_event_digest.is_none()
            }
            RunPhase::FinalWitnessed => {
                self.last_run_sequence == 4
                    && self.lease.is_some()
                    && self.attempt.is_some()
                    && matches!(
                        self.terminal,
                        Some(TerminalState::Attempt {
                            output_envelope_digest: Some(_),
                            capture_record_digest: Some(_),
                            final_state_digest: Some(_),
                            ..
                        })
                    )
                    && self.final_witness_event_digest.is_some()
            }
        };
        if !valid || self.last_event_digest != *self.expected_last_digest()? {
            return Err(AuthorityError::CorruptState("run state"));
        }
        if let Some(lease) = &self.lease {
            lease.service_issued_at.validate()?;
            lease.not_after.validate()?;
            if lease.fence == 0
                || lease.resource_ids.is_empty()
                || !lease.resource_ids.contains(&lease.physical_parent_id)
                || lease.resource_ids.windows(2).any(|pair| pair[0] >= pair[1])
                || lease.service_issued_at.unix_millis() >= lease.not_after.unix_millis()
            {
                return Err(AuthorityError::CorruptState("lease state"));
            }
        }
        if let Some(TerminalState::Attempt {
            output_envelope_digest,
            capture_record_digest,
            final_state_digest,
            ..
        }) = &self.terminal
        {
            let output_count = [
                output_envelope_digest.is_some(),
                capture_record_digest.is_some(),
                final_state_digest.is_some(),
            ]
            .into_iter()
            .filter(|present| *present)
            .count();
            if output_count != 0 && output_count != 3 {
                return Err(AuthorityError::CorruptState("terminal output set"));
            }
        }
        Ok(())
    }

    fn expected_last_digest(&self) -> Result<&Digest, AuthorityError> {
        if let Some(value) = &self.final_witness_event_digest {
            return Ok(value);
        }
        if let Some(value) = &self.terminal {
            return Ok(value.event_digest());
        }
        if let Some(value) = &self.attempt {
            return Ok(&value.event_digest);
        }
        if let Some(value) = &self.lease {
            return Ok(&value.event_digest);
        }
        Ok(&self.registration_event_digest)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceOwner {
    pub project_authority_digest: Digest,
    pub run_id: OpaqueId,
}

impl From<&RunKey> for ResourceOwner {
    fn from(value: &RunKey) -> Self {
        Self {
            project_authority_digest: value.project_authority_digest.clone(),
            run_id: value.run_id.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceState {
    pub resource_id: OpaqueId,
    #[serde(with = "crate::decimal::u64_string")]
    pub fence: u64,
    pub last_event_digest: Digest,
    pub disposition: ResourceDisposition,
    pub owner: Option<ResourceOwner>,
}

impl ResourceState {
    pub(crate) fn validate(&self) -> Result<(), AuthorityError> {
        let active = matches!(
            self.disposition,
            ResourceDisposition::HeldPreStart | ResourceDisposition::AttemptStarted
        );
        if self.fence == 0 || active != self.owner.is_some() {
            return Err(AuthorityError::CorruptState("resource state"));
        }
        Ok(())
    }

    pub(crate) fn reusable(&self) -> bool {
        matches!(
            self.disposition,
            ResourceDisposition::ReleasedUnstarted | ResourceDisposition::ReleasedAfterCleanup
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeasePolicy {
    pub lease_id: OpaqueId,
    #[serde(with = "crate::decimal::u64_string")]
    pub fence: u64,
    pub service_issued_at: CanonicalInstant,
    pub not_after: CanonicalInstant,
    pub max_attempts: u8,
    pub renew: bool,
    pub release_for_reuse: bool,
    pub reassign: bool,
    pub reclaim: bool,
    pub retry: bool,
}

impl LeasePolicy {
    pub(crate) fn single_use(
        lease_id: OpaqueId,
        fence: u64,
        service_issued_at: CanonicalInstant,
        not_after: CanonicalInstant,
    ) -> Self {
        Self {
            lease_id,
            fence,
            service_issued_at,
            not_after,
            max_attempts: 1,
            renew: false,
            release_for_reuse: false,
            reassign: false,
            reclaim: false,
            retry: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourcePriorState {
    pub resource_id: OpaqueId,
    pub event_digest: Option<Digest>,
    #[serde(with = "crate::decimal::option_u64_string")]
    pub fence: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceTransition {
    pub runner_enrollment_record_digest: Digest,
    pub physical_parent_id: OpaqueId,
    pub conflict_set_digest: Digest,
    #[serde(with = "crate::decimal::u64_string")]
    pub fence: u64,
    pub members: Vec<ResourcePriorState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "eventKind", content = "body", deny_unknown_fields)]
pub enum ResolvedOperation {
    #[serde(rename = "claim-registered-v2")]
    Register(RegisterClaim),
    #[serde(rename = "runner-lease-granted-v2")]
    GrantLease {
        command: GrantLease,
        lease: LeasePolicy,
    },
    #[serde(rename = "capture-attempt-start-committed-v2")]
    StartAttempt(StartAttempt),
    #[serde(rename = "capture-run-terminal-v2")]
    PreStartTerminal {
        command: PreStartTerminal,
        lease_event_digest: Option<Digest>,
        lease_id: Option<OpaqueId>,
        fence: Option<u64>,
    },
    #[serde(rename = "capture-attempt-terminal-v2")]
    AttemptTerminal(AttemptTerminal),
    #[serde(rename = "capture-final-witness-v2")]
    FinalWitness(FinalWitness),
}

impl ResolvedOperation {
    fn event_kind(&self) -> EventKind {
        match self {
            Self::Register(_) => EventKind::ClaimRegistered,
            Self::GrantLease { .. } => EventKind::RunnerLeaseGranted,
            Self::StartAttempt(_) => EventKind::AttemptStartCommitted,
            Self::PreStartTerminal { .. } => EventKind::RunTerminal,
            Self::AttemptTerminal(_) => EventKind::AttemptTerminal,
            Self::FinalWitness(_) => EventKind::FinalWitness,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Unsigned transaction-local materializer input, not a V2 event envelope.
/// The unavailable signer/head adapter must turn this closed proposal into the
/// canonical external record and bind its digest before persistence.
pub struct EventProposal {
    pub schema_version: u8,
    pub transaction_kind: &'static str,
    pub record_kind: &'static str,
    pub event_kind: EventKind,
    pub project_authority_digest: Digest,
    pub run_id: OpaqueId,
    pub semantic_request_digest: Digest,
    #[serde(with = "crate::decimal::u64_string")]
    pub global_sequence: u64,
    #[serde(with = "crate::decimal::u64_string")]
    pub run_sequence: u64,
    pub previous_run_event_digest: Option<Digest>,
    pub resource_transition: Option<ResourceTransition>,
    pub operation: ResolvedOperation,
}

impl EventProposal {
    pub(crate) fn new(
        key: &RunKey,
        semantic_request_digest: Digest,
        global_sequence: u64,
        run_sequence: u64,
        previous_run_event_digest: Option<Digest>,
        resource_transition: Option<ResourceTransition>,
        operation: ResolvedOperation,
    ) -> Result<Self, AuthorityError> {
        if operation.event_kind().as_str().is_empty() {
            return Err(AuthorityError::CorruptState("event kind"));
        }
        Ok(Self {
            schema_version: SCHEMA_VERSION_V2,
            transaction_kind: TRANSACTION_KIND_V2,
            record_kind: RUN_EVENT_RECORD_KIND_V2,
            event_kind: operation.event_kind(),
            project_authority_digest: key.project_authority_digest.clone(),
            run_id: key.run_id.clone(),
            semantic_request_digest,
            global_sequence,
            run_sequence,
            previous_run_event_digest,
            resource_transition,
            operation,
        })
    }

    pub fn proposal_digest(&self) -> Result<Digest, AuthorityError> {
        let value = serde_json::json!({
            "domain": KERNEL_PROPOSAL_DIGEST_DOMAIN_V1,
            "proposal": self,
        });
        let bytes = serde_json::to_vec(&value)
            .map_err(|_| AuthorityError::CorruptState("event proposal"))?;
        Ok(Digest::sha256(bytes))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PlannedMutation {
    pub proposal: EventProposal,
    pub prior_run: Option<RunState>,
    pub prior_resources: Vec<ResourceState>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactResult {
    pub event_kind: EventKind,
    pub event_digest: Digest,
    pub global_sequence: u64,
    pub run_sequence: u64,
    pub event_envelope: Vec<u8>,
    pub response_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct StoredEvent {
    pub project_authority_digest: Digest,
    pub run_id: OpaqueId,
    pub semantic_request_digest: Digest,
    pub canonical_request: Vec<u8>,
    pub canonical_request_sha256: Digest,
    pub operation_digest: Digest,
    pub proposal_digest: Digest,
    pub event_kind: EventKind,
    pub event_digest: Digest,
    pub global_sequence: u64,
    pub run_sequence: u64,
    pub event_envelope: Vec<u8>,
    pub response_bytes: Vec<u8>,
    pub response_sha256: Digest,
}

impl StoredEvent {
    pub(crate) fn exact_result(&self) -> ExactResult {
        ExactResult {
            event_kind: self.event_kind,
            event_digest: self.event_digest.clone(),
            global_sequence: self.global_sequence,
            run_sequence: self.run_sequence,
            event_envelope: self.event_envelope.clone(),
            response_bytes: self.response_bytes.clone(),
        }
    }

    pub(crate) fn validate(&self) -> Result<(), AuthorityError> {
        if Digest::sha256(&self.canonical_request) != self.canonical_request_sha256
            || Digest::sha256(&self.response_bytes) != self.response_sha256
            || self.event_envelope.is_empty()
            || self.response_bytes.is_empty()
        {
            return Err(AuthorityError::CorruptState("stored exact result"));
        }
        Ok(())
    }
}
