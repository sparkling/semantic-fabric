// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use common::*;
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use sf_capture_supervisor::{
    ApplyOutcome, AuthorityError, AuthorityStore, Digest, PostgresAuthorityStore, ResolvedOperation,
};
use tokio_postgres::NoTls;

#[tokio::test]
#[ignore = "requires an explicit isolated PostgreSQL Unix-socket test instance"]
async fn postgres_contention_expiry_and_exact_recovery_fail_closed() {
    let pool = isolated_pool();
    PostgresAuthorityStore::migrate(&pool).await.unwrap();
    concurrent_duplicate_and_overlap(&pool).await;
    concurrent_start_and_terminal(&pool).await;
    post_lock_clock_rejects_start_after_expiry(&pool).await;
}

async fn concurrent_duplicate_and_overlap(pool: &Pool) {
    let (store, authority) = store(pool, "overlap").await;
    let materializer = shared_materializer();
    let project = project("pg-contention-overlap");
    let run_a = run("pg_contention_a");
    let register_a = register_request(&project, &run_a, "register-a");
    let (left, right) = tokio::join!(
        store.apply(&register_a, materializer.as_ref()),
        store.apply(&register_a, materializer.as_ref()),
    );
    let (left, right) = (left.unwrap(), right.unwrap());
    assert_ne!(left.was_recovered(), right.was_recovered());
    assert_eq!(left.result(), right.result());
    let registration_a = left.result().clone();

    let run_b = run("pg_contention_b");
    let register_b = register_request(&project, &run_b, "register-b");
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
        resources("parent_pg_shared_01", "resource_pg_cpu_a_01"),
        "lease-a",
    );
    let lease_b = lease_request(
        &project,
        &run_b,
        &registration_b,
        resources("parent_pg_shared_01", "resource_pg_cpu_b_01"),
        "lease-b",
    );
    let (lease_a_result, lease_b_result) = tokio::join!(
        store.apply(&lease_a, materializer.as_ref()),
        store.apply(&lease_b, materializer.as_ref()),
    );
    assert_eq!(
        usize::from(lease_a_result.is_ok()) + usize::from(lease_b_result.is_ok()),
        1
    );
    let failure = if let Err(error) = &lease_a_result {
        error
    } else {
        lease_b_result.as_ref().unwrap_err()
    };
    assert!(matches!(
        failure,
        AuthorityError::ResourceUnavailable | AuthorityError::Postgres(_)
    ));
    assert_redacted(failure, &project, &run_a);
    assert_eq!(event_count(pool, &authority).await, 3);
}

async fn concurrent_start_and_terminal(pool: &Pool) {
    let (store, authority) = store(pool, "start-terminal").await;
    let materializer = shared_materializer();
    let project = project("pg-start-terminal");
    let run_id = run("pg_start_terminal");
    let register = register_request(&project, &run_id, "register");
    let registration = exact(store.apply(&register, materializer.as_ref()).await.unwrap());
    let lease_request = lease_request(
        &project,
        &run_id,
        &registration,
        resources("parent_pg_start_01", "resource_pg_start_01"),
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
    let terminal = prestart_terminal_request(&project, &run_id, &registration, false, "terminal");
    let (start_result, terminal_result) = tokio::join!(
        store.apply(&start, materializer.as_ref()),
        store.apply(&terminal, materializer.as_ref()),
    );
    assert_eq!(
        usize::from(start_result.is_ok()) + usize::from(terminal_result.is_ok()),
        1
    );
    let winner_request = if start_result.is_ok() {
        &start
    } else {
        &terminal
    };
    let winner = if let Ok(outcome) = start_result {
        outcome
    } else {
        terminal_result.unwrap()
    };
    assert_eq!(
        store.recover_exact(winner_request).await.unwrap(),
        *winner.result()
    );
    assert!(matches!(
        store
            .apply(winner_request, materializer.as_ref())
            .await
            .unwrap(),
        ApplyOutcome::Recovered(_)
    ));
    assert_eq!(event_count(pool, &authority).await, 3);
}

async fn post_lock_clock_rejects_start_after_expiry(pool: &Pool) {
    let (store, authority) = store(pool, "post-lock-expiry").await;
    let materializer = shared_materializer();
    let project = project("pg-post-lock-expiry");
    let run_id = run("pg_post_lock_expiry");
    let register = register_request(&project, &run_id, "register");
    let registration = exact(store.apply(&register, materializer.as_ref()).await.unwrap());
    let lease_request = lease_request_with_duration(
        &project,
        &run_id,
        &registration,
        resources("parent_pg_expiry_01", "resource_pg_expiry_01"),
        "lease",
        2_000,
    );
    let lease = exact(
        store
            .apply(&lease_request, materializer.as_ref())
            .await
            .unwrap(),
    );
    let lease_proposal = materializer.last().await;
    let transition = lease_proposal.resource_transition.unwrap();
    let not_after = match lease_proposal.operation {
        ResolvedOperation::GrantLease { lease, .. } => lease.not_after,
        _ => unreachable!(),
    };
    let start = start_request(&project, &run_id, &lease, &transition, "blocked-start");

    let mut blocker = pool.get().await.unwrap();
    let blocker_tx = blocker.transaction().await.unwrap();
    blocker_tx
        .query_one(
            "SELECT next_global_sequence FROM sf_capture_authority_v1.authority_heads \
             WHERE authority_digest = $1 FOR UPDATE",
            &[&authority.as_str()],
        )
        .await
        .unwrap();
    let apply_store = store.clone();
    let apply_materializer = materializer.clone();
    let apply_start = start.clone();
    let blocked = tokio::spawn(async move {
        apply_store
            .apply(&apply_start, apply_materializer.as_ref())
            .await
    });
    wait_for_lock_waiter(pool).await;
    sleep_past(not_after.unix_millis()).await;
    blocker_tx.commit().await.unwrap();

    assert!(matches!(
        blocked.await.unwrap(),
        Err(AuthorityError::LeaseExpired)
    ));
    let expiry = prestart_terminal_request(&project, &run_id, &registration, true, "expiry");
    store.apply(&expiry, materializer.as_ref()).await.unwrap();
    assert_eq!(event_count(pool, &authority).await, 3);
}

async fn store(pool: &Pool, label: &str) -> (Arc<PostgresAuthorityStore>, Digest) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let authority = digest(format!("pg-contention:{label}:{nonce}"));
    let store = Arc::new(
        PostgresAuthorityStore::bind(authority.clone(), pool.clone(), pool.clone())
            .await
            .unwrap(),
    );
    store.provision_authority().await.unwrap();
    (store, authority)
}

async fn event_count(pool: &Pool, authority: &Digest) -> i64 {
    pool.get()
        .await
        .unwrap()
        .query_one(
            "SELECT count(*) FROM sf_capture_authority_v1.events WHERE authority_digest = $1",
            &[&authority.as_str()],
        )
        .await
        .unwrap()
        .get(0)
}

async fn wait_for_lock_waiter(pool: &Pool) {
    for _ in 0..100 {
        let waiting: bool = pool
            .get()
            .await
            .unwrap()
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                 WHERE wait_event_type = 'Lock' AND query LIKE '%authority_heads%FOR UPDATE%')",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        if waiting {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("authority writer did not block on the held head row");
}

async fn sleep_past(not_after_millis: i64) {
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let remaining = not_after_millis.saturating_sub(now).saturating_add(50);
    tokio::time::sleep(Duration::from_millis(
        u64::try_from(remaining).unwrap_or(50),
    ))
    .await;
}

fn assert_redacted(
    error: &AuthorityError,
    project: &Digest,
    run_id: &sf_capture_supervisor::OpaqueId,
) {
    let rendered = error.to_string();
    assert!(!rendered.contains(project.as_str()));
    assert!(!rendered.contains(run_id.as_str()));
    assert!(!rendered.contains("constraint"));
}

fn isolated_pool() -> Pool {
    assert_eq!(
        std::env::var("SF_CAPTURE_SUPERVISOR_TEST_PG_CONFIRM").as_deref(),
        Ok("isolated-no-product-mock")
    );
    let dsn = std::env::var("SF_CAPTURE_SUPERVISOR_TEST_PG_DSN").unwrap();
    assert!(!dsn.to_ascii_lowercase().contains("product"));
    let config: tokio_postgres::Config = dsn.parse().unwrap();
    let manager = Manager::from_config(
        config,
        NoTls,
        ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        },
    );
    Pool::builder(manager).max_size(12).build().unwrap()
}
