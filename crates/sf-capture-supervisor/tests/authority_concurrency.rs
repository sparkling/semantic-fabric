// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;

use std::sync::Arc;

use common::*;
use sf_capture_supervisor::{
    ApplyOutcome, AttemptOutcome, AuthorityError, AuthorityStore, ExactResult,
    InMemoryAuthorityStore, ManualServiceClock, ResourceDisposition, ResourceTransition, RunPhase,
};

#[tokio::test]
async fn concurrent_identical_registration_commits_once_and_recovers_once() {
    let store = Arc::new(InMemoryAuthorityStore::new(
        digest("authority:duplicate-race"),
        Arc::new(ManualServiceClock::new(NOW_MILLIS)),
    ));
    let materializer = shared_materializer();
    let request = register_request(&project("duplicate-race"), &run("duplicate_race"), "same");

    let (left, right) = tokio::join!(
        store.apply(&request, materializer.as_ref()),
        store.apply(&request, materializer.as_ref()),
    );
    let (left, right) = (left.unwrap(), right.unwrap());
    assert_ne!(left.was_recovered(), right.was_recovered());
    assert_eq!(left.result(), right.result());
    assert!(matches!(
        left,
        ApplyOutcome::Committed(_) | ApplyOutcome::Recovered(_)
    ));
    assert_eq!(materializer.calls(), 1);
    assert_eq!(store.snapshot().await.unwrap().event_count, 1);
}

#[tokio::test]
async fn concurrent_changed_registration_occupies_one_immutable_slot() {
    let store = Arc::new(InMemoryAuthorityStore::new(
        digest("authority:changed-race"),
        Arc::new(ManualServiceClock::new(NOW_MILLIS)),
    ));
    let materializer = shared_materializer();
    let project = project("changed-race");
    let run_id = run("changed_race");
    let left = register_request(&project, &run_id, "left");
    let right = register_request(&project, &run_id, "right");

    let (left, right) = tokio::join!(
        store.apply(&left, materializer.as_ref()),
        store.apply(&right, materializer.as_ref()),
    );
    let committed = usize::from(left.is_ok()) + usize::from(right.is_ok());
    let rejected = usize::from(matches!(left, Err(AuthorityError::ChangedRequest)))
        + usize::from(matches!(right, Err(AuthorityError::ChangedRequest)));
    assert_eq!((committed, rejected), (1, 1));
    assert_eq!(materializer.calls(), 1);
    assert_eq!(store.snapshot().await.unwrap().event_count, 1);
}

#[tokio::test]
async fn overlapping_conflict_sets_have_one_winner_and_monotonic_fences() {
    let clock = Arc::new(ManualServiceClock::new(NOW_MILLIS));
    let store = Arc::new(InMemoryAuthorityStore::new(
        digest("authority:overlap"),
        clock,
    ));
    let materializer = shared_materializer();
    let project = project("overlap");
    let run_a = run("overlap_a");
    let run_b = run("overlap_b");
    let register_a = register_request(&project, &run_a, "a");
    let register_b = register_request(&project, &run_b, "b");
    let registration_a = exact(
        store
            .apply(&register_a, materializer.as_ref())
            .await
            .unwrap(),
    );
    let registration_b = exact(
        store
            .apply(&register_b, materializer.as_ref())
            .await
            .unwrap(),
    );
    let lease_a = lease_request(
        &project,
        &run_a,
        &registration_a,
        resources("parent_shared_0001", "resource_cpu_a_0001"),
        "a",
    );
    let lease_b = lease_request(
        &project,
        &run_b,
        &registration_b,
        resources("parent_shared_0001", "resource_cpu_b_0001"),
        "b",
    );

    let (result_a, result_b) = tokio::join!(
        store.apply(&lease_a, materializer.as_ref()),
        store.apply(&lease_b, materializer.as_ref()),
    );
    assert_eq!(
        usize::from(result_a.is_ok()) + usize::from(result_b.is_ok()),
        1
    );
    assert_eq!(
        usize::from(matches!(
            &result_a,
            Err(AuthorityError::ResourceUnavailable)
        )) + usize::from(matches!(
            &result_b,
            Err(AuthorityError::ResourceUnavailable)
        )),
        1
    );
    let (winner_run, winner_registration, winner_lease) = match (result_a, result_b) {
        (Ok(outcome), Err(AuthorityError::ResourceUnavailable)) => {
            (&run_a, &registration_a, exact(outcome))
        }
        (Err(AuthorityError::ResourceUnavailable), Ok(outcome)) => {
            (&run_b, &registration_b, exact(outcome))
        }
        _ => unreachable!("one lease must win and one overlap must fail"),
    };
    let winner_proposal = materializer.last().await;
    assert_eq!(
        winner_proposal.resource_transition.as_ref().unwrap().fence,
        1
    );

    let release = prestart_terminal_request(
        &project,
        winner_run,
        winner_registration,
        false,
        "release-winner",
    );
    store.apply(&release, materializer.as_ref()).await.unwrap();

    let run_c = run("overlap_c");
    let register_c = register_request(&project, &run_c, "c");
    let registration_c = exact(
        store
            .apply(&register_c, materializer.as_ref())
            .await
            .unwrap(),
    );
    let lease_c = lease_request(
        &project,
        &run_c,
        &registration_c,
        resources("parent_shared_0001", "resource_cpu_c_0001"),
        "c",
    );
    let next_lease = exact(store.apply(&lease_c, materializer.as_ref()).await.unwrap());
    let next_proposal = materializer.last().await;
    let next_transition = next_proposal.resource_transition.unwrap();
    assert_eq!(next_transition.fence, 2);
    assert_ne!(winner_lease.event_digest, next_lease.event_digest);

    let snapshot = store.snapshot().await.unwrap();
    let parent = snapshot
        .resources
        .iter()
        .find(|resource| resource.resource_id.as_str() == "parent_shared_0001")
        .unwrap();
    assert_eq!(parent.fence, 2);
    assert_eq!(parent.disposition, ResourceDisposition::HeldPreStart);
}

#[tokio::test]
async fn attempt_start_and_preflight_terminal_race_has_exactly_one_successor() {
    let (store, _clock, materializer, project, run_id, registration, lease, transition) =
        leased_fixture("start_terminal").await;
    let start = start_request(&project, &run_id, &lease, &transition, "start");
    let terminal = prestart_terminal_request(&project, &run_id, &registration, false, "terminal");

    let (start, terminal) = tokio::join!(
        store.apply(&start, materializer.as_ref()),
        store.apply(&terminal, materializer.as_ref()),
    );
    assert_eq!(
        usize::from(start.is_ok()) + usize::from(terminal.is_ok()),
        1
    );
    assert_eq!(
        usize::from(matches!(start, Err(AuthorityError::InvalidTransition)))
            + usize::from(matches!(terminal, Err(AuthorityError::InvalidTransition))),
        1
    );
    let snapshot = store.snapshot().await.unwrap();
    assert_eq!(snapshot.event_count, 3);
    assert!(matches!(
        snapshot.runs[0].1,
        RunPhase::AttemptStarted | RunPhase::PreStartTerminal
    ));
}

#[tokio::test]
async fn lease_expiry_and_start_boundary_cannot_authorize_an_attempt() {
    let (store, clock, materializer, project, run_id, registration, lease, transition) =
        leased_fixture("expiry_race").await;
    clock.set(NOW_MILLIS + 1_000);
    let start = start_request(&project, &run_id, &lease, &transition, "late-start");
    let expiry = prestart_terminal_request(&project, &run_id, &registration, true, "expiry");

    let (start, expiry) = tokio::join!(
        store.apply(&start, materializer.as_ref()),
        store.apply(&expiry, materializer.as_ref()),
    );
    assert!(matches!(start, Err(AuthorityError::LeaseExpired)));
    assert!(expiry.is_ok());
    let snapshot = store.snapshot().await.unwrap();
    assert_eq!(snapshot.event_count, 3);
    assert_eq!(snapshot.runs[0].1, RunPhase::PreStartTerminal);
    assert!(snapshot.resources.iter().all(|resource| {
        resource.disposition == ResourceDisposition::ReleasedUnstarted && resource.owner.is_none()
    }));
}

#[tokio::test]
async fn expired_lease_rejects_non_expiry_terminal_classification() {
    let (store, clock, materializer, project, run_id, registration, _lease, _transition) =
        leased_fixture("expired_preflight").await;
    clock.set(NOW_MILLIS + 1_000);
    let preflight =
        prestart_terminal_request(&project, &run_id, &registration, false, "late-preflight");
    assert!(matches!(
        store.apply(&preflight, materializer.as_ref()).await,
        Err(AuthorityError::LeaseExpired)
    ));
    assert_eq!(store.snapshot().await.unwrap().event_count, 2);
}

#[tokio::test]
async fn concurrent_changed_terminal_outcomes_commit_exactly_one_terminal() {
    let (store, _clock, materializer, project, run_id, _registration, lease, transition) =
        leased_fixture("terminal_race").await;
    let start_request = start_request(&project, &run_id, &lease, &transition, "start");
    let start = exact(
        store
            .apply(&start_request, materializer.as_ref())
            .await
            .unwrap(),
    );
    let left = attempt_terminal_request(
        &project,
        &run_id,
        &lease,
        &start,
        transition.fence,
        AttemptOutcome::InternalFailure,
        "left",
    );
    let right = attempt_terminal_request(
        &project,
        &run_id,
        &lease,
        &start,
        transition.fence,
        AttemptOutcome::InternalFailure,
        "right",
    );

    let (left, right) = tokio::join!(
        store.apply(&left, materializer.as_ref()),
        store.apply(&right, materializer.as_ref()),
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    assert_eq!(
        usize::from(matches!(left, Err(AuthorityError::ChangedRequest)))
            + usize::from(matches!(right, Err(AuthorityError::ChangedRequest))),
        1
    );
    let snapshot = store.snapshot().await.unwrap();
    assert_eq!(snapshot.event_count, 4);
    assert_eq!(snapshot.runs[0].1, RunPhase::FailedFinalOptional);
    assert!(snapshot.resources.iter().all(|resource| {
        resource.disposition == ResourceDisposition::Quarantined && resource.owner.is_none()
    }));
}

async fn leased_fixture(
    label: &str,
) -> (
    Arc<InMemoryAuthorityStore>,
    Arc<ManualServiceClock>,
    Arc<RecordingMaterializer>,
    sf_capture_supervisor::Digest,
    sf_capture_supervisor::OpaqueId,
    ExactResult,
    ExactResult,
    ResourceTransition,
) {
    let clock = Arc::new(ManualServiceClock::new(NOW_MILLIS));
    let store = Arc::new(InMemoryAuthorityStore::new(
        digest(format!("authority:{label}")),
        clock.clone(),
    ));
    let materializer = shared_materializer();
    let project = project(label);
    let run_id = run(label);
    let register = register_request(&project, &run_id, "register");
    let registration = exact(store.apply(&register, materializer.as_ref()).await.unwrap());
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
    (
        store,
        clock,
        materializer,
        project,
        run_id,
        registration,
        lease,
        transition,
    )
}
