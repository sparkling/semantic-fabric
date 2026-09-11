use super::*;
use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
use spargebra::algebra::Expression;
use spargebra::term::{NamedNode, Variable};

fn constant(name: &str) -> Query {
    Query::Describe {
        dataset: None,
        base_iri: None,
        pattern: GraphPattern::Project {
            variables: vec![Variable::new(name).unwrap()],
            inner: Box::new(GraphPattern::Extend {
                inner: Box::new(GraphPattern::Bgp { patterns: vec![] }),
                variable: Variable::new(name).unwrap(),
                expression: Expression::NamedNode(NamedNode::new("urn:a").unwrap()),
            }),
        },
    }
}

#[test]
fn describe_parse_forced_random_lengths_have_identical_ast_and_paid_key_work() {
    let mut expected = None;
    // Force the upstream unpadded-hex range; no probabilistic regression test.
    for length in 1..=32 {
        let mut query = constant(&"a".repeat(length));
        normalize_describe_parse(&mut query).unwrap();
        let budget = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
        let key = controlled(
            &query,
            &AlgebraEnvelopeV1::validate(&query).unwrap(),
            &budget,
        )
        .unwrap()
        .to_string();
        let actual = (query, key, budget.consumed(QueryCharge::CompilerWork));
        if let Some(expected) = &expected {
            assert_eq!(&actual, expected);
        } else {
            expected = Some(actual);
        }
    }
}

#[test]
fn describe_parse_direct_repeats_and_preserves_observable_names() {
    for source in [
        "DESCRIBE <urn:a>",
        "DESCRIBE <urn:a> <urn:b> ?__sf_cache0 WHERE { VALUES ?__sf_cache0 { <urn:c> } }",
    ] {
        let expected = crate::parse_query(source).unwrap();
        for _ in 0..16 {
            assert_eq!(crate::parse_query(source).unwrap(), expected);
        }
    }
    for source in [
        "SELECT (COUNT(*) AS ?n) WHERE { VALUES ?x { 1 } }",
        "ASK { ?s ?p ?o }",
        "DESCRIBE ?x WHERE { ?x <urn:p> ?o }",
        "DESCRIBE ?x WHERE { BIND(<urn:a> AS ?x) FILTER(?x = <urn:a>) }",
        "DESCRIBE ?x WHERE { VALUES ?x { <urn:a> } }",
        "DESCRIBE ?x WHERE { BIND(<urn:a> AS ?x) FILTER EXISTS { ?x <urn:p> ?o } }",
    ] {
        let mut query = spargebra::SparqlParser::new().parse_query(source).unwrap();
        let original = query.clone();
        normalize_describe_parse(&mut query).unwrap();
        assert_eq!(query, original, "{source}");
    }
}

#[test]
fn describe_parse_cache_reuses_parsed_and_string_entries_in_both_orders() {
    use std::sync::Arc;
    for source in [
        "DESCRIBE <urn:a>",
        "DESCRIBE <urn:a> WHERE { VALUES (?__sf_cache0 ?__sf_parse0) { (<urn:b> <urn:c>) } }",
    ] {
        for parsed_first in [false, true] {
            let binding = crate::CompilerBinding::from_unverified_observation(
                sf_core::SourceMapping::new(sf_core::SourceId::new(0).unwrap(), vec![]),
                sf_sql::Dialect::Sqlite,
                Default::default(),
                vec![],
                Default::default(),
                1,
            );
            let parsed = spargebra::SparqlParser::new().parse_query(source).unwrap();
            let get_parsed = || crate::translate_cached_shared(&parsed, &binding).unwrap();
            let get_string = || binding.compile_shared(source).unwrap();
            let (first, second) = if parsed_first {
                (get_parsed(), get_string())
            } else {
                (get_string(), get_parsed())
            };
            assert!(
                Arc::ptr_eq(&first, &second),
                "{source}, parsed first: {parsed_first}"
            );
        }
    }
}
