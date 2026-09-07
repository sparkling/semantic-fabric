// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::resource_plan::{
    continuation_resource_transition, continue_resources, lease_resource_transition,
    transition_resources,
};
use crate::{
    AttemptOutcome, AuthorityError, AuthorityRequest, CanonicalInstant, Digest, EventProposal,
    LeasePolicy, LeaseState, Operation, PlannedMutation, ResolvedOperation, ResourceDisposition,
    ResourceState, ResourceTransition, RunKey, RunPhase, RunState, RunTerminalOutcome,
    RunTerminalStage, TerminalState,
};

pub(crate) fn plan_transition(
    request: &AuthorityRequest,
    current_run: Option<&RunState>,
    current_resources: &[ResourceState],
    global_sequence: u64,
    now_millis: i64,
) -> Result<PlannedMutation, AuthorityError> {
    request.operation.validate()?;
    if global_sequence == 0 {
        return Err(AuthorityError::CorruptState("global sequence"));
    }
    let key = RunKey {
        project_authority_digest: request.project_authority_digest.clone(),
        run_id: request.run_id.clone(),
    };
    if let Some(run) = current_run {
        run.validate()?;
        if run.key != key {
            return Err(AuthorityError::CorruptState("run identity"));
        }
    }
    for resource in current_resources {
        resource.validate()?;
    }
    let now = CanonicalInstant::from_unix_millis(now_millis)?;
    let (run_sequence, previous, resource_transition, operation) = match &request.operation {
        Operation::Register(value) => {
            if current_run.is_some() {
                return Err(AuthorityError::ChangedRequest);
            }
            (0, None, None, ResolvedOperation::Register(value.clone()))
        }
        Operation::GrantLease(value) => {
            let run = require_phase(
                current_run,
                RunPhase::Registered,
                request.operation.event_kind(),
            )?;
            if value.registration_event_digest != run.registration_event_digest
                || value.claim_digest != run.claim_digest
            {
                return Err(AuthorityError::InvalidTransition);
            }
            let not_after_millis = now_millis
                .checked_add(
                    i64::try_from(value.lease_duration_millis)
                        .map_err(|_| AuthorityError::InvalidInput("lease duration"))?,
                )
                .ok_or(AuthorityError::InvalidInput("lease deadline"))?;
            let not_after = CanonicalInstant::from_unix_millis(not_after_millis)?;
            let transition = lease_resource_transition(value, current_resources)?;
            let lease =
                LeasePolicy::single_use(value.lease_id.clone(), transition.fence, now, not_after);
            (
                1,
                Some(run.last_event_digest.clone()),
                Some(transition),
                ResolvedOperation::GrantLease {
                    command: value.clone(),
                    lease,
                },
            )
        }
        Operation::StartAttempt(value) => {
            let run = require_phase(
                current_run,
                RunPhase::Leased,
                request.operation.event_kind(),
            )?;
            let lease = run
                .lease
                .as_ref()
                .ok_or(AuthorityError::CorruptState("missing lease"))?;
            if now_millis >= lease.not_after.unix_millis() {
                return Err(AuthorityError::LeaseExpired);
            }
            validate_start(value, lease)?;
            let transition = continuation_resource_transition(
                &run.key,
                lease,
                current_resources,
                ResourceDisposition::HeldPreStart,
            )?;
            (
                2,
                Some(run.last_event_digest.clone()),
                Some(transition),
                ResolvedOperation::StartAttempt(value.clone()),
            )
        }
        Operation::PreStartTerminal(value) => {
            plan_pre_start_terminal(current_run, current_resources, value, now_millis)?
        }
        Operation::AttemptTerminal(value) => {
            let run = require_phase(
                current_run,
                RunPhase::AttemptStarted,
                request.operation.event_kind(),
            )?;
            let lease = run
                .lease
                .as_ref()
                .ok_or(AuthorityError::CorruptState("missing lease"))?;
            let attempt = run
                .attempt
                .as_ref()
                .ok_or(AuthorityError::CorruptState("missing attempt"))?;
            if value.start_event_digest != attempt.event_digest
                || value.attempt_id != attempt.attempt_id
            {
                return Err(AuthorityError::InvalidTransition);
            }
            validate_lease_refs(
                &value.lease_event_digest,
                &value.lease_id,
                value.fence,
                lease,
            )?;
            let transition = continuation_resource_transition(
                &run.key,
                lease,
                current_resources,
                ResourceDisposition::AttemptStarted,
            )?;
            (
                3,
                Some(run.last_event_digest.clone()),
                Some(transition),
                ResolvedOperation::AttemptTerminal(value.clone()),
            )
        }
        Operation::FinalWitness(value) => {
            let run = require_phase(
                current_run,
                RunPhase::CandidateSuccessAwaitingFinal,
                request.operation.event_kind(),
            )?;
            validate_final_witness(value, run)?;
            (
                4,
                Some(run.last_event_digest.clone()),
                None,
                ResolvedOperation::FinalWitness(value.clone()),
            )
        }
    };
    let proposal = EventProposal::new(
        &key,
        request.semantic_request_digest.clone(),
        global_sequence,
        run_sequence,
        previous,
        resource_transition,
        operation,
    )?;
    Ok(PlannedMutation {
        proposal,
        prior_run: current_run.cloned(),
        prior_resources: current_resources.to_vec(),
    })
}

pub(crate) fn finalize_transition(
    plan: &PlannedMutation,
    event_digest: &Digest,
) -> Result<(RunState, Vec<ResourceState>), AuthorityError> {
    let key = RunKey {
        project_authority_digest: plan.proposal.project_authority_digest.clone(),
        run_id: plan.proposal.run_id.clone(),
    };
    let mut resources = plan.prior_resources.clone();
    let run = match &plan.proposal.operation {
        ResolvedOperation::Register(value) => RunState {
            key,
            phase: RunPhase::Registered,
            claim_digest: value.claim_digest.clone(),
            registration_event_digest: event_digest.clone(),
            lease: None,
            attempt: None,
            terminal: None,
            final_witness_event_digest: None,
            last_event_digest: event_digest.clone(),
            last_run_sequence: 0,
        },
        ResolvedOperation::GrantLease { command, lease } => {
            let mut run = prior_run(plan)?;
            let transition = plan
                .proposal
                .resource_transition
                .as_ref()
                .ok_or(AuthorityError::CorruptState("lease resource transition"))?;
            run.phase = RunPhase::Leased;
            run.lease = Some(LeaseState {
                event_digest: event_digest.clone(),
                lease_id: lease.lease_id.clone(),
                runner_id: command.runner.runner_id.clone(),
                session_id: command.runner.session_id.clone(),
                boot_id: command.runner.boot_id.clone(),
                runner_enrollment_record_digest: command.runner.enrollment_record_digest.clone(),
                physical_parent_id: command.physical_parent_id.clone(),
                resource_ids: command.resource_ids.clone(),
                conflict_set_digest: transition.conflict_set_digest.clone(),
                fence: transition.fence,
                service_issued_at: lease.service_issued_at.clone(),
                not_after: lease.not_after.clone(),
                pre_review: command.pre_review.clone(),
            });
            run.last_event_digest = event_digest.clone();
            run.last_run_sequence = 1;
            resources = transition_resources(
                transition,
                &run.key,
                event_digest,
                ResourceDisposition::HeldPreStart,
            );
            run
        }
        ResolvedOperation::StartAttempt(value) => {
            let mut run = prior_run(plan)?;
            run.phase = RunPhase::AttemptStarted;
            run.attempt = Some(crate::AttemptState {
                event_digest: event_digest.clone(),
                attempt_id: value.attempt_id.clone(),
            });
            run.last_event_digest = event_digest.clone();
            run.last_run_sequence = 2;
            resources = continue_resources(
                &resources,
                event_digest,
                ResourceDisposition::AttemptStarted,
            );
            run
        }
        ResolvedOperation::PreStartTerminal { command, .. } => {
            let mut run = prior_run(plan)?;
            run.phase = RunPhase::PreStartTerminal;
            run.terminal = Some(TerminalState::PreStart {
                event_digest: event_digest.clone(),
                stage: command.terminal_stage,
            });
            run.last_event_digest = event_digest.clone();
            run.last_run_sequence = plan.proposal.run_sequence;
            if let Some(disposition) = &command.resource_disposition {
                resources = continue_resources(&resources, event_digest, disposition.kind);
            }
            run
        }
        ResolvedOperation::AttemptTerminal(value) => {
            let mut run = prior_run(plan)?;
            run.phase = if value.outcome_code == AttemptOutcome::CandidateComplete {
                RunPhase::CandidateSuccessAwaitingFinal
            } else {
                RunPhase::FailedFinalOptional
            };
            run.terminal = Some(TerminalState::Attempt {
                event_digest: event_digest.clone(),
                outcome: value.outcome_code,
                output_envelope_digest: value.output_envelope_digest.clone(),
                capture_record_digest: value.capture_record_digest.clone(),
                final_state_digest: value.final_state_digest.clone(),
            });
            run.last_event_digest = event_digest.clone();
            run.last_run_sequence = 3;
            resources =
                continue_resources(&resources, event_digest, value.resource_disposition.kind);
            run
        }
        ResolvedOperation::FinalWitness(_) => {
            let mut run = prior_run(plan)?;
            run.phase = RunPhase::FinalWitnessed;
            run.final_witness_event_digest = Some(event_digest.clone());
            run.last_event_digest = event_digest.clone();
            run.last_run_sequence = 4;
            run
        }
    };
    run.validate()?;
    for resource in &resources {
        resource.validate()?;
    }
    Ok((run, resources))
}

fn require_phase(
    current: Option<&RunState>,
    expected: RunPhase,
    requested: crate::EventKind,
) -> Result<&RunState, AuthorityError> {
    let run = current.ok_or(AuthorityError::UnknownRun)?;
    if run.phase != expected {
        return changed_or_invalid(run, requested);
    }
    Ok(run)
}

fn changed_or_invalid<T>(run: &RunState, requested: crate::EventKind) -> Result<T, AuthorityError> {
    let occupied = match requested {
        crate::EventKind::ClaimRegistered => true,
        crate::EventKind::RunnerLeaseGranted => run.lease.is_some(),
        crate::EventKind::AttemptStartCommitted => run.attempt.is_some(),
        crate::EventKind::RunTerminal | crate::EventKind::AttemptTerminal => run.terminal.is_some(),
        crate::EventKind::FinalWitness => run.final_witness_event_digest.is_some(),
    };
    Err(if occupied {
        AuthorityError::ChangedRequest
    } else {
        AuthorityError::InvalidTransition
    })
}

fn plan_pre_start_terminal(
    current: Option<&RunState>,
    resources: &[ResourceState],
    command: &crate::PreStartTerminal,
    now_millis: i64,
) -> Result<
    (
        u64,
        Option<Digest>,
        Option<ResourceTransition>,
        ResolvedOperation,
    ),
    AuthorityError,
> {
    let run = current.ok_or(AuthorityError::UnknownRun)?;
    if command.registration_event_digest != run.registration_event_digest {
        return Err(AuthorityError::InvalidTransition);
    }
    match command.terminal_stage {
        RunTerminalStage::Registration | RunTerminalStage::PreLease => {
            if run.phase != RunPhase::Registered {
                return changed_or_invalid(run, crate::EventKind::RunTerminal);
            }
            Ok((
                1,
                Some(run.last_event_digest.clone()),
                None,
                ResolvedOperation::PreStartTerminal {
                    command: command.clone(),
                    lease_event_digest: None,
                    lease_id: None,
                    fence: None,
                },
            ))
        }
        RunTerminalStage::LeasedPreStart => {
            if run.phase != RunPhase::Leased {
                return changed_or_invalid(run, crate::EventKind::RunTerminal);
            }
            let lease = run
                .lease
                .as_ref()
                .ok_or(AuthorityError::CorruptState("missing lease"))?;
            if command.outcome_code == RunTerminalOutcome::LeasedPreStartExpired
                && now_millis < lease.not_after.unix_millis()
            {
                return Err(AuthorityError::LeaseNotExpired);
            }
            if command.outcome_code != RunTerminalOutcome::LeasedPreStartExpired
                && now_millis >= lease.not_after.unix_millis()
            {
                return Err(AuthorityError::LeaseExpired);
            }
            let transition = continuation_resource_transition(
                &run.key,
                lease,
                resources,
                ResourceDisposition::HeldPreStart,
            )?;
            Ok((
                2,
                Some(run.last_event_digest.clone()),
                Some(transition),
                ResolvedOperation::PreStartTerminal {
                    command: command.clone(),
                    lease_event_digest: Some(lease.event_digest.clone()),
                    lease_id: Some(lease.lease_id.clone()),
                    fence: Some(lease.fence),
                },
            ))
        }
    }
}

fn validate_start(value: &crate::StartAttempt, lease: &LeaseState) -> Result<(), AuthorityError> {
    validate_lease_refs(
        &value.lease_event_digest,
        &value.lease_id,
        value.fence,
        lease,
    )?;
    if value.runner_id != lease.runner_id
        || value.session_id != lease.session_id
        || value.boot_id != lease.boot_id
        || value.resource_conflict_set_digest != lease.conflict_set_digest
    {
        return Err(AuthorityError::InvalidTransition);
    }
    Ok(())
}

fn validate_lease_refs(
    event_digest: &Digest,
    lease_id: &crate::OpaqueId,
    fence: u64,
    lease: &LeaseState,
) -> Result<(), AuthorityError> {
    if event_digest != &lease.event_digest || lease_id != &lease.lease_id || fence != lease.fence {
        return Err(AuthorityError::InvalidTransition);
    }
    Ok(())
}

fn validate_final_witness(
    value: &crate::FinalWitness,
    run: &RunState,
) -> Result<(), AuthorityError> {
    let lease = run
        .lease
        .as_ref()
        .ok_or(AuthorityError::CorruptState("missing lease"))?;
    let attempt = run
        .attempt
        .as_ref()
        .ok_or(AuthorityError::CorruptState("missing attempt"))?;
    let terminal = match run.terminal.as_ref() {
        Some(TerminalState::Attempt {
            event_digest,
            outcome: AttemptOutcome::CandidateComplete,
            output_envelope_digest: Some(output),
            capture_record_digest: Some(record),
            final_state_digest: Some(state),
            ..
        }) => (event_digest, output, record, state),
        _ => return Err(AuthorityError::InvalidTransition),
    };
    validate_lease_refs(
        &value.lease_event_digest,
        &value.lease_id,
        value.fence,
        lease,
    )?;
    if value.attempt_terminal_event_digest != *terminal.0
        || value.attempt_id != attempt.attempt_id
        || value.frozen_envelope_digest != *terminal.1
        || value.capture_record_digest != *terminal.2
        || value.final_state_digest != *terminal.3
        || value.post_review.codex_receipt_digest == lease.pre_review.codex_receipt_digest
        || value.post_review.codex_receipt_digest == lease.pre_review.claude_receipt_digest
        || value.post_review.claude_receipt_digest == lease.pre_review.codex_receipt_digest
        || value.post_review.claude_receipt_digest == lease.pre_review.claude_receipt_digest
    {
        return Err(AuthorityError::InvalidTransition);
    }
    Ok(())
}

fn prior_run(plan: &PlannedMutation) -> Result<RunState, AuthorityError> {
    plan.prior_run.clone().ok_or(AuthorityError::UnknownRun)
}
