use super::*;
use crate::compiler_control::CompileContext;
use crate::plan_measure::clone_root::{measure_compiler_clone_root_v1, CompilerCloneRootV1};
use sf_core::ir::{ObjectMap, RefObjectMap, SubjectMap};
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use spargebra::term::Variable;

fn iri(local: &str) -> TermMap {
    TermMap::Constant(
        sf_core::NamedNode::new(format!("http://example.test/{local}"))
            .unwrap()
            .into(),
    )
}

fn fixture() -> Vec<TriplesMap> {
    vec![TriplesMap {
        id: "map".into(),
        source: LogicalSource::Table("items".into()),
        subject: SubjectMap {
            term: iri("s"),
            classes: vec![],
            graphs: vec![],
        },
        predicate_object_maps: vec![PredicateObjectMap {
            predicates: vec![iri("a"), iri("b")],
            objects: ["x", "y", "z"].map(|v| ObjectMap::Term(iri(v))).into(),
            graphs: vec![],
        }],
    }]
}

fn pattern(predicate: Option<&str>) -> TriplePattern {
    TriplePattern {
        subject: Variable::new_unchecked("s").into(),
        predicate: predicate.map_or_else(
            || Variable::new_unchecked("p").into(),
            |p| sf_core::NamedNode::new_unchecked(p).into(),
        ),
        object: Variable::new_unchecked("o").into(),
    }
}

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn expand(
    maps: &[TriplesMap],
    tp: &TriplePattern,
    graph: Option<&NamedNodePattern>,
    control: &dyn QueryControl,
) -> Result<Vec<Branch>> {
    let tbox = crate::Tbox::default();
    Unfolder::new(maps, &tbox, sf_sql::Dialect::Sqlite, &[])
        .with_work_mode(CompilerWorkMode::Metered(CompileContext::new(control)))
        .resolve_pattern(tp, graph)
}

fn source_work(source: &LogicalSource) -> u64 {
    measure_compiler_clone_root_v1(CompilerCloneRootV1::LogicalSource(source))
        .unwrap()
        .deep_clone_work
}

#[test]
fn rejected_candidates_have_an_inclusive_prospective_boundary() {
    let maps = fixture();
    let tp = pattern(Some("http://example.test/absent"));
    // One map visit + one POM visit + six prospective candidates; no source copy.
    let short = budget(7);
    assert!(matches!(
        expand(&maps, &tp, None, &short),
        Err(crate::Error::QueryControl(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 2);
    let exact = budget(8);
    assert!(expand(&maps, &tp, None, &exact).unwrap().is_empty());
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), 8);
}

#[test]
fn source_copies_are_exact_and_preserve_raw_branch_order() {
    let maps = fixture();
    let copy = source_work(&maps[0].source);
    let exact = budget(8 + 6 * copy);
    let tp = pattern(None);
    let metered = expand(&maps, &tp, None, &exact).unwrap();
    let tbox = crate::Tbox::default();
    let raw = Unfolder::new(&maps, &tbox, sf_sql::Dialect::Sqlite, &[])
        .resolve_pattern(&tp, None)
        .unwrap();
    assert_eq!(metered.len(), 6);
    assert_eq!(format!("{metered:?}"), format!("{raw:?}"));
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), 8 + 6 * copy);
    let short = budget(8 + 6 * copy - 1);
    assert!(expand(&maps, &tp, None, &short).is_err());
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 8 + 5 * copy);
}

#[test]
fn graph_union_charges_visits_and_prospective_comparisons_without_reordering() {
    let subject = vec![iri("g2"), iri("g1"), iri("g2")];
    let pom = vec![iri("g1"), iri("g3")];
    let tbox = crate::Tbox::default();
    let exact = budget(12); // Five input visits plus 0+1+2+2+2 comparison candidates.
    let uf = Unfolder::new(&[], &tbox, sf_sql::Dialect::Sqlite, &[])
        .with_work_mode(CompilerWorkMode::Metered(CompileContext::new(&exact)));
    assert_eq!(
        uf.graph_union(&subject, &pom).unwrap(),
        sf_core::graph_map::union(&subject, &pom)
    );
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), 12);
    let short = budget(11);
    let uf = Unfolder::new(&[], &tbox, sf_sql::Dialect::Sqlite, &[])
        .with_work_mode(CompilerWorkMode::Metered(CompileContext::new(&short)));
    assert!(uf.graph_union(&subject, &pom).is_err());
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 10);
}

#[test]
fn named_graph_attempts_exclude_default_and_classes_ignore_pom_graphs() {
    let mut maps = fixture();
    maps[0].subject.graphs = vec![
        iri("g1"),
        iri("g1"),
        TermMap::Constant(
            sf_core::NamedNode::new_unchecked(crate::graph_map::RR_DEFAULT_GRAPH).into(),
        ),
    ];
    maps[0].subject.classes = vec![sf_core::NamedNode::new_unchecked("http://example.test/C")];
    maps[0].predicate_object_maps[0].graphs = vec![iri("g2"), iri("g1")];
    let graph = NamedNodePattern::Variable(Variable::new_unchecked("g"));
    let tp = pattern(None);
    let branches = expand(&maps, &tp, Some(&graph), &budget(100_000)).unwrap();
    assert_eq!(branches.len(), 13); // One class on g1, six POM triples on g1 and g2.
    let tbox = crate::Tbox::default();
    let raw = Unfolder::new(&maps, &tbox, sf_sql::Dialect::Sqlite, &[])
        .resolve_pattern(&tp, Some(&graph))
        .unwrap();
    assert_eq!(format!("{branches:?}"), format!("{raw:?}"));
    maps[0].subject.graphs.clear();
    maps[0].predicate_object_maps[0].graphs.clear();
    assert!(expand(&maps, &tp, Some(&graph), &budget(100_000))
        .unwrap()
        .is_empty());
    maps[0].subject.graphs = vec![TermMap::Constant(
        sf_core::NamedNode::new_unchecked(crate::graph_map::RR_DEFAULT_GRAPH).into(),
    )];
    assert!(expand(&maps, &tp, Some(&graph), &budget(100_000))
        .unwrap()
        .is_empty());
}

#[test]
fn parent_reference_borrows_map_and_charges_only_actual_sources() {
    let mut maps = fixture();
    let mut parent = maps[0].clone();
    parent.id = "parent".into();
    parent.source = LogicalSource::Query("SELECT id FROM parent".into());
    parent.predicate_object_maps.clear();
    maps[0].predicate_object_maps[0].predicates.truncate(1);
    maps[0].predicate_object_maps[0].objects = vec![ObjectMap::Ref(RefObjectMap {
        parent_triples_map: "parent".into(),
        joins: vec![],
    })];
    maps.push(parent);
    let expected = 2 + 1 + 1 + 2 + source_work(&maps[0].source) + source_work(&maps[1].source);
    let control = budget(expected);
    let branches = expand(
        &maps,
        &pattern(Some("http://example.test/a")),
        None,
        &control,
    )
    .unwrap();
    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0].core.len(), 2);
    assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
}

struct CancelAt(QueryBudget, u64);
impl QueryControl for CancelAt {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.0.checkpoint()
    }
    fn consume(
        &self,
        charge: QueryCharge,
        amount: u64,
    ) -> std::result::Result<(), QueryControlError> {
        self.0.consume(charge, amount)?;
        if self.0.consumed(QueryCharge::CompilerWork) >= self.1 {
            self.0.terminate(QueryControlError::Cancelled);
        }
        Ok(())
    }
    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.0.terminate(cause)
    }
}

#[test]
fn cancellation_after_candidate_reservation_prevents_any_source_copy() {
    let control = CancelAt(budget(100_000), 8);
    assert!(matches!(
        expand(&fixture(), &pattern(None), None, &control),
        Err(crate::Error::QueryControl(QueryControlError::Cancelled))
    ));
    assert_eq!(control.0.consumed(QueryCharge::CompilerWork), 8);
}

#[test]
fn class_candidates_charge_before_rejection_and_preserve_duplicates() {
    let mut maps = fixture();
    maps[0].predicate_object_maps.clear();
    maps[0].subject.classes = vec![sf_core::NamedNode::new_unchecked("http://example.test/C"); 2];
    let mut tp = pattern(Some(RDF_TYPE));
    tp.object = sf_core::NamedNode::new_unchecked("http://example.test/absent").into();
    let short = budget(2);
    assert!(expand(&maps, &tp, None, &short).is_err());
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 1);
    let exact = budget(3);
    assert!(expand(&maps, &tp, None, &exact).unwrap().is_empty());
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), 3);
    tp.object = Variable::new_unchecked("o").into();
    let exact = budget(3 + 2 * source_work(&maps[0].source));
    assert_eq!(expand(&maps, &tp, None, &exact).unwrap().len(), 2);
    assert_eq!(
        exact.consumed(QueryCharge::CompilerWork),
        exact.limits().max_compiler_work()
    );
}

#[test]
fn fixed_graph_filter_attempts_are_charged_independently_of_atom_candidates() {
    let mut maps = fixture();
    maps[0].subject.graphs = vec![iri("g")];
    let tp = pattern(Some("http://example.test/a"));
    // Map + POM + graph visit + six candidates, then three source copies and
    // three graph-filter candidates. Other three atoms reject before any copy.
    let expected = 9 + 3 * (source_work(&maps[0].source) + 1);
    let exact = budget(expected);
    let graph =
        NamedNodePattern::NamedNode(sf_core::NamedNode::new_unchecked("http://example.test/g"));
    assert_eq!(expand(&maps, &tp, Some(&graph), &exact).unwrap().len(), 3);
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), expected);
    let short = budget(expected - 1);
    assert!(expand(&maps, &tp, Some(&graph), &short).is_err());
    assert_eq!(short.consumed(QueryCharge::CompilerWork), expected - 1);
}

#[test]
fn empty_products_have_no_candidates_and_cancelled_empty_input_stays_terminal() {
    let mut maps = fixture();
    maps[0].predicate_object_maps[0].objects.clear();
    let exact = budget(2);
    assert!(expand(&maps, &pattern(None), None, &exact)
        .unwrap()
        .is_empty());
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), 2);
    let stopped = budget(0);
    stopped.terminate(QueryControlError::Cancelled);
    assert!(matches!(
        expand(&[], &pattern(None), None, &stopped),
        Err(crate::Error::QueryControl(QueryControlError::Cancelled))
    ));
}
