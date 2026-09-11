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
    let (plan, work) = lowering_plan_and_work(source, maps);
    assert!(
        plan.branches.len() > 1
            || plan
                .branches
                .iter()
                .any(|b| !b.opts.is_empty() || b.subplan_joins.iter().any(|sp| sp.left)),
        "the fixture must actually lower an OPTIONAL result carrier"
    );
    work
}

#[allow(dead_code)]
pub(crate) fn lower_scope_work(source: &str, maps: &[sf_core::ir::TriplesMap]) -> u64 {
    let (plan, work) = lowering_plan_and_work(source, maps);
    assert_eq!(plan.branches.len(), 1);
    let branch = &plan.branches[0];
    assert_eq!(branch.core.len(), 1);
    assert!(branch.opts.is_empty() && branch.subplan_joins.is_empty());
    assert!(matches!(&plan.form, sf_sparql::PlanForm::Select { vars } if !vars.is_empty()));
    assert!(work > 0, "the plain mapped scope must no longer be unpaid");
    work
}

fn lowering_plan_and_work(
    source: &str,
    maps: &[sf_core::ir::TriplesMap],
) -> (sf_sparql::Plan, u64) {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
    let normalized = normalized_tree(source, maps);
    let control = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let plan = sf_sparql::iq::lower::lower_with_work_control(
        normalized,
        sf_sql::Dialect::Sqlite,
        &Default::default(),
        &Default::default(),
        &control,
    )
    .unwrap();
    (plan, control.consumed(QueryCharge::CompilerWork))
}

fn normalized_tree(source: &str, maps: &[sf_core::ir::TriplesMap]) -> sf_sparql::iq::node::IqNode {
    let spargebra::Query::Select { pattern, .. } =
        spargebra::SparqlParser::new().parse_query(source).unwrap()
    else {
        panic!("LOWER calibration requires ordinary SELECT");
    };
    let tree = sf_sparql::build::build_tree(&pattern, None).unwrap();
    let tbox = sf_sparql::Tbox::default();
    let mut cx = sf_sparql::iq::resolve::ResolveCx::new(maps, &tbox, sf_sql::Dialect::Sqlite, &[]);
    let resolved = sf_sparql::iq::resolve::resolve(tree, &mut cx).unwrap();
    sf_sparql::iq::normalize::normalize(resolved).unwrap()
}

/// Hand-count only the entry prefix and final projection for these source-free
/// fixtures. Never execute/subtract LOWER: that could pay away the clone/product
/// rejection under test. Shape assertions fail if a new operator needs accounting.
#[allow(dead_code)]
pub(crate) fn source_free_entry_work(source: &str) -> (u64, u64) {
    use sf_sparql::iq::node::{IqCond, IqNode, Var};
    fn visits(node: &IqNode) -> usize {
        1 + match node {
            IqNode::Construction { child, subst, .. } => {
                assert!(subst.is_empty());
                assert!(matches!(
                    **child,
                    IqNode::Values { .. }
                        | IqNode::Filter { .. }
                        | IqNode::InnerJoin { .. }
                        | IqNode::Empty { .. }
                ));
                visits(child)
            }
            IqNode::Filter { child, cond } => {
                assert!(matches!(**child, IqNode::Values { .. }));
                let [IqCond::Exists(inner)] = cond.as_slice() else {
                    panic!("one EXISTS only")
                };
                assert!(matches!(**inner, IqNode::Values { .. }));
                visits(child) + 1 + 1 + visits(inner) // condition slot + condition node
            }
            IqNode::InnerJoin { children, cond } => {
                assert!(cond.is_empty());
                assert!(children.iter().all(|c| matches!(c, IqNode::Values { .. })));
                children.len() + children.iter().map(visits).sum::<usize>()
            }
            IqNode::Values { .. } | IqNode::Empty { .. } => 0,
            _ => panic!("fixture has an uncalibrated LOWER operator"),
        }
    }
    let node = normalized_tree(source, &[]);
    let vars = match &node {
        IqNode::Construction { project, .. } => project,
        IqNode::Values { vars, .. } | IqNode::Empty { vars } => vars,
        _ => panic!("fixture must have a direct, non-modifier output scope"),
    };
    let payload = vars.iter().map(|v| 1 + v.len()).sum::<usize>();
    // Alias visits + successor, one spine dispatch, output-scope visit, then its
    // logical Var vector and payload. Final String vector/payload are a tail.
    let prefix = visits(&node) + 3 + vars.len() * (1 + std::mem::size_of::<Var>()) + payload;
    let tail = vars.len() * (1 + std::mem::size_of::<String>()) + payload;
    (prefix as u64, tail as u64)
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
