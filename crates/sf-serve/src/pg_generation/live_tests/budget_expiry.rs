//! Expired-request cleanup evidence for the required-live lifecycle test.

use super::*;

pub(super) async fn exercise(
    backend: &Backend,
    generation: &SourceGeneration,
    binding: &RuntimeBindingIdentity,
    source_id: SourceId,
) {
    let request = RequestBudget::after(
        Duration::from_secs(2),
        QueryLimits::new(1_000, 1_000, 1_000, 1_000_000),
    );
    let lease = acquire_with_budget(backend, generation, binding, source_id, &request).await;
    let execution = lease.execution_client();
    let before = backend_identity(&execution).await;
    drop(execution);

    let reserve = Duration::from_millis(300);
    let remaining = request
        .remaining_duration()
        .expect("request budget remains valid")
        .expect("live test request has a deadline");
    assert!(remaining > reserve);
    tokio::time::sleep(remaining - reserve).await;
    assert!(request
        .remaining_duration()
        .is_ok_and(|remaining| remaining.is_some_and(|duration| !duration.is_zero())));
    assert!(matches!(
        lease
            .with_revalidation_delay(Duration::from_millis(600))
            .finish_bounded(&request)
            .await,
        Err(PgGenerationError::Control(
            QueryControlError::DeadlineExceeded
        ))
    ));

    let successor = acquire(backend, generation, binding, source_id)
        .await
        .expect("cleanup after expiry recycles the member");
    let execution = successor.execution_client();
    assert_eq!(backend_identity(&execution).await, before);
    drop(execution);
    successor
        .finish_bounded(&budget())
        .await
        .expect("close successor generation");
}

async fn acquire_with_budget(
    backend: &Backend,
    generation: &SourceGeneration,
    binding: &RuntimeBindingIdentity,
    source_id: SourceId,
    budget: &RequestBudget,
) -> VerifiedPostgresGenerationLease {
    let requirement = generation
        .requirement(backend, binding)
        .expect("valid generation requirement")
        .expect("verified source has a generation requirement");
    let mut leases = VerifiedGenerationLeases::acquire(vec![requirement], budget)
        .await
        .expect("acquire budget-bound generation");
    leases
        .take(source_id, binding)
        .expect("binding-matched generation lease")
        .into_postgres()
        .expect("PostgreSQL generation variant")
}
