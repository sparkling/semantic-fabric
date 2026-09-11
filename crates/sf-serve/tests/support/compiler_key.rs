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

/// Isolate NORMALIZE after raw BUILD/RESOLVE. Later LOWER/copy tests pay this
/// prerequisite without measuring away the operation they intend to reject.
/// No database access, and no RDF-star/DESCRIBE rewrite is modeled here.
pub(crate) fn normalization_work(source: &str, maps: &[sf_core::ir::TriplesMap]) -> u64 {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
    let spargebra::Query::Select { pattern, .. } =
        spargebra::SparqlParser::new().parse_query(source).unwrap()
    else {
        panic!("NORMALIZE calibration requires ordinary SELECT");
    };
    let tree = sf_sparql::build::build_tree(&pattern, None).unwrap();
    let tbox = sf_sparql::Tbox::default();
    let mut cx = sf_sparql::iq::resolve::ResolveCx::new(maps, &tbox, sf_sql::Dialect::Sqlite, &[]);
    let resolved = sf_sparql::iq::resolve::resolve(tree, &mut cx).unwrap();
    let control = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    sf_sparql::iq::normalize::normalize_with_work_control(resolved, &control).unwrap();
    control.consumed(QueryCharge::CompilerWork)
}

/// Independent LOWER cost, without subtracting an end-to-end cold compilation.
/// Only ordinary SELECT fixtures; the caller proves the actual stage path.
#[allow(dead_code)] // Shared support is also included by earlier-phase test modules.
pub(crate) fn lowering_work(source: &str, maps: &[sf_core::ir::TriplesMap]) -> u64 {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
    let spargebra::Query::Select { pattern, .. } =
        spargebra::SparqlParser::new().parse_query(source).unwrap()
    else {
        panic!("LOWER calibration requires ordinary SELECT");
    };
    let tree = sf_sparql::build::build_tree(&pattern, None).unwrap();
    let tbox = sf_sparql::Tbox::default();
    let mut cx = sf_sparql::iq::resolve::ResolveCx::new(maps, &tbox, sf_sql::Dialect::Sqlite, &[]);
    let resolved = sf_sparql::iq::resolve::resolve(tree, &mut cx).unwrap();
    let normalized = sf_sparql::iq::normalize::normalize(resolved).unwrap();
    let control = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let plan = sf_sparql::iq::lower::lower_with_work_control(
        normalized,
        sf_sql::Dialect::Sqlite,
        &Default::default(),
        &Default::default(),
        &control,
    )
    .unwrap();
    assert!(
        plan.branches.len() > 1
            || plan
                .branches
                .iter()
                .any(|b| !b.opts.is_empty() || b.subplan_joins.iter().any(|sp| sp.left)),
        "the fixture must actually lower an OPTIONAL result carrier"
    );
    control.consumed(QueryCharge::CompilerWork)
}

pub(crate) const CONSTANT_QUERIES: [&str; 3] = [
    "SELECT DISTINCT ?value WHERE { VALUES ?value { \"one\" \"one\" \"two\" } }",
    "SELECT ?value WHERE { { VALUES ?value { \"one\" } } UNION { VALUES ?value { \"two\" } } }",
    "SELECT ?value WHERE { VALUES ?value { \"skip\" \"one\" \"two\" \"tail\" } } LIMIT 2 OFFSET 1",
];

pub(crate) const STRUCTURAL_QUERIES: [&str; 3] = [
    "SELECT ?value WHERE { VALUES ?value { \"one\" \"two\" } FILTER EXISTS { VALUES ?inside { 7 } } }",
    "SELECT ?value WHERE { { SELECT ?value WHERE { VALUES ?value { \"one\" \"two\" } } } }",
    "SELECT ?value WHERE { VALUES ?value { \"one\" \"two\" } OPTIONAL { VALUES ?inside { 7 } } }",
];

pub(crate) fn structural_compile_work(source: &str) -> u64 {
    assert!(STRUCTURAL_QUERIES.contains(&source));
    source_free_compile_work(source)
}

/// End-to-end cold compiler allowance for the three source-free row-rule fixtures.
/// Unit tests independently pin the new NORMALIZE schedule and copy boundaries.
/// This excludes serving's decoded-input charge; a warm hit still pays only key work.
pub(crate) fn constant_compile_work(source: &str) -> u64 {
    assert!(CONSTANT_QUERIES.contains(&source));
    source_free_compile_work(source)
}

fn source_free_compile_work(source: &str) -> u64 {
    use sf_core::{
        query_control::{QueryBudget, QueryCharge, QueryLimits},
        SourceId, SourceMapping,
    };
    use sf_sparql::{
        cache::{CompilerBinding, Epoch},
        Tbox,
    };
    let binding = CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), vec![]),
        sf_sql::Dialect::Sqlite,
        Tbox::default(),
        vec![],
        Epoch::default(),
        1,
    );
    let control = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    binding
        .compile_shared_with_work_control(source, &control)
        .unwrap();
    control.consumed(QueryCharge::CompilerWork)
}
