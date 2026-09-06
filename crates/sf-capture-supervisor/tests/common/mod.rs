// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(dead_code)]

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use async_trait::async_trait;
use sf_capture_supervisor::{
    ApplyOutcome, AttemptOutcome, AttemptTerminal, AuthorityError, AuthorityRequest,
    CleanupEvidence, Digest, DispositionEvidence, EgressDisposition, EventMaterializerPort,
    EventProposal, ExactResult, FinalWitness, GrantLease, LeaseDisposition, MaterializedEvent,
    OpaqueId, Operation, PreStartTerminal, ProcessDisposition, RegisterClaim, ResourceDisposition,
    ResourceTransition, ReviewPair, RunTerminalOutcome, RunTerminalStage, RunnerIdentity,
    StartAttempt,
};
use tokio::sync::Mutex;

pub const NOW_MILLIS: i64 = 1_787_000_000_000;

pub fn digest(value: impl AsRef<[u8]>) -> Digest {
    Digest::sha256(value)
}

pub fn oid(value: impl Into<String>) -> OpaqueId {
    OpaqueId::parse(value).expect("valid fixture ID")
}

pub fn project(label: &str) -> Digest {
    digest(format!("project:{label}"))
}

pub fn run(label: &str) -> OpaqueId {
    oid(format!("run_{label}_20260906"))
}

pub fn claim(run_id: &OpaqueId) -> Digest {
    digest(format!("claim:{}", run_id.as_str()))
}

#[derive(Default)]
pub struct RecordingMaterializer {
    calls: AtomicUsize,
    proposals: Mutex<Vec<EventProposal>>,
}

impl RecordingMaterializer {
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub async fn last(&self) -> EventProposal {
        self.proposals
            .lock()
            .await
            .last()
            .expect("materialized proposal")
            .clone()
    }

    pub async fn all(&self) -> Vec<EventProposal> {
        self.proposals.lock().await.clone()
    }
}

#[async_trait]
impl EventMaterializerPort for RecordingMaterializer {
    async fn materialize(
        &self,
        proposal: &EventProposal,
    ) -> Result<MaterializedEvent, AuthorityError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.proposals.lock().await.push(proposal.clone());
        let proposal_digest = proposal.proposal_digest()?;
        let event_digest = digest(format!("event:{proposal_digest}"));
        let mut event_envelope = serde_json::to_vec(&serde_json::json!({
            "eventDigest": event_digest,
            "proposalDigest": proposal_digest,
        }))
        .map_err(|error| AuthorityError::Materialization(error.to_string()))?;
        event_envelope.push(b'\n');
        let mut response_bytes = serde_json::to_vec(&serde_json::json!({
            "eventDigest": event_digest,
            "globalSequence": proposal.global_sequence,
            "runSequence": proposal.run_sequence,
        }))
        .map_err(|error| AuthorityError::Materialization(error.to_string()))?;
        response_bytes.push(b'\n');
        Ok(MaterializedEvent {
            proposal_digest,
            event_digest,
            event_envelope,
            response_bytes,
        })
    }
}

pub fn exact(outcome: ApplyOutcome) -> ExactResult {
    outcome.result().clone()
}

pub fn register_request(project: &Digest, run_id: &OpaqueId, variant: &str) -> AuthorityRequest {
    request(
        project,
        run_id,
        &format!("register:{variant}"),
        Operation::Register(RegisterClaim {
            claim_key_digest: digest(format!("claim-key:{variant}")),
            claim_digest: claim(run_id),
            rooted_claim_validation_digest: digest(format!("rooted-validation:{variant}")),
        }),
    )
}

pub fn lease_request(
    project: &Digest,
    run_id: &OpaqueId,
    registration: &ExactResult,
    resource_ids: Vec<OpaqueId>,
    variant: &str,
) -> AuthorityRequest {
    lease_request_with_duration(project, run_id, registration, resource_ids, variant, 1_000)
}

pub fn lease_request_with_duration(
    project: &Digest,
    run_id: &OpaqueId,
    registration: &ExactResult,
    resource_ids: Vec<OpaqueId>,
    variant: &str,
    lease_duration_millis: u64,
) -> AuthorityRequest {
    let physical_parent_id = resource_ids
        .iter()
        .find(|id| id.as_str().starts_with("parent_"))
        .expect("parent fixture resource")
        .clone();
    request(
        project,
        run_id,
        &format!("lease:{variant}"),
        Operation::GrantLease(GrantLease {
            registration_event_digest: registration.event_digest.clone(),
            claim_digest: claim(run_id),
            admission_challenge_digest: digest(format!("challenge:{variant}")),
            admission_evidence_digest: digest(format!("admission:{variant}")),
            runner: RunnerIdentity {
                runner_id: oid("runner_native_20260906"),
                enrollment_record_digest: digest("runner-enrollment"),
                session_id: oid("session_native_20260906"),
                boot_id: oid("boot_native_20260906"),
                key_epoch: 1,
                key_fingerprint: digest("key-fingerprint"),
                possession_proof_digest: digest("possession-proof"),
            },
            host_evidence_digest: digest("host-evidence"),
            runner_profile_digest: digest("runner-profile"),
            control_policy_digest: digest("control-policy"),
            pre_review: ReviewPair {
                codex_receipt_digest: digest("pre-review-codex"),
                claude_receipt_digest: digest("pre-review-claude"),
            },
            lease_id: oid(format!("lease_{}", run_id.as_str())),
            lease_duration_millis,
            physical_parent_id,
            resource_ids,
        }),
    )
}

pub fn start_request(
    project: &Digest,
    run_id: &OpaqueId,
    lease: &ExactResult,
    transition: &ResourceTransition,
    variant: &str,
) -> AuthorityRequest {
    request(
        project,
        run_id,
        &format!("start:{variant}"),
        Operation::StartAttempt(StartAttempt {
            lease_event_digest: lease.event_digest.clone(),
            lease_id: oid(format!("lease_{}", run_id.as_str())),
            fence: transition.fence,
            runner_id: oid("runner_native_20260906"),
            session_id: oid("session_native_20260906"),
            boot_id: oid("boot_native_20260906"),
            resource_conflict_set_digest: transition.conflict_set_digest.clone(),
            quiescence_digest: digest("quiescence"),
            fresh_host_preflight_digest: digest("fresh-host-preflight"),
            held_source_digest: digest("held-source"),
            producer_agreement_digest: digest("producer-agreement"),
            producer_artifact_digest: digest("producer-artifact"),
            producer_runtime_closure_digest: digest("producer-runtime-closure"),
            command_digest: digest("command"),
            environment_digest: digest("environment"),
            output_slot_digest: digest("output-slot"),
            capture_nonce_digest: digest("capture-nonce"),
            attempt_id: oid(format!("attempt_{}", run_id.as_str())),
        }),
    )
}

pub fn prestart_terminal_request(
    project: &Digest,
    run_id: &OpaqueId,
    registration: &ExactResult,
    expired: bool,
    variant: &str,
) -> AuthorityRequest {
    request(
        project,
        run_id,
        &format!("prestart-terminal:{variant}"),
        Operation::PreStartTerminal(PreStartTerminal {
            terminal_stage: RunTerminalStage::LeasedPreStart,
            outcome_code: if expired {
                RunTerminalOutcome::LeasedPreStartExpired
            } else {
                RunTerminalOutcome::LeasedPreStartPreflightFailed
            },
            registration_event_digest: registration.event_digest.clone(),
            outcome_evidence_digest: digest(format!("prestart-evidence:{variant}")),
            resource_disposition: Some(DispositionEvidence {
                kind: ResourceDisposition::ReleasedUnstarted,
                evidence_digest: digest(format!("release-evidence:{variant}")),
            }),
        }),
    )
}

pub fn attempt_terminal_request(
    project: &Digest,
    run_id: &OpaqueId,
    lease: &ExactResult,
    start: &ExactResult,
    fence: u64,
    outcome: AttemptOutcome,
    variant: &str,
) -> AuthorityRequest {
    let candidate = outcome == AttemptOutcome::CandidateComplete;
    request(
        project,
        run_id,
        &format!("attempt-terminal:{variant}"),
        Operation::AttemptTerminal(AttemptTerminal {
            start_event_digest: start.event_digest.clone(),
            lease_event_digest: lease.event_digest.clone(),
            lease_id: oid(format!("lease_{}", run_id.as_str())),
            fence,
            attempt_id: oid(format!("attempt_{}", run_id.as_str())),
            outcome_code: outcome,
            outcome_evidence_digest: digest(format!("terminal-evidence:{variant}")),
            process_disposition: DispositionEvidence {
                kind: if candidate {
                    ProcessDisposition::ExitedZero
                } else {
                    ProcessDisposition::Unknown
                },
                evidence_digest: digest(format!("process-evidence:{variant}")),
            },
            egress_disposition: DispositionEvidence {
                kind: if candidate {
                    EgressDisposition::IsolatedNoViolation
                } else {
                    EgressDisposition::Unknown
                },
                evidence_digest: digest(format!("egress-evidence:{variant}")),
            },
            lease_disposition: DispositionEvidence {
                kind: LeaseDisposition::SpentNeverReusable,
                evidence_digest: digest(format!("lease-disposition:{variant}")),
            },
            resource_disposition: DispositionEvidence {
                kind: if candidate {
                    ResourceDisposition::ReleasedAfterCleanup
                } else {
                    ResourceDisposition::Quarantined
                },
                evidence_digest: digest(format!("resource-disposition:{variant}")),
            },
            cleanup: CleanupEvidence {
                process_cleanup_digest: candidate.then(|| digest("process-cleanup")),
                egress_cleanup_digest: candidate.then(|| digest("egress-cleanup")),
                resource_cleanup_digest: candidate.then(|| digest("resource-cleanup")),
            },
            output_envelope_digest: candidate.then(|| digest("output-envelope")),
            capture_record_digest: candidate.then(|| digest("capture-record")),
            final_state_digest: candidate.then(|| digest("final-state")),
        }),
    )
}

pub fn final_witness_request(
    project: &Digest,
    run_id: &OpaqueId,
    lease: &ExactResult,
    terminal: &ExactResult,
    fence: u64,
    variant: &str,
) -> AuthorityRequest {
    request(
        project,
        run_id,
        &format!("final-witness:{variant}"),
        Operation::FinalWitness(FinalWitness {
            attempt_terminal_event_digest: terminal.event_digest.clone(),
            lease_event_digest: lease.event_digest.clone(),
            lease_id: oid(format!("lease_{}", run_id.as_str())),
            fence,
            attempt_id: oid(format!("attempt_{}", run_id.as_str())),
            frozen_envelope_digest: digest("output-envelope"),
            capture_record_digest: digest("capture-record"),
            final_state_digest: digest("final-state"),
            replay_validation_digest: digest("replay-validation"),
            post_review: ReviewPair {
                codex_receipt_digest: digest("post-review-codex"),
                claude_receipt_digest: digest("post-review-claude"),
            },
        }),
    )
}

pub fn resources(parent: &str, child: &str) -> Vec<OpaqueId> {
    let mut values = vec![oid(parent), oid(child)];
    values.sort();
    values
}

pub fn request(
    project: &Digest,
    run_id: &OpaqueId,
    label: &str,
    operation: Operation,
) -> AuthorityRequest {
    let mut canonical_request = serde_json::to_vec(&serde_json::json!({
        "label": label,
        "operation": &operation,
    }))
    .expect("serialize fixture request");
    canonical_request.push(b'\n');
    AuthorityRequest::new(
        project.clone(),
        run_id.clone(),
        digest(format!("semantic-request:{label}")),
        canonical_request,
        operation,
    )
    .expect("valid fixture request")
}

pub fn shared_materializer() -> Arc<RecordingMaterializer> {
    Arc::new(RecordingMaterializer::default())
}
