use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::num::NonZeroUsize;
use std::sync::Arc;

use sf_core::security_context::{
    PolicySnapshotId, RequestAttributesIdentity, SecurityContext, SubjectIdentity,
};
use sf_core::{SourceId, SourceMapping};
use sf_sql::Dialect;

use super::{SecurityCachedPlan, SecurityCompileError, SecurityPlanCache, SecurityPlanKey};
use crate::cache::CompileProfileId;
use crate::{CompilerBinding, CompilerSchema, Tbox};

const QUERY: &str = "SELECT * WHERE { ?s ?p ?o }";

fn digest(value: u8) -> [u8; 32] {
    [value; 32]
}

fn policy(value: u8) -> PolicySnapshotId {
    PolicySnapshotId::from_digest(digest(value)).unwrap()
}

fn context(policy_value: u8, subject: u8, attributes: u8) -> SecurityContext {
    SecurityContext::new(
        policy(policy_value),
        SubjectIdentity::from_digest(digest(subject)).unwrap(),
        RequestAttributesIdentity::from_digest(digest(attributes)).unwrap(),
    )
}

fn binding() -> CompilerBinding {
    CompilerBinding::new(
        SourceMapping::new(SourceId::new(0).unwrap(), Vec::new()),
        Dialect::Sqlite,
        Tbox::default(),
        CompilerSchema::from_unverified_observation(Vec::new()),
        8,
    )
}

fn security_cache() -> SecurityPlanCache {
    SecurityPlanCache::new(NonZeroUsize::new(8).unwrap())
}

#[test]
fn identical_security_context_reuses_only_its_own_plan() {
    let binding = binding();
    let cache = security_cache();
    let compiler = binding.for_security_policy(policy(1), &cache);
    let context = context(1, 2, 3);

    let first = compiler.compile_shared(&context, QUERY).unwrap();
    let second = compiler.compile_shared(&context, QUERY).unwrap();

    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(cache.len(), 1);
    assert_eq!(binding.cache_len(), 0);
}

#[test]
fn policy_subject_and_attributes_each_partition_plan_reuse() {
    let binding = binding();
    let cache = security_cache();
    let policy_one = binding.for_security_policy(policy(1), &cache);
    let policy_four = binding.for_security_policy(policy(4), &cache);

    let baseline = policy_one.compile_shared(&context(1, 2, 3), QUERY).unwrap();
    let changed_policy = policy_four
        .compile_shared(&context(4, 2, 3), QUERY)
        .unwrap();
    let changed_subject = policy_one.compile_shared(&context(1, 4, 3), QUERY).unwrap();
    let changed_attributes = policy_one.compile_shared(&context(1, 2, 4), QUERY).unwrap();

    for candidate in [&changed_policy, &changed_subject, &changed_attributes] {
        assert!(!Arc::ptr_eq(&baseline, candidate));
    }
    assert_eq!(cache.len(), 4);
    assert_eq!(binding.cache_len(), 0);
}

#[test]
fn security_scoped_compile_never_reuses_an_unscoped_entry_or_changes_plan() {
    let binding = binding();
    let cache = security_cache();
    let unscoped = binding.compile_shared(QUERY).unwrap();
    let secured = binding
        .for_security_policy(policy(1), &cache)
        .compile_shared(&context(1, 2, 3), QUERY)
        .unwrap();

    assert!(!Arc::ptr_eq(&unscoped, &secured));
    assert_eq!(format!("{unscoped:?}"), format!("{secured:?}"));
    assert_eq!(binding.cache_len(), 1);
    assert_eq!(cache.len(), 1);
}

#[test]
fn policy_mismatch_rejects_before_parse_or_cache_read_or_write() {
    let binding = binding();
    let cache = security_cache();
    binding
        .for_security_policy(policy(1), &cache)
        .compile_shared(&context(1, 2, 3), QUERY)
        .unwrap();
    cache.reset_access_counts();

    let error = binding
        .for_security_policy(policy(1), &cache)
        .compile_shared(&context(4, 2, 3), "not valid SPARQL")
        .unwrap_err();

    assert!(matches!(error, SecurityCompileError::PolicyMismatch));
    assert_eq!(cache.access_counts(), (0, 0));
    assert_eq!(cache.len(), 1);
}

#[derive(Default)]
struct ConstantHasher;

impl Hasher for ConstantHasher {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, _bytes: &[u8]) {}
}

#[test]
fn forced_hash_collision_cannot_cross_security_partitions() {
    let binding = binding();
    let cache = security_cache();
    let query = spargebra::SparqlParser::new().parse_query(QUERY).unwrap();
    let first_context = context(1, 2, 3);
    let second_context = context(1, 4, 3);
    let first_key = SecurityPlanKey::from_canonical_with_hash(
        binding.scope(),
        CompileProfileId::Uncontrolled,
        first_context.cache_identity(),
        7,
        query.to_string(),
    );
    let second_key = SecurityPlanKey::from_canonical_with_hash(
        binding.scope(),
        CompileProfileId::Uncontrolled,
        second_context.cache_identity(),
        7,
        query.to_string(),
    );

    assert_ne!(first_key, second_key);
    // This map's hasher forces every key into one bucket. Exact key equality,
    // rather than the hash alone, must still keep the partitions disjoint.
    let mut collision_map: HashMap<SecurityPlanKey, (), BuildHasherDefault<ConstantHasher>> =
        HashMap::default();
    collision_map.insert(first_key.clone(), ());
    assert!(!collision_map.contains_key(&second_key));

    let plan = binding.compile_uncached_shared(QUERY).unwrap();
    cache.put(
        first_key,
        SecurityCachedPlan::from_shared(
            binding.scope(),
            CompileProfileId::Uncontrolled,
            first_context.cache_identity(),
            plan,
        ),
    );
    assert!(cache.get(&second_key).is_none());
}

#[test]
fn corrupted_cached_identity_fails_closed() {
    let binding = binding();
    let cache = security_cache();
    let request_context = context(1, 2, 3);
    let wrong_context = context(1, 4, 3);
    let query = spargebra::SparqlParser::new().parse_query(QUERY).unwrap();
    let key = SecurityPlanKey::from_query(
        &query,
        binding.scope(),
        CompileProfileId::Uncontrolled,
        request_context.cache_identity(),
    );
    let plan = binding.compile_uncached_shared(QUERY).unwrap();
    cache.put(
        key,
        SecurityCachedPlan::from_shared(
            binding.scope(),
            CompileProfileId::Uncontrolled,
            wrong_context.cache_identity(),
            plan,
        ),
    );

    let error = binding
        .for_security_policy(policy(1), &cache)
        .compile_shared(&request_context, QUERY)
        .unwrap_err();
    assert!(matches!(error, SecurityCompileError::CacheIdentityMismatch));
}

#[test]
fn corrupted_cached_scope_fails_closed() {
    let binding = binding();
    let other_binding = CompilerBinding::new(
        SourceMapping::new(SourceId::new(1).unwrap(), Vec::new()),
        Dialect::Sqlite,
        Tbox::default(),
        CompilerSchema::from_unverified_observation(Vec::new()),
        8,
    );
    let cache = security_cache();
    let request_context = context(1, 2, 3);
    let query = spargebra::SparqlParser::new().parse_query(QUERY).unwrap();
    let key = SecurityPlanKey::from_query(
        &query,
        binding.scope(),
        CompileProfileId::Uncontrolled,
        request_context.cache_identity(),
    );
    cache.put(
        key,
        SecurityCachedPlan::from_shared(
            other_binding.scope(),
            CompileProfileId::Uncontrolled,
            request_context.cache_identity(),
            binding.compile_uncached_shared(QUERY).unwrap(),
        ),
    );

    let error = binding
        .for_security_policy(policy(1), &cache)
        .compile_shared(&request_context, QUERY)
        .unwrap_err();
    assert!(matches!(error, SecurityCompileError::CacheScopeMismatch));
}

#[test]
fn corrupted_cached_profile_fails_closed() {
    let binding = binding();
    let cache = security_cache();
    let request_context = context(1, 2, 3);
    let query = spargebra::SparqlParser::new().parse_query(QUERY).unwrap();
    let key = SecurityPlanKey::from_query(
        &query,
        binding.scope(),
        CompileProfileId::Uncontrolled,
        request_context.cache_identity(),
    );
    cache.put(
        key,
        SecurityCachedPlan::from_shared(
            binding.scope(),
            CompileProfileId::GovernedV1,
            request_context.cache_identity(),
            binding.compile_uncached_shared(QUERY).unwrap(),
        ),
    );

    let error = binding
        .for_security_policy(policy(1), &cache)
        .compile_shared(&request_context, QUERY)
        .unwrap_err();
    assert!(matches!(error, SecurityCompileError::CacheProfileMismatch));
}

#[test]
fn cache_and_error_diagnostics_do_not_render_identity_material() {
    let binding = binding();
    let cache = security_cache();
    let context = context(0xa1, 0xb2, 0xc3);
    let compiler = binding.for_security_policy(policy(0xa1), &cache);
    let key = SecurityPlanKey::from_query(
        &spargebra::SparqlParser::new().parse_query(QUERY).unwrap(),
        binding.scope(),
        CompileProfileId::Uncontrolled,
        context.cache_identity(),
    );
    let cached_plan = SecurityCachedPlan::from_shared(
        binding.scope(),
        CompileProfileId::Uncontrolled,
        context.cache_identity(),
        binding.compile_uncached_shared(QUERY).unwrap(),
    );
    let key_output = format!("{key:?} {cache:?} {cached_plan:?}");
    let mismatch = binding
        .for_security_policy(policy(0xd4), &cache)
        .compile_shared(&context, QUERY)
        .unwrap_err();
    let error_output = format!("{mismatch:?} {mismatch}");

    for secret in ["a1".repeat(32), "b2".repeat(32), "c3".repeat(32)] {
        assert!(!key_output.contains(&secret), "output={key_output}");
        assert!(!error_output.contains(&secret), "output={error_output}");
    }
    assert!(key_output.contains("redacted"));
    assert!(matches!(mismatch, SecurityCompileError::PolicyMismatch));
    assert_eq!(compiler.expected_policy(), context.policy_snapshot());
}

#[test]
fn work_control_preserves_security_partitions_and_never_caches_failed_misses() {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
    let query = "SELECT ?x WHERE { VALUES ?x { 1 2 3 } FILTER EXISTS { VALUES ?inside { 7 } } }";
    let control = |work| QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX));
    let binding = binding();
    let cache = security_cache();
    let compiler = binding.for_security_policy(policy(1), &cache);
    let alice = context(1, 2, 3);
    assert!(compiler
        .compile_shared_with_work_control(&alice, query, &control(0))
        .is_err());
    assert_eq!(cache.len(), 0);
    let paid = control(u64::MAX);
    let first = compiler
        .compile_shared_with_work_control(&alice, query, &paid)
        .unwrap();
    assert!(paid.consumed(QueryCharge::CompilerWork) > 0);
    let hit_control = control(0);
    let hit = compiler
        .compile_shared_with_work_control(&alice, query, &hit_control)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &hit));
    assert_eq!(hit_control.consumed(QueryCharge::CompilerWork), 0);
    assert!(compiler
        .compile_shared_with_work_control(&context(1, 4, 3), query, &control(0))
        .is_err());
    assert!(compiler
        .compile_shared_with_work_control(&context(1, 2, 4), query, &control(0))
        .is_err());
    assert_eq!(cache.len(), 1);
    assert_eq!(binding.cache_len(), 0);
}

#[test]
fn policy_mismatch_precedes_cancelled_control_and_cancelled_hits_cannot_escape() {
    use sf_core::query_control::{QueryBudget, QueryControlError, QueryLimits};
    let binding = binding();
    let cache = security_cache();
    let compiler = binding.for_security_policy(policy(1), &cache);
    let context = context(1, 2, 3);
    compiler.compile_shared(&context, QUERY).unwrap();
    let control = QueryBudget::new(QueryLimits::new(0, 0, 0, 0));
    control.terminate(QueryControlError::Cancelled);
    cache.reset_access_counts();
    assert!(matches!(
        compiler.compile_shared_with_work_control(&context, QUERY, &control),
        Err(SecurityCompileError::Compiler(crate::Error::QueryControl(
            QueryControlError::Cancelled
        )))
    ));
    assert_eq!(cache.access_counts(), (0, 0));
    assert!(matches!(
        binding
            .for_security_policy(policy(2), &cache)
            .compile_shared_with_work_control(&context, "invalid query", &control),
        Err(SecurityCompileError::PolicyMismatch)
    ));
    assert_eq!(cache.access_counts(), (0, 0));
}
