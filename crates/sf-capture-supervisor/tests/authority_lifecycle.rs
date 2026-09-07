// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;

use std::sync::Arc;

use common::*;
use sf_capture_supervisor::{
    ApplyOutcome, AuthorityError, AuthorityRequest, AuthorityStore, DeferredCapability,
    InMemoryAuthorityStore, ManualServiceClock, Operation, ResolvedOperation, ResourceDisposition,
    RunPhase, RUN_EVENT_RECORD_KIND_V2, SCHEMA_VERSION_V2, TRANSACTION_KIND_V2,
    UNAVAILABLE_CAPABILITIES,
};

#[tokio::test]
async fn full_single_attempt_lifecycle_recovers_exact_bytes_without_append() {
    let project = project("lifecycle");
    let run_id = run("lifecycle");
    let clock = Arc::new(ManualServiceClock::new(NOW_MILLIS));
    let store = InMemoryAuthorityStore::new(digest("authority:lifecycle"), clock);
    let materializer = shared_materializer();

    let register = register_request(&project, &run_id, "v1");
    let registration = exact(store.apply(&register, materializer.as_ref()).await.unwrap());
    let recovered = store.apply(&register, materializer.as_ref()).await.unwrap();
    assert!(matches!(recovered, ApplyOutcome::Recovered(_)));
    assert_eq!(recovered.result(), &registration);
    assert_eq!(materializer.calls(), 1);
    assert_eq!(
        materializer
            .last()
            .await
            .proposal_digest()
            .unwrap()
            .as_str(),
        "6a6318c8aa3de697bd7ce20babb73b724ca3f5ae031e12595fcae6fee9520b60"
    );

    let lease_request = lease_request(
        &project,
        &run_id,
        &registration,
        resources("parent_numa_0001", "resource_cpu_0001"),
        "v1",
    );
    let lease = exact(
        store
            .apply(&lease_request, materializer.as_ref())
            .await
            .unwrap(),
    );
    let lease_proposal = materializer.last().await;
    assert_eq!(lease_proposal.schema_version, SCHEMA_VERSION_V2);
    assert_eq!(lease_proposal.transaction_kind, TRANSACTION_KIND_V2);
    assert_eq!(lease_proposal.record_kind, RUN_EVENT_RECORD_KIND_V2);
    let transition = lease_proposal.resource_transition.clone().unwrap();
    assert_eq!(transition.fence, 1);
    match lease_proposal.operation {
        ResolvedOperation::GrantLease { lease, .. } => {
            assert_eq!(lease.max_attempts, 1);
            assert!(!lease.renew);
            assert!(!lease.release_for_reuse);
            assert!(!lease.reassign);
            assert!(!lease.reclaim);
            assert!(!lease.retry);
        }
        other => panic!("unexpected lease proposal: {other:?}"),
    }

    let start_command = start_request(&project, &run_id, &lease, &transition, "v1");
    let start = exact(
        store
            .apply(&start_command, materializer.as_ref())
            .await
            .unwrap(),
    );
    let changed_start = start_request(&project, &run_id, &lease, &transition, "v2");
    assert!(matches!(
        store.apply(&changed_start, materializer.as_ref()).await,
        Err(AuthorityError::ChangedRequest)
    ));

    let terminal_request = attempt_terminal_request(
        &project,
        &run_id,
        &lease,
        &start,
        transition.fence,
        sf_capture_supervisor::AttemptOutcome::CandidateComplete,
        "v1",
    );
    let terminal = exact(
        store
            .apply(&terminal_request, materializer.as_ref())
            .await
            .unwrap(),
    );
    let final_request =
        final_witness_request(&project, &run_id, &lease, &terminal, transition.fence, "v1");
    let final_result = exact(
        store
            .apply(&final_request, materializer.as_ref())
            .await
            .unwrap(),
    );

    let recovered = store.recover_exact(&final_request).await.unwrap();
    assert_eq!(recovered, final_result);
    assert_eq!(recovered.event_envelope, final_result.event_envelope);
    assert_eq!(recovered.response_bytes, final_result.response_bytes);
    assert!(store
        .apply(&final_request, materializer.as_ref())
        .await
        .unwrap()
        .was_recovered());

    let snapshot = store.snapshot().await.unwrap();
    assert_eq!(snapshot.event_count, 5);
    assert_eq!(snapshot.next_global_sequence, 6);
    assert_eq!(
        snapshot.runs,
        vec![(
            sf_capture_supervisor::RunKey {
                project_authority_digest: project,
                run_id,
            },
            RunPhase::FinalWitnessed,
        )]
    );
    assert_eq!(snapshot.resources.len(), 2);
    assert!(snapshot.resources.iter().all(|resource| {
        resource.fence == 1
            && resource.disposition == ResourceDisposition::ReleasedAfterCleanup
            && resource.owner.is_none()
    }));
    assert_eq!(materializer.calls(), 5);
}

#[tokio::test]
async fn changed_exact_request_and_unknown_run_fail_closed() {
    let project = project("fail-closed");
    let run_id = run("fail_closed");
    let store = InMemoryAuthorityStore::new(
        digest("authority:fail-closed"),
        Arc::new(ManualServiceClock::new(NOW_MILLIS)),
    );
    let materializer = shared_materializer();
    let request = register_request(&project, &run_id, "original");
    store.apply(&request, materializer.as_ref()).await.unwrap();

    let changed = AuthorityRequest::new(
        request.project_authority_digest.clone(),
        request.run_id.clone(),
        request.semantic_request_digest.clone(),
        b"{\"changed\":true}\n".to_vec(),
        request.operation.clone(),
    )
    .unwrap();
    assert!(matches!(
        store.apply(&changed, materializer.as_ref()).await,
        Err(AuthorityError::ChangedRequest)
    ));
    let mut forged = request.clone();
    forged.canonical_request = b"{\"forged\":true}\n".to_vec();
    assert!(matches!(
        store.apply(&forged, materializer.as_ref()).await,
        Err(AuthorityError::InvalidInput("request binding"))
    ));
    assert_eq!(store.snapshot().await.unwrap().event_count, 1);

    let unknown_run = run("unknown_1");
    let fake_registration = sf_capture_supervisor::ExactResult {
        event_kind: sf_capture_supervisor::EventKind::ClaimRegistered,
        event_digest: digest("missing-registration"),
        global_sequence: 1,
        run_sequence: 0,
        event_envelope: b"{}\n".to_vec(),
        response_bytes: b"{}\n".to_vec(),
    };
    let lease = lease_request(
        &project,
        &unknown_run,
        &fake_registration,
        resources("parent_numa_0002", "resource_cpu_0002"),
        "unknown",
    );
    assert!(matches!(
        store.apply(&lease, materializer.as_ref()).await,
        Err(AuthorityError::UnknownRun)
    ));
    assert!(matches!(
        store.recover_exact(&lease).await,
        Err(AuthorityError::ExactResultMissing)
    ));
}

#[test]
fn deferred_capabilities_are_explicitly_unavailable() {
    assert_eq!(
        UNAVAILABLE_CAPABILITIES,
        &[
            DeferredCapability::HttpMtlsTransport,
            DeferredCapability::ServiceSigner,
            DeferredCapability::TransparencyLog,
            DeferredCapability::CheckpointWitnessQuorum,
            DeferredCapability::SemanticWitnessQuorum,
            DeferredCapability::ControlledRunnerLaunch,
        ]
    );
    let operation = Operation::Register(sf_capture_supervisor::RegisterClaim {
        claim_key_digest: digest("key"),
        claim_digest: digest("claim"),
        rooted_claim_validation_digest: digest("root"),
    });
    assert_eq!(operation.event_kind().as_str(), "claim-registered-v2");
}
