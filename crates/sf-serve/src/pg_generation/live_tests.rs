//! Required-live PostgreSQL generation-lifecycle evidence.
//!
//! The dedicated environment variable must name a disposable, project-owned
//! administrator endpoint. Ordinary local tests never inspect any live source.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sf_core::query_control::{QueryControlError, QueryLimits};
use tokio_postgres::{error::SqlState, Client, Config, NoTls};

use super::*;
use crate::backend::BackendKind;
use crate::binding_identity::RuntimeBindingIdentity;
use crate::source::POSTGRES_RELATION_SCOPE_OPTIONS;
use crate::{Backend, IntrospectedSource};

#[path = "live_tests/budget_expiry.rs"]
mod budget_expiry;
#[path = "live_tests/fixture.rs"]
mod fixture;
#[path = "live_tests/request_route.rs"]
mod request_route;
#[path = "live_tests/runtime_role.rs"]
mod runtime_role;
use fixture::*;

const LIVE_TEST_URL: &str = "SF_PG_GENERATION_TEST_URL";

async fn exercise_verified_generation_lifecycle(fixture: Arc<Fixture>) {
    let (backend, generation, binding, source_id) = fixture.generation().await;

    let lease = acquire(&backend, &generation, &binding, source_id)
        .await
        .expect("acquire exact generation");
    let execution = lease.execution_client();
    let first_backend = backend_identity(&execution).await;
    drop(execution);
    lease
        .finish_bounded(&budget())
        .await
        .expect("exact final recheck and rollback");
    budget_expiry::exercise(&backend, &generation, &binding, source_id).await;

    // ACCESS EXCLUSIVE schema replacement cannot cross the held relation set.
    let lease = acquire(&backend, &generation, &binding, source_id)
        .await
        .expect("acquire lock-barrier generation");
    fixture
        .admin
        .batch_execute("SET lock_timeout = '100ms'")
        .await
        .expect("bound DDL lock wait");
    let blocked = fixture
        .admin
        .batch_execute("ALTER TABLE public.child ADD COLUMN blocked integer")
        .await
        .expect_err("ACCESS EXCLUSIVE DDL must be blocked");
    assert_eq!(blocked.code(), Some(&SqlState::LOCK_NOT_AVAILABLE));
    fixture
        .admin
        .batch_execute("SET lock_timeout = 0")
        .await
        .expect("restore DDL session timeout");
    lease
        .finish_bounded(&budget())
        .await
        .expect("release relation locks");
    fixture
        .admin
        .batch_execute("BEGIN; LOCK TABLE public.child IN ACCESS EXCLUSIVE MODE NOWAIT; ROLLBACK")
        .await
        .expect("ACCESS EXCLUSIVE proceeds after verified lease closes");

    // Compatible additive FK DDL may commit while the old repeatable-read
    // generation is open. That request remains coherent; the next generation
    // acquisition observes the successor and rejects the stale expectation.
    let lease = acquire(&backend, &generation, &binding, source_id)
        .await
        .expect("acquire old generation");
    fixture
        .admin
        .batch_execute(
            "ALTER TABLE public.child ADD CONSTRAINT child_alternate_fk \
             FOREIGN KEY (alternate_parent_id) REFERENCES public.parent(id) NOT VALID",
        )
        .await
        .expect("compatible additive DDL completes beside ACCESS SHARE");
    lease
        .finish_bounded(&budget())
        .await
        .expect("old transaction closes as one coherent generation");
    assert!(matches!(
        acquire(&backend, &generation, &binding, source_id).await,
        Err(PgGenerationError::SchemaDrift)
    ));
    fixture
        .admin
        .batch_execute("ALTER TABLE public.child DROP CONSTRAINT child_alternate_fk")
        .await
        .expect("restore candidate schema");

    // A lease-local policy mutation is caught at the final same-backend check,
    // but rollback still cleans the member for the next acquisition.
    let lease = acquire(&backend, &generation, &binding, source_id)
        .await
        .expect("acquire policy-mutation generation");
    let execution = lease.execution_client();
    execution
        .batch_execute("SET LOCAL row_security = off")
        .await
        .expect("mutate test-local policy");
    drop(execution);
    assert!(matches!(
        lease.finish_bounded(&budget()).await,
        Err(PgGenerationError::CapabilityDrift)
    ));
    acquire(&backend, &generation, &binding, source_id)
        .await
        .expect("rolled-back member remains usable")
        .finish_bounded(&budget())
        .await
        .expect("successor exact recheck");

    // Aborting in-flight work and dropping without the closer can never recycle
    // its open transaction.
    let lease = acquire(&backend, &generation, &binding, source_id)
        .await
        .expect("acquire cancellation generation");
    let execution = lease.execution_client();
    let cancelled = backend_identity(&execution).await;
    let blocked = tokio::spawn(async move {
        execution
            .query_one("SELECT pg_catalog.pg_sleep(30)", &[])
            .await
    });
    wait_for_backend(&fixture, cancelled.pid, true).await;
    blocked.abort();
    assert!(blocked
        .await
        .expect_err("query task must abort")
        .is_cancelled());
    drop(lease);
    wait_for_backend(&fixture, cancelled.pid, false).await;

    // The closer refuses to recycle while an execution view is retained; once
    // that last view drops, the still-dirty member is detached.
    let lease = acquire(&backend, &generation, &binding, source_id)
        .await
        .expect("acquire retained-client generation");
    let retained = lease.execution_client();
    let retained_identity = backend_identity(&retained).await;
    assert!(matches!(
        lease.finish_bounded(&budget()).await,
        Err(PgGenerationError::Internal)
    ));
    drop(retained);
    wait_for_backend(&fixture, retained_identity.pid, false).await;

    let lease = acquire(&backend, &generation, &binding, source_id)
        .await
        .expect("pool replaces detached dirty member");
    let execution = lease.execution_client();
    let replacement = backend_identity(&execution).await;
    assert_ne!(cancelled, replacement);
    assert_ne!(retained_identity, replacement);
    assert_ne!(first_backend.pid, 0);
    drop(execution);
    lease
        .finish_bounded(&budget())
        .await
        .expect("close replacement lease");

    drop(backend);
    drop(generation);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a disposable PostgreSQL administrator endpoint"]
async fn verified_generation_lifecycle_is_coherent_and_fail_closed() {
    let spec = std::env::var(LIVE_TEST_URL)
        .expect("SF_PG_GENERATION_TEST_URL is required by the explicit live test");
    let root_config: Config = spec.parse().expect("valid dedicated generation-test URL");
    let fixture = Arc::new(Fixture::create(root_config).await);
    let work_fixture = Arc::clone(&fixture);
    let outcome = tokio::spawn(async move {
        runtime_role::exercise(&work_fixture).await;
        request_route::exercise(&work_fixture).await;
        exercise_verified_generation_lifecycle(work_fixture).await;
    })
    .await;
    let fixture = match Arc::try_unwrap(fixture) {
        Ok(fixture) => fixture,
        Err(_) => panic!("lifecycle task retained its fixture"),
    };
    fixture.cleanup().await;
    match outcome {
        Ok(()) => {}
        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        Err(error) => panic!("verified generation lifecycle task failed: {error}"),
    }
}
