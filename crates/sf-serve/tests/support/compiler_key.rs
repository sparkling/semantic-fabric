//! Pay the exact prerequisite key work in tests aimed at a later phase.
//! This calibrates through a raw-populated warm hit, not a second copy of the
//! private AST visitor. Independent sf-sparql observer tests check its charges.

pub(crate) fn key_work(source: &str) -> u64 {
    use sf_core::{
        query_control::{QueryBudget, QueryCharge, QueryLimits},
        SourceId, SourceMapping,
    };
    use sf_sparql::{
        cache::{CompilerBinding, Epoch},
        Tbox,
    };
    let maps = sf_mapping::parse_r2rml(r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        @prefix ex: <http://example.test/> .
        <http://example.test/key-fixture> a rr:TriplesMap;
          rr:logicalTable [rr:tableName "edges"];
          rr:subjectMap [rr:template "http://example.test/node/{s}"; rr:graph ex:g, rr:defaultGraph];
          rr:predicateObjectMap [rr:predicate ex:a, ex:b, ex:value;
            rr:objectMap [rr:template "http://example.test/node/{o}"]].
    "#).unwrap();
    let binding = CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), maps),
        sf_sql::Dialect::Sqlite,
        Tbox::default(),
        vec![],
        Epoch::default(),
        1,
    );
    let raw = binding
        .compile_shared(source)
        .expect("raw fixture compiles");
    let control = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let controlled = binding
        .compile_shared_with_work_control(source, &control)
        .unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&raw, &controlled),
        "measure a hit, never miss work"
    );
    control.consumed(QueryCharge::CompilerWork)
}

/// Isolate structural BUILD, so a later-stage test pays this new prerequisite
/// without calibrating away the expansion/mapping work that it actually checks.
/// Only ordinary SELECT fixtures: this does not model RDF-star/DESCRIBE rewrites.
pub(crate) fn build_work(source: &str) -> u64 {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
    let parsed = spargebra::SparqlParser::new().parse_query(source).unwrap();
    let spargebra::Query::Select { pattern, .. } = &parsed else {
        panic!("BUILD calibration requires an ordinary SELECT fixture")
    };
    let control = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    sf_sparql::build::build_tree_with_work_control(pattern, None, &control).unwrap();
    control.consumed(QueryCharge::CompilerWork)
}
