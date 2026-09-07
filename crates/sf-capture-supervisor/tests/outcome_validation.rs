// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;

use std::sync::Arc;

use common::*;
use sf_capture_supervisor::{
    AttemptOutcome, AuthorityError, AuthorityRequest, AuthorityStore, CleanupEvidence, EventKind,
    ExactResult, InMemoryAuthorityStore, ManualServiceClock, Operation, ProcessDisposition,
    ResourceDisposition,
};

#[test]
fn failure_outcomes_reject_success_shaped_output_sets() {
    let project = project("outcome-matrix");
    let run_id = run("outcome_matrix");
    let lease = event(EventKind::RunnerLeaseGranted, "lease", 2, 1);
    let start = event(EventKind::AttemptStartCommitted, "start", 3, 2);
    let candidate = attempt_terminal_request(
        &project,
        &run_id,
        &lease,
        &start,
        1,
        AttemptOutcome::CandidateComplete,
        "candidate",
    );
    let Operation::AttemptTerminal(base) = candidate.operation else {
        unreachable!()
    };

    let mut process_failed = base.clone();
    process_failed.outcome_code = AttemptOutcome::ProcessFailed;
    process_failed.process_disposition.kind = ProcessDisposition::ExitedNonzero;
    assert_invalid(&project, &run_id, process_failed, "process-failed");

    let mut timeout = base.clone();
    timeout.outcome_code = AttemptOutcome::Timeout;
    timeout.process_disposition.kind = ProcessDisposition::Terminated;
    assert_invalid(&project, &run_id, timeout, "timeout");

    let mut output_invalid = base.clone();
    output_invalid.outcome_code = AttemptOutcome::OutputInvalid;
    assert_invalid(&project, &run_id, output_invalid, "output-invalid");

    let mut internal_failure = base;
    internal_failure.outcome_code = AttemptOutcome::InternalFailure;
    internal_failure.resource_disposition.kind = ResourceDisposition::Quarantined;
    assert_invalid(&project, &run_id, internal_failure, "internal-failure");
}

#[tokio::test]
async fn failed_attempt_with_forensic_outputs_cannot_be_final_witnessed_as_success() {
    let project = project("failed-final");
    let run_id = run("failed_final");
    let store = InMemoryAuthorityStore::new(
        digest("authority:failed-final"),
        Arc::new(ManualServiceClock::new(NOW_MILLIS)),
    );
    let materializer = shared_materializer();
    let register = register_request(&project, &run_id, "register");
    let registration = exact(store.apply(&register, materializer.as_ref()).await.unwrap());
    let lease_request = lease_request(
        &project,
        &run_id,
        &registration,
        resources("parent_failed_final_1", "resource_failed_final_1"),
        "lease",
    );
    let lease = exact(
        store
            .apply(&lease_request, materializer.as_ref())
            .await
            .unwrap(),
    );
    let transition = materializer.last().await.resource_transition.unwrap();
    let start_request = start_request(&project, &run_id, &lease, &transition, "start");
    let start = exact(
        store
            .apply(&start_request, materializer.as_ref())
            .await
            .unwrap(),
    );
    let candidate = attempt_terminal_request(
        &project,
        &run_id,
        &lease,
        &start,
        transition.fence,
        AttemptOutcome::CandidateComplete,
        "terminal",
    );
    let Operation::AttemptTerminal(mut failed) = candidate.operation else {
        unreachable!()
    };
    failed.outcome_code = AttemptOutcome::CleanupFailed;
    failed.resource_disposition.kind = ResourceDisposition::Quarantined;
    failed.cleanup = CleanupEvidence {
        process_cleanup_digest: Some(digest("process-cleanup")),
        egress_cleanup_digest: None,
        resource_cleanup_digest: Some(digest("resource-cleanup")),
    };
    let failed_request = request(
        &project,
        &run_id,
        "failed-terminal",
        Operation::AttemptTerminal(failed),
    );
    let terminal = exact(
        store
            .apply(&failed_request, materializer.as_ref())
            .await
            .unwrap(),
    );
    let final_request = final_witness_request(
        &project,
        &run_id,
        &lease,
        &terminal,
        transition.fence,
        "forbidden-final",
    );
    assert!(matches!(
        store.apply(&final_request, materializer.as_ref()).await,
        Err(AuthorityError::InvalidTransition)
    ));
    assert_eq!(store.snapshot().await.unwrap().event_count, 4);
}

fn assert_invalid(
    project: &sf_capture_supervisor::Digest,
    run_id: &sf_capture_supervisor::OpaqueId,
    command: sf_capture_supervisor::AttemptTerminal,
    label: &str,
) {
    let result = AuthorityRequest::new(
        project.clone(),
        run_id.clone(),
        digest(format!("request:{label}")),
        format!("{{\"label\":\"{label}\"}}\n").into_bytes(),
        Operation::AttemptTerminal(command),
    );
    assert!(matches!(result, Err(AuthorityError::InvalidInput(_))));
}

fn event(kind: EventKind, label: &str, global_sequence: u64, run_sequence: u64) -> ExactResult {
    ExactResult {
        event_kind: kind,
        event_digest: digest(label),
        global_sequence,
        run_sequence,
        event_envelope: b"{}\n".to_vec(),
        response_bytes: b"{}\n".to_vec(),
    }
}
