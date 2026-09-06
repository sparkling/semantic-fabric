// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::{
    exact_match,
    planner::{finalize_transition, plan_transition},
    validate_materialized, ApplyOutcome, AuthorityError, AuthorityRequest, AuthorityStore, Digest,
    EventMaterializerPort, ExactResult, OpaqueId, Operation, ResourceState, RunKey, RunPhase,
    RunState, ServiceClock, StoredEvent,
};

/// Reference-model boundary only; these are not database restart probes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CrashPoint {
    BeforeMaterialization,
    AfterMaterializationBeforePersist,
    AfterPersistBeforeCommit,
    AfterCommitBeforeReply,
}

impl CrashPoint {
    fn label(self) -> &'static str {
        match self {
            Self::BeforeMaterialization => "before-materialization",
            Self::AfterMaterializationBeforePersist => "after-materialization-before-persist",
            Self::AfterPersistBeforeCommit => "after-persist-before-commit",
            Self::AfterCommitBeforeReply => "after-commit-before-reply",
        }
    }
}

#[derive(Debug)]
pub struct ManualServiceClock {
    unix_millis: AtomicI64,
}

impl ManualServiceClock {
    pub fn new(unix_millis: i64) -> Self {
        Self {
            unix_millis: AtomicI64::new(unix_millis),
        }
    }

    pub fn set(&self, unix_millis: i64) {
        self.unix_millis.store(unix_millis, Ordering::SeqCst);
    }
}

impl ServiceClock for ManualServiceClock {
    fn now_unix_millis(&self) -> Result<i64, AuthorityError> {
        Ok(self.unix_millis.load(Ordering::SeqCst))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferenceSnapshot {
    pub next_global_sequence: u64,
    pub event_count: usize,
    pub runs: Vec<(RunKey, RunPhase)>,
    pub resources: Vec<ResourceState>,
}

#[derive(Clone, Default)]
struct MemoryState {
    next_global_sequence: u64,
    runs: BTreeMap<RunKey, RunState>,
    resources: BTreeMap<OpaqueId, ResourceState>,
    events: BTreeMap<u64, StoredEvent>,
    exact: BTreeMap<(Digest, Digest), u64>,
}

/// Deterministic reference model for differential and fault-injection tests.
/// It is not a production persistence adapter.
pub struct InMemoryAuthorityStore {
    authority_digest: Digest,
    state: Mutex<MemoryState>,
    next_crash: Mutex<Option<CrashPoint>>,
    clock: Arc<dyn ServiceClock>,
}

impl InMemoryAuthorityStore {
    pub fn new(authority_digest: Digest, clock: Arc<dyn ServiceClock>) -> Self {
        Self {
            authority_digest,
            state: Mutex::new(MemoryState {
                next_global_sequence: 1,
                ..MemoryState::default()
            }),
            next_crash: Mutex::new(None),
            clock,
        }
    }

    pub fn authority_digest(&self) -> &Digest {
        &self.authority_digest
    }

    pub async fn inject_crash_once(&self, point: CrashPoint) {
        *self.next_crash.lock().await = Some(point);
    }

    pub async fn snapshot(&self) -> Result<ReferenceSnapshot, AuthorityError> {
        let state = self.state.lock().await;
        validate_memory_state(&state)?;
        Ok(ReferenceSnapshot {
            next_global_sequence: state.next_global_sequence,
            event_count: state.events.len(),
            runs: state
                .runs
                .iter()
                .map(|(key, run)| (key.clone(), run.phase))
                .collect(),
            resources: state.resources.values().cloned().collect(),
        })
    }

    async fn take_crash(&self) -> Option<CrashPoint> {
        self.next_crash.lock().await.take()
    }
}

#[async_trait]
impl AuthorityStore for InMemoryAuthorityStore {
    async fn apply(
        &self,
        request: &AuthorityRequest,
        materializer: &dyn EventMaterializerPort,
    ) -> Result<ApplyOutcome, AuthorityError> {
        request.validate()?;
        let crash = self.take_crash().await;
        let mut state = self.state.lock().await;
        validate_memory_state(&state)?;
        if let Some(stored) = lookup_exact(&state, request)? {
            return Ok(ApplyOutcome::Recovered(exact_match(stored, request)?));
        }
        let key = run_key(request);
        let current_run = state.runs.get(&key);
        let resources = resource_snapshot(&state, request, current_run);
        let now_millis = self.clock.now_unix_millis()?;
        let plan = plan_transition(
            request,
            current_run,
            &resources,
            state.next_global_sequence,
            now_millis,
        )?;
        fail_at(crash, CrashPoint::BeforeMaterialization)?;
        let materialized = materializer.materialize(&plan.proposal).await?;
        let stored = validate_materialized(request, &plan.proposal, materialized)?;
        fail_at(crash, CrashPoint::AfterMaterializationBeforePersist)?;
        let (next_run, next_resources) = finalize_transition(&plan, &stored.event_digest)?;

        let mut staged = state.clone();
        stage_mutation(&mut staged, stored.clone(), next_run, next_resources)?;
        fail_at(crash, CrashPoint::AfterPersistBeforeCommit)?;
        validate_memory_state(&staged)?;
        *state = staged;
        if crash == Some(CrashPoint::AfterCommitBeforeReply) {
            return Err(AuthorityError::CommitOutcomeUnknown);
        }
        Ok(ApplyOutcome::Committed(stored.exact_result()))
    }

    async fn recover_exact(
        &self,
        request: &AuthorityRequest,
    ) -> Result<ExactResult, AuthorityError> {
        request.validate()?;
        let state = self.state.lock().await;
        let stored = lookup_exact(&state, request)?.ok_or(AuthorityError::ExactResultMissing)?;
        exact_match(stored, request)
    }
}

fn run_key(request: &AuthorityRequest) -> RunKey {
    RunKey {
        project_authority_digest: request.project_authority_digest.clone(),
        run_id: request.run_id.clone(),
    }
}

fn lookup_exact<'a>(
    state: &'a MemoryState,
    request: &AuthorityRequest,
) -> Result<Option<&'a StoredEvent>, AuthorityError> {
    let key = (
        request.project_authority_digest.clone(),
        request.semantic_request_digest.clone(),
    );
    let Some(sequence) = state.exact.get(&key) else {
        return Ok(None);
    };
    state
        .events
        .get(sequence)
        .map(Some)
        .ok_or(AuthorityError::CorruptState("exact-result index"))
}

fn resource_snapshot(
    state: &MemoryState,
    request: &AuthorityRequest,
    run: Option<&RunState>,
) -> Vec<ResourceState> {
    let ids = match &request.operation {
        Operation::GrantLease(value) => Some(&value.resource_ids),
        Operation::StartAttempt(_)
        | Operation::PreStartTerminal(_)
        | Operation::AttemptTerminal(_) => {
            run.and_then(|value| value.lease.as_ref().map(|lease| &lease.resource_ids))
        }
        Operation::Register(_) | Operation::FinalWitness(_) => None,
    };
    ids.into_iter()
        .flatten()
        .filter_map(|id| state.resources.get(id).cloned())
        .collect()
}

fn stage_mutation(
    state: &mut MemoryState,
    stored: StoredEvent,
    run: RunState,
    resources: Vec<ResourceState>,
) -> Result<(), AuthorityError> {
    let global_sequence = stored.global_sequence;
    let exact_key = (
        stored.project_authority_digest.clone(),
        stored.semantic_request_digest.clone(),
    );
    if global_sequence != state.next_global_sequence
        || state.events.contains_key(&global_sequence)
        || state.exact.contains_key(&exact_key)
    {
        return Err(AuthorityError::CorruptState("event uniqueness"));
    }
    state.next_global_sequence = state
        .next_global_sequence
        .checked_add(1)
        .ok_or(AuthorityError::CorruptState("global sequence overflow"))?;
    state.exact.insert(exact_key, global_sequence);
    state.events.insert(global_sequence, stored);
    state.runs.insert(run.key.clone(), run);
    for resource in resources {
        state
            .resources
            .insert(resource.resource_id.clone(), resource);
    }
    Ok(())
}

fn validate_memory_state(state: &MemoryState) -> Result<(), AuthorityError> {
    if state.next_global_sequence == 0 || state.events.len() != state.exact.len() {
        return Err(AuthorityError::CorruptState("memory store cardinality"));
    }
    for (sequence, event) in &state.events {
        event.validate()?;
        if event.global_sequence != *sequence || *sequence >= state.next_global_sequence {
            return Err(AuthorityError::CorruptState("global event sequence"));
        }
        let exact_key = (
            event.project_authority_digest.clone(),
            event.semantic_request_digest.clone(),
        );
        if state.exact.get(&exact_key) != Some(sequence) {
            return Err(AuthorityError::CorruptState("exact-result reverse index"));
        }
    }
    for run in state.runs.values() {
        run.validate()?;
    }
    for resource in state.resources.values() {
        resource.validate()?;
    }
    Ok(())
}

fn fail_at(actual: Option<CrashPoint>, expected: CrashPoint) -> Result<(), AuthorityError> {
    if actual == Some(expected) {
        return Err(AuthorityError::InjectedCrash(expected.label()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_memory_state_fails_closed() {
        let mut state = MemoryState {
            next_global_sequence: 0,
            ..MemoryState::default()
        };
        assert!(matches!(
            validate_memory_state(&state),
            Err(AuthorityError::CorruptState(_))
        ));
        state.next_global_sequence = 1;
        state
            .exact
            .insert((Digest::sha256("p"), Digest::sha256("r")), 1);
        assert!(matches!(
            validate_memory_state(&state),
            Err(AuthorityError::CorruptState(_))
        ));

        state.exact.clear();
        state.resources.insert(
            OpaqueId::parse("resource_invalid_0001").unwrap(),
            ResourceState {
                resource_id: OpaqueId::parse("resource_invalid_0001").unwrap(),
                fence: 0,
                last_event_digest: Digest::sha256("event"),
                disposition: crate::ResourceDisposition::ReleasedUnstarted,
                owner: None,
            },
        );
        assert!(matches!(
            validate_memory_state(&state),
            Err(AuthorityError::CorruptState("resource state"))
        ));

        state.resources.clear();
        let terminal_digest = Digest::sha256("terminal");
        let key = RunKey {
            project_authority_digest: Digest::sha256("project"),
            run_id: OpaqueId::parse("run_invalid_0001").unwrap(),
        };
        state.runs.insert(
            key.clone(),
            RunState {
                key,
                phase: RunPhase::PreStartTerminal,
                claim_digest: Digest::sha256("claim"),
                registration_event_digest: Digest::sha256("registration"),
                lease: None,
                attempt: None,
                terminal: Some(crate::TerminalState::PreStart {
                    event_digest: terminal_digest.clone(),
                    stage: crate::RunTerminalStage::LeasedPreStart,
                }),
                final_witness_event_digest: None,
                last_event_digest: terminal_digest,
                last_run_sequence: 2,
            },
        );
        assert!(matches!(
            validate_memory_state(&state),
            Err(AuthorityError::CorruptState("run state"))
        ));
    }

    #[test]
    fn exact_event_kinds_are_closed() {
        assert_eq!(
            crate::EventKind::ClaimRegistered.as_str(),
            "claim-registered-v2"
        );
        assert_eq!(
            crate::EventKind::FinalWitness.as_str(),
            "capture-final-witness-v2"
        );
    }
}
