// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use sf_capture_supervisor::{
    ApplyOutcome, AuthorityError, AuthorityStore, CrashPoint, EventMaterializerPort, EventProposal,
    InMemoryAuthorityStore, ManualServiceClock, MaterializedEvent,
};

#[tokio::test]
async fn every_modeled_crash_boundary_is_atomic_and_exactly_recoverable() {
    let points = [
        CrashPoint::BeforeMaterialization,
        CrashPoint::AfterMaterializationBeforePersist,
        CrashPoint::AfterPersistBeforeCommit,
        CrashPoint::AfterCommitBeforeReply,
    ];
    for point in points {
        let label = format!("crash_{point:?}").to_lowercase();
        let (store, materializer, start) = started_fixture(&label).await;
        store.inject_crash_once(point).await;

        let error = store
            .apply(&start, materializer.as_ref())
            .await
            .expect_err("fault is surfaced");
        let after_fault = store.snapshot().await.unwrap();
        if point == CrashPoint::AfterCommitBeforeReply {
            assert!(matches!(error, AuthorityError::CommitOutcomeUnknown));
            assert_eq!(after_fault.event_count, 3);
            let recovered = store.recover_exact(&start).await.unwrap();
            let replay = store.apply(&start, materializer.as_ref()).await.unwrap();
            assert!(matches!(replay, ApplyOutcome::Recovered(_)));
            assert_eq!(replay.result(), &recovered);

            let changed = start_request_from_existing(&start, "changed-after-unknown");
            assert!(matches!(
                store.apply(&changed, materializer.as_ref()).await,
                Err(AuthorityError::ChangedRequest)
            ));
            assert_eq!(store.snapshot().await.unwrap().event_count, 3);
        } else {
            assert!(matches!(error, AuthorityError::InjectedCrash(_)));
            assert_eq!(after_fault.event_count, 2);
            assert!(matches!(
                store.recover_exact(&start).await,
                Err(AuthorityError::ExactResultMissing)
            ));
            let committed = store.apply(&start, materializer.as_ref()).await.unwrap();
            assert!(matches!(committed, ApplyOutcome::Committed(_)));
            assert_eq!(store.snapshot().await.unwrap().event_count, 3);
        }
    }
}

#[tokio::test]
async fn rejected_materialization_never_advances_authority_state() {
    let project = project("bad-materializer");
    let run_id = run("bad_materializer");
    let store = InMemoryAuthorityStore::new(
        digest("authority:bad-materializer"),
        Arc::new(ManualServiceClock::new(NOW_MILLIS)),
    );
    let request = register_request(&project, &run_id, "bad-materializer");

    let error = store.apply(&request, &WrongProposalMaterializer).await;
    assert!(matches!(
        error,
        Err(AuthorityError::MaterializationMismatch("proposal digest"))
    ));
    assert_eq!(store.snapshot().await.unwrap().event_count, 0);
    assert!(matches!(
        store.recover_exact(&request).await,
        Err(AuthorityError::ExactResultMissing)
    ));
}

async fn started_fixture(
    label: &str,
) -> (
    Arc<InMemoryAuthorityStore>,
    Arc<RecordingMaterializer>,
    sf_capture_supervisor::AuthorityRequest,
) {
    let project = project(label);
    let run_id = run(label);
    let store = Arc::new(InMemoryAuthorityStore::new(
        digest(format!("authority:{label}")),
        Arc::new(ManualServiceClock::new(NOW_MILLIS)),
    ));
    let materializer = shared_materializer();
    let registration_request = register_request(&project, &run_id, "register");
    let registration = exact(
        store
            .apply(&registration_request, materializer.as_ref())
            .await
            .unwrap(),
    );
    let lease_request = lease_request(
        &project,
        &run_id,
        &registration,
        resources(
            &format!("parent_{label}_0001"),
            &format!("resource_{label}_0001"),
        ),
        "lease",
    );
    let lease = exact(
        store
            .apply(&lease_request, materializer.as_ref())
            .await
            .unwrap(),
    );
    let transition = materializer.last().await.resource_transition.unwrap();
    let start = start_request(&project, &run_id, &lease, &transition, "start");
    (store, materializer, start)
}

fn start_request_from_existing(
    request: &sf_capture_supervisor::AuthorityRequest,
    label: &str,
) -> sf_capture_supervisor::AuthorityRequest {
    let mut bytes = serde_json::to_vec(&serde_json::json!({
        "label": label,
        "operation": &request.operation,
    }))
    .unwrap();
    bytes.push(b'\n');
    sf_capture_supervisor::AuthorityRequest::new(
        request.project_authority_digest.clone(),
        request.run_id.clone(),
        digest(format!("semantic-request:{label}")),
        bytes,
        request.operation.clone(),
    )
    .unwrap()
}

struct WrongProposalMaterializer;

#[async_trait]
impl EventMaterializerPort for WrongProposalMaterializer {
    async fn materialize(
        &self,
        _proposal: &EventProposal,
    ) -> Result<MaterializedEvent, AuthorityError> {
        Ok(MaterializedEvent {
            proposal_digest: digest("wrong-proposal"),
            event_digest: digest("wrong-event"),
            event_envelope: b"{}\n".to_vec(),
            response_bytes: b"{}\n".to_vec(),
        })
    }
}
