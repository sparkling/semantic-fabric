use sf_core::{
    query_control::{QueryCharge, QueryControlError},
    security_context::{RequestAttributesIdentity, SubjectIdentity},
};

use super::*;
use crate::cache::contention_tests::{
    binding, budget, key_work, while_write_locked, StopAfterKey, QUERY,
};

fn context(subject: u8) -> SecurityContext {
    SecurityContext::new(
        PolicySnapshotId::from_digest([1; 32]).unwrap(),
        SubjectIdentity::from_digest([subject; 32]).unwrap(),
        RequestAttributesIdentity::from_digest([3; 32]).unwrap(),
    )
}

fn key(binding: &CompilerBinding, context: &SecurityContext) -> SecurityPlanKey {
    SecurityPlanKey::from_query(
        &crate::parse_query(QUERY).unwrap(),
        binding.scope(),
        CompileProfileId::Uncontrolled,
        context.cache_identity(),
    )
}

#[test]
fn security_contended_lookup_and_publication_preserve_resident_and_partition() {
    let binding = binding();
    let cache = SecurityPlanCache::new(NonZeroUsize::new(8).unwrap());
    let alice = context(2);
    let compiler = binding.for_security_policy(alice.policy_snapshot(), &cache);
    let resident = compiler.compile_shared(&alice, QUERY).unwrap();
    cache.reset_access_counts();
    let control = budget(u64::MAX);
    let compiled = while_write_locked(&cache.inner, &key(&binding, &alice), || {
        compiler.compile_shared_with_work_control(&alice, QUERY, &control)
    })
    .unwrap();
    assert_eq!(
        cache.access_counts(),
        (1, 1),
        "both attempts ran under the held lock"
    );
    assert!(!Arc::ptr_eq(&resident, &compiled));
    assert_eq!(format!("{resident:?}"), format!("{compiled:?}"));
    assert!(control.consumed(QueryCharge::CompilerWork) > key_work(&binding));
    assert!(Arc::ptr_eq(
        &resident,
        &compiler.compile_shared(&alice, QUERY).unwrap()
    ));
    assert!(Arc::ptr_eq(
        &resident,
        &compiler
            .compile_shared_with_work_control(&alice, QUERY, &budget(u64::MAX))
            .unwrap()
    ));
    let bob = compiler
        .compile_shared_with_work_control(&context(4), QUERY, &budget(u64::MAX))
        .unwrap();
    assert!(!Arc::ptr_eq(&resident, &bob));
    assert_eq!(cache.len(), 2);
    assert_eq!(binding.cache_len(), 0);
}

#[test]
fn security_contended_miss_preserves_terminal_causes_and_policy_precedence() {
    let binding = binding();
    let cache = SecurityPlanCache::new(NonZeroUsize::new(8).unwrap());
    let alice = context(2);
    let compiler = binding.for_security_policy(alice.policy_snapshot(), &cache);
    let resident = compiler.compile_shared(&alice, QUERY).unwrap();
    let key = key(&binding, &alice);
    let work = key_work(&binding);
    let short = budget(work);
    cache.reset_access_counts();
    let error = while_write_locked(&cache.inner, &key, || {
        compiler.compile_shared_with_work_control(&alice, QUERY, &short)
    })
    .unwrap_err();
    assert!(matches!(
        error,
        SecurityCompileError::Compiler(crate::Error::QueryControl(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
    assert_eq!(cache.access_counts(), (1, 0));
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        let control = StopAfterKey {
            budget: budget(u64::MAX),
            key_work: work,
            cause,
        };
        cache.reset_access_counts();
        let error = while_write_locked(&cache.inner, &key, || {
            compiler.compile_shared_with_work_control(&alice, QUERY, &control)
        })
        .unwrap_err();
        assert!(
            matches!(error, SecurityCompileError::Compiler(crate::Error::QueryControl(actual)) if actual == cause)
        );
        assert_eq!(cache.access_counts(), (1, 0));
        assert!(control.budget.consumed(QueryCharge::CompilerWork) > work);
        cache.reset_access_counts();
        let mismatched =
            binding.for_security_policy(PolicySnapshotId::from_digest([9; 32]).unwrap(), &cache);
        let error = while_write_locked(&cache.inner, &key, || {
            mismatched.compile_shared_with_work_control(&alice, "invalid query", &control)
        })
        .unwrap_err();
        assert!(matches!(error, SecurityCompileError::PolicyMismatch));
        assert_eq!(cache.access_counts(), (0, 0));
    }
    assert!(Arc::ptr_eq(
        &resident,
        &compiler.compile_shared(&alice, QUERY).unwrap()
    ));
    assert_eq!(cache.len(), 1);
    assert_eq!(binding.cache_len(), 0);
}
