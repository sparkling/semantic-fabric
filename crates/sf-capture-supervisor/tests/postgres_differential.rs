// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use common::*;
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use sf_capture_supervisor::{
    AuthorityStore, EventKind, InMemoryAuthorityStore, ManualServiceClock, PostgresAuthorityStore,
};
use tokio_postgres::NoTls;

#[derive(Debug, Eq, PartialEq)]
struct Transcript {
    event_kinds: Vec<EventKind>,
    run_sequences: Vec<u64>,
    fences: Vec<u64>,
}

#[tokio::test]
#[ignore = "requires an explicit isolated PostgreSQL Unix-socket test instance"]
async fn postgres_serializable_store_matches_reference_transcript() {
    assert_eq!(
        std::env::var("SF_CAPTURE_SUPERVISOR_TEST_PG_CONFIRM").as_deref(),
        Ok("isolated-no-product-mock"),
        "explicit isolated-store confirmation is required"
    );
    let dsn = std::env::var("SF_CAPTURE_SUPERVISOR_TEST_PG_DSN")
        .expect("isolated PostgreSQL DSN is required");
    assert!(
        !dsn.to_ascii_lowercase().contains("product"),
        "Product Mock databases are forbidden"
    );
    let pool = pool(&dsn);
    PostgresAuthorityStore::migrate(&pool).await.unwrap();
    PostgresAuthorityStore::migrate(&pool).await.unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let authority = digest(format!("postgres-differential:{nonce}"));
    let postgres = PostgresAuthorityStore::bind(authority.clone(), pool.clone(), pool)
        .await
        .unwrap();
    postgres.provision_authority().await.unwrap();
    let reference =
        InMemoryAuthorityStore::new(authority, Arc::new(ManualServiceClock::new(NOW_MILLIS)));

    let postgres_result = run_transcript(&postgres, "postgres").await;
    let reference_result = run_transcript(&reference, "postgres").await;
    assert_eq!(postgres_result, reference_result);
}

async fn run_transcript(store: &impl AuthorityStore, label: &str) -> Transcript {
    let materializer = shared_materializer();
    let project = project(label);
    let first_run = run(&format!("{label}_first"));
    let register = register_request(&project, &first_run, "first-register");
    let (first, duplicate) = tokio::join!(
        store.apply(&register, materializer.as_ref()),
        store.apply(&register, materializer.as_ref()),
    );
    let first = first.unwrap();
    let duplicate = duplicate.unwrap();
    assert_ne!(first.was_recovered(), duplicate.was_recovered());
    assert_eq!(first.result(), duplicate.result());
    let registration = first.result().clone();

    let resource_ids = resources("parent_pg_diff_0001", "resource_pg_diff_0001");
    let first_lease_request = lease_request(
        &project,
        &first_run,
        &registration,
        resource_ids.clone(),
        "first-lease",
    );
    let lease = exact(
        store
            .apply(&first_lease_request, materializer.as_ref())
            .await
            .unwrap(),
    );
    let first_transition = materializer.last().await.resource_transition.unwrap();
    let release =
        prestart_terminal_request(&project, &first_run, &registration, false, "first-release");
    store.apply(&release, materializer.as_ref()).await.unwrap();

    let second_run = run(&format!("{label}_second"));
    let second_register = register_request(&project, &second_run, "second-register");
    let second_registration = exact(
        store
            .apply(&second_register, materializer.as_ref())
            .await
            .unwrap(),
    );
    let second_lease = lease_request(
        &project,
        &second_run,
        &second_registration,
        resource_ids,
        "second-lease",
    );
    store
        .apply(&second_lease, materializer.as_ref())
        .await
        .unwrap();
    let second_transition = materializer.last().await.resource_transition.unwrap();
    assert_eq!(
        store.recover_exact(&first_lease_request).await.unwrap(),
        lease
    );

    let proposals = materializer.all().await;
    Transcript {
        event_kinds: proposals.iter().map(|value| value.event_kind).collect(),
        run_sequences: proposals.iter().map(|value| value.run_sequence).collect(),
        fences: vec![first_transition.fence, second_transition.fence],
    }
}

fn pool(dsn: &str) -> Pool {
    let config: tokio_postgres::Config = dsn.parse().expect("valid PostgreSQL DSN");
    let manager = Manager::from_config(
        config,
        NoTls,
        ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        },
    );
    Pool::builder(manager).max_size(8).build().unwrap()
}
