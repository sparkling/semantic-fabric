use super::*;

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

#[test]
fn cache_identity_security_raw_and_controlled_reuse_without_partition_leaks() {
    for query in [
        "SELECT (COUNT(*) AS ?n) WHERE { VALUES ?x { 1 1 2 } }",
        "DESCRIBE <urn:a>",
    ] {
        for controlled_first in [false, true] {
            let binding = binding();
            let cache = security_cache();
            let compiler = binding.for_security_policy(policy(1), &cache);
            let identity = context(1, 2, 3);
            let first = if controlled_first {
                compiler
                    .compile_shared_with_work_control(&identity, query, &budget(u64::MAX))
                    .unwrap()
            } else {
                compiler.compile_shared(&identity, query).unwrap()
            };
            let paid = budget(u64::MAX);
            let second = compiler
                .compile_shared_with_work_control(&identity, query, &paid)
                .unwrap();
            assert!(paid.consumed(QueryCharge::CompilerWork) > 0);
            assert!(Arc::ptr_eq(&first, &second));
            assert!(Arc::ptr_eq(
                &first,
                &compiler.compile_shared(&identity, query).unwrap()
            ));
            assert_eq!(cache.len(), 1);
            assert_eq!(binding.cache_len(), 0);
            assert!(compiler
                .compile_shared_with_work_control(&identity, query, &budget(0))
                .is_err());
            assert!(Arc::ptr_eq(
                &first,
                &compiler.compile_shared(&identity, query).unwrap()
            ));
            for other in [context(1, 4, 3), context(1, 2, 4)] {
                let plan = compiler
                    .compile_shared_with_work_control(&other, query, &budget(u64::MAX))
                    .unwrap();
                assert!(!Arc::ptr_eq(&first, &plan));
            }
            let other_policy = binding
                .for_security_policy(policy(4), &cache)
                .compile_shared(&context(4, 2, 3), query)
                .unwrap();
            let unscoped = binding.compile_shared(query).unwrap();
            assert!(!Arc::ptr_eq(&first, &other_policy));
            assert!(!Arc::ptr_eq(&first, &unscoped));
            assert_eq!(cache.len(), 4);
        }
    }
}

#[test]
fn cache_identity_security_canonical_equality_survives_forced_hash_collision() {
    let binding = binding();
    let query = |text| spargebra::SparqlParser::new().parse_query(text).unwrap();
    let count = "SELECT (COUNT(*) AS ?n) WHERE { VALUES ?x { 1 2 } }";
    let mut key = SecurityPlanKey::from_query(
        &query(count),
        binding.scope(),
        CompileProfileId::Uncontrolled,
        context(1, 2, 3).cache_identity(),
    );
    let mut same = SecurityPlanKey::from_query(
        &query(count),
        binding.scope(),
        CompileProfileId::Uncontrolled,
        context(1, 2, 3).cache_identity(),
    );
    key.structural_hash = 7;
    same.structural_hash = 7;
    assert_eq!(key, same);
    for (text, identity, profile) in [
        (
            "SELECT (COUNT(DISTINCT ?x) AS ?n) WHERE { VALUES ?x { 1 2 } }",
            context(1, 2, 3),
            CompileProfileId::Uncontrolled,
        ),
        (count, context(1, 4, 3), CompileProfileId::Uncontrolled),
        (count, context(1, 2, 3), CompileProfileId::GovernedV1),
    ] {
        let mut other = SecurityPlanKey::from_query(
            &query(text),
            binding.scope(),
            profile,
            identity.cache_identity(),
        );
        other.structural_hash = 7;
        assert_ne!(key, other);
    }
}
