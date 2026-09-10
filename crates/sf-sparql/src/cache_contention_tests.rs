//! The real shard lock remains owned until the controlled operation has returned.
use std::{hash::Hash, sync::mpsc, time::Duration};

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};

use super::*;

pub(super) const QUERY: &str =
    "SELECT ?x WHERE { VALUES ?x { 1 2 3 } FILTER EXISTS { VALUES ?inside { 7 } } }";

pub(super) fn binding() -> CompilerBinding {
    CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), vec![]),
        Dialect::Sqlite,
        Tbox::default(),
        vec![],
        Epoch::default(),
        8,
    )
}

pub(super) fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

pub(super) fn key_work(binding: &CompilerBinding) -> u64 {
    let control = budget(u64::MAX);
    bounded_key::plan_key_with_work_control(
        &crate::parse_query(QUERY).unwrap(),
        binding.scope(),
        CompileProfileId::Uncontrolled,
        &control,
    )
    .unwrap();
    control.consumed(QueryCharge::CompilerWork)
}

/// Release on both success and timeout before asserting, so a blocking regression
/// fails the test instead of leaving the scoped holder permanently stuck.
pub(super) fn while_write_locked<K, V, T>(
    cache: &quick_cache::sync::Cache<K, V>,
    key: &K,
    operation: impl FnOnce() -> T + Send,
) -> T
where
    K: Eq + Hash + Send + Sync,
    V: Clone + Send + Sync,
    T: Send,
{
    std::thread::scope(|threads| {
        let (started, locked) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let (finished, result) = mpsc::channel();
        let holder = threads.spawn(move || {
            assert!(cache
                .remove_if(key, |_| {
                    started.send(()).unwrap();
                    // Sender drop on any unwind also releases this holder.
                    let _ = released.recv();
                    false
                })
                .is_none());
        });
        locked
            .recv_timeout(Duration::from_secs(5))
            .expect("holder must acquire the actual resident shard");
        let worker = threads.spawn(move || {
            let _ = finished.send(operation());
        });
        let observed = result.recv_timeout(Duration::from_secs(2));
        // Even a raw blocking regression can now finish and be joined safely.
        drop(release);
        holder.join().unwrap();
        worker.join().unwrap();
        observed.expect("controlled lookup/insertion waited for a held cache lock")
    })
}

pub(super) struct StopAfterKey {
    pub budget: QueryBudget,
    pub key_work: u64,
    pub cause: QueryControlError,
}

impl QueryControl for StopAfterKey {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.budget.checkpoint()
    }

    fn consume(
        &self,
        charge: QueryCharge,
        amount: u64,
    ) -> std::result::Result<(), QueryControlError> {
        self.budget.consume(charge, amount)?;
        if self.budget.consumed(QueryCharge::CompilerWork) > self.key_work {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }

    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.budget.terminate(cause)
    }
}

#[test]
fn ordinary_contended_lookup_and_publication_finish_before_lock_release() {
    let binding = binding();
    let key = plan_key(&crate::parse_query(QUERY).unwrap(), binding.scope());
    let resident = binding.compile_shared(QUERY).unwrap();
    let control = budget(u64::MAX);
    let compiled = while_write_locked(&binding.cache().inner, &key, || {
        binding.compile_shared_with_work_control(QUERY, &control)
    })
    .unwrap();
    assert!(!Arc::ptr_eq(&resident, &compiled));
    assert_eq!(format!("{resident:?}"), format!("{compiled:?}"));
    assert!(control.consumed(QueryCharge::CompilerWork) > key_work(&binding));
    assert!(Arc::ptr_eq(
        &resident,
        &binding.compile_shared(QUERY).unwrap()
    ));
    assert!(Arc::ptr_eq(
        &resident,
        &binding
            .compile_shared_with_work_control(QUERY, &budget(u64::MAX))
            .unwrap()
    ));
    assert_eq!(binding.cache_len(), 1);
}

#[test]
fn ordinary_contended_miss_preserves_budget_cancellation_and_deadline() {
    let binding = binding();
    let key = plan_key(&crate::parse_query(QUERY).unwrap(), binding.scope());
    let resident = binding.compile_shared(QUERY).unwrap();
    let work = key_work(&binding);
    let short = budget(work);
    let error = while_write_locked(&binding.cache().inner, &key, || {
        binding.compile_shared_with_work_control(QUERY, &short)
    })
    .unwrap_err();
    assert!(matches!(
        error,
        crate::Error::QueryControl(QueryControlError::CompilerWorkExceeded)
    ));
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        let control = StopAfterKey {
            budget: budget(u64::MAX),
            key_work: work,
            cause,
        };
        let error = while_write_locked(&binding.cache().inner, &key, || {
            binding.compile_shared_with_work_control(QUERY, &control)
        })
        .unwrap_err();
        assert!(matches!(error, crate::Error::QueryControl(actual) if actual == cause));
        assert!(control.budget.consumed(QueryCharge::CompilerWork) > work);
    }
    assert!(Arc::ptr_eq(
        &resident,
        &binding.compile_shared(QUERY).unwrap()
    ));
    assert_eq!(binding.cache_len(), 1);
}
