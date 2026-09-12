use super::*;
use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};

const COUNT: &str = "SELECT (COUNT(*) AS ?count) WHERE { VALUES ?x { 1 1 2 } }";

fn parse(source: &str) -> Query {
    spargebra::SparqlParser::new().parse_query(source).unwrap()
}

fn scope() -> CompileScope {
    test_scope(SourceId::new(0).unwrap(), Dialect::Sqlite, Epoch(0))
}

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

#[test]
fn cache_identity_independent_parses_keep_internal_binders_stable() {
    for source in [
        COUNT,
        "SELECT ?x (COUNT(*) AS ?n) (SUM(?y) AS ?sum) WHERE { \
         VALUES (?x ?y) { (\"a\" 1) (\"a\" 2) } } GROUP BY ?x \
         HAVING (COUNT(*) > 1) ORDER BY DESC(COUNT(*))",
        "SELECT ?total WHERE { { SELECT (COUNT(*) AS ?total) \
         WHERE { VALUES ?x { 1 2 } } } }",
        "SELECT ?a ?b WHERE { \
         { SELECT (COUNT(*) AS ?a) WHERE { VALUES ?x { 1 2 } } } \
         { SELECT (COUNT(*) AS ?b) WHERE { VALUES ?x { 3 4 } } } }",
        "ASK WHERE { { SELECT (COUNT(*) AS ?n) WHERE { VALUES ?x { 1 } } } \
         FILTER (?n = 1) }",
        "CONSTRUCT { <http://example.test/s> <http://example.test/count> ?n } \
         WHERE { SELECT (COUNT(*) AS ?n) WHERE { VALUES ?x { 1 } } }",
        "DESCRIBE <http://example.test/s>",
        "DESCRIBE <http://example.test/s> ?s WHERE { ?s ?p ?o }",
    ] {
        let left = parse(source);
        let right = parse(source);
        let raw = plan_key(&left, scope());
        assert_eq!(raw, plan_key(&right, scope()), "{source}");
        let paid = budget(u64::MAX);
        assert_eq!(
            raw,
            bounded_key::plan_key_with_work_control(
                &right,
                scope(),
                CompileProfileId::Uncontrolled,
                &paid
            )
            .unwrap(),
            "raw and controlled identity: {source}"
        );
        assert!(paid.consumed(QueryCharge::CompilerWork) > 0);
    }
}

#[test]
fn cache_identity_independent_source_compiles_reuse_the_same_plan() {
    for controlled_first in [false, true] {
        let binding = CompilerBinding::from_unverified_observation(
            SourceMapping::new(SourceId::new(0).unwrap(), vec![]),
            Dialect::Sqlite,
            Tbox::default(),
            vec![],
            Epoch(0),
            8,
        );
        let first = if controlled_first {
            binding
                .compile_shared_with_work_control(COUNT, &budget(u64::MAX))
                .unwrap()
        } else {
            binding.compile_shared(COUNT).unwrap()
        };
        let second = binding
            .compile_shared_with_work_control(COUNT, &budget(u64::MAX))
            .unwrap();
        let raw = binding.compile_shared(COUNT).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&first, &raw));
        assert!(matches!(&first.form, crate::PlanForm::Select { vars } if vars == &["count"]));
        assert_eq!(binding.cache_len(), 1);
    }
}

#[test]
fn cache_identity_distinct_semantics_and_authored_names_do_not_collide() {
    for (left, right) in [
        (
            COUNT,
            "SELECT (COUNT(DISTINCT ?x) AS ?count) WHERE { VALUES ?x { 1 1 2 } }",
        ),
        (
            COUNT,
            "SELECT (SUM(?x) AS ?count) WHERE { VALUES ?x { 1 1 2 } }",
        ),
        (
            COUNT,
            "SELECT (COUNT(*) AS ?renamed) WHERE { VALUES ?x { 1 1 2 } }",
        ),
        (
            COUNT,
            "SELECT (COUNT(*) AS ?count) WHERE { VALUES ?x { 1 1 3 } }",
        ),
        (
            "DESCRIBE <http://example.test/a>",
            "DESCRIBE <http://example.test/b>",
        ),
        (
            "SELECT ?0123456789abcdef WHERE { VALUES ?0123456789abcdef { 1 } }",
            "SELECT ?fedcba9876543210 WHERE { VALUES ?fedcba9876543210 { 1 } }",
        ),
        (
            "SELECT ?__sf_internal_0 WHERE { VALUES ?__sf_internal_0 { 1 } }",
            "SELECT ?__sf_internal_1 WHERE { VALUES ?__sf_internal_1 { 1 } }",
        ),
    ] {
        let mut left = plan_key(&parse(left), scope());
        let mut right = plan_key(&parse(right), scope());
        left.structural_hash = 7;
        right.structural_hash = 7;
        assert_ne!(
            left, right,
            "full equality must survive forced hash collision"
        );
        let cache = PlanCache::new(2);
        cache.put(left.clone(), 11);
        assert_eq!(cache.get(&right), None);
        assert_eq!(cache.get(&left), Some(11));
    }
}

#[test]
fn cache_identity_unpaid_key_never_publishes_and_warm_hit_pays_key_and_lookup() {
    let binding = CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), vec![]),
        Dialect::Sqlite,
        Tbox::default(),
        vec![],
        Epoch(0),
        8,
    );
    let query = parse(COUNT);
    let paid = budget(u64::MAX);
    bounded_key::plan_key_with_work_control(
        &query,
        binding.scope(),
        CompileProfileId::Uncontrolled,
        &paid,
    )
    .unwrap();
    let key_work = paid.consumed(QueryCharge::CompilerWork);
    let short = budget(key_work - 1);
    assert!(binding
        .compile_parsed_shared_with_work_control(&query, &short)
        .is_err());
    assert_eq!(binding.cache_len(), 0);
    let first = binding.compile_shared(COUNT).unwrap();
    let warm_work = key_work + super::test_work(8, COUNT).1;
    let exact = budget(warm_work);
    let warm = binding
        .compile_parsed_shared_with_work_control(&query, &exact)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &warm));
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), warm_work);
    assert_eq!(binding.cache_len(), 1);
}
