use super::*;
use crate::compiler_control::CompileContext;
use crate::plan_measure::clone_root::{measure_compiler_clone_root_v1, CompilerCloneRootV1};
use sf_core::ir::{LogicalSource, TriplesMap};
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};

fn fixture() -> Vec<TriplesMap> {
    sf_mapping::parse_r2rml(
        r#"
        @prefix rr: <http://www.w3.org/ns/r2rml#> .
        @prefix ex: <http://example.test/> .
        <#Edges> a rr:TriplesMap ;
          rr:logicalTable [ rr:tableName "edges" ] ;
          rr:subjectMap [ rr:template "http://example.test/node/{s}" ] ;
          rr:predicateObjectMap [ rr:predicate ex:a, ex:b ;
            rr:objectMap [ rr:template "http://example.test/node/{o}" ] ] .
    "#,
    )
    .unwrap()
}

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn node(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("http://example.test/{local}"))
}

fn compile(
    maps: &[TriplesMap],
    control: Option<&dyn QueryControl>,
    graph: Option<NamedNode>,
    negated: Option<&[NamedNode]>,
) -> Result<CompiledHop> {
    let tbox = crate::Tbox::default();
    let mut unfolder = Unfolder::new(maps, &tbox, sf_sql::Dialect::Sqlite, &[]);
    if let Some(control) = control {
        unfolder = unfolder.with_work_mode(CompilerWorkMode::Metered(CompileContext::new(control)));
    }
    unfolder.current_graph = graph;
    match negated {
        Some(negated) => unfolder.compile_nps(negated),
        None => unfolder.resolve_pred_hop(node("a").as_str()),
    }
}

fn copy_work(maps: &[TriplesMap]) -> u64 {
    let tm = &maps[0];
    let ObjectMap::Term(object) = &tm.predicate_object_maps[0].objects[0] else {
        panic!()
    };
    [
        CompilerCloneRootV1::TermMap(&tm.subject.term),
        CompilerCloneRootV1::TermMap(object),
        CompilerCloneRootV1::LogicalSource(&tm.source),
    ]
    .into_iter()
    .map(|root| {
        measure_compiler_clone_root_v1(root)
            .unwrap()
            .deep_clone_work
    })
    .sum()
}

fn signature(hop: &CompiledHop) -> String {
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        hop.expr, hop.subj_map, hop.obj_map, hop.subj_shape, hop.obj_shape, hop.single_pred
    )
}

#[test]
fn negated_search_and_actual_scalar_copies_have_inclusive_bounds() {
    let maps = fixture();
    // Complement: map+POM+2 visits+2 exclusions+1 comparison+2 leaf visits = 9.
    // Each leaf: map+POM+2 predicate candidates+1 object, then two term maps/source.
    let expected = 9 + 2 * (5 + copy_work(&maps));
    let exact = budget(expected);
    let metered = compile(&maps, Some(&exact), None, Some(&[node("absent")])).unwrap();
    let raw = compile(&maps, None, None, Some(&[node("absent")])).unwrap();
    assert_eq!(signature(&metered), signature(&raw));
    assert!(matches!(metered.expr, HopExpr::Nps(ref hops) if hops.len() == 2));
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), expected);
    assert!(matches!(
        compile(
            &maps,
            Some(&budget(expected - 1)),
            None,
            Some(&[node("absent")])
        ),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
}

#[test]
fn duplicate_predicates_are_not_duplicate_producers_or_duplicate_complements() {
    let mut maps = fixture();
    let predicates = &mut maps[0].predicate_object_maps[0].predicates;
    predicates.insert(1, predicates[0].clone());
    // Complement 12; each leaf 6 + its three actual copies.
    let expected = 12 + 2 * (6 + copy_work(&maps));
    let exact = budget(expected);
    let hop = compile(&maps, Some(&exact), None, Some(&[node("absent")])).unwrap();
    assert!(matches!(hop.expr, HopExpr::Nps(ref hops) if hops.len() == 2));
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), expected);
    let single = compile(&maps, Some(&budget(100_000)), None, Some(&[node("a")])).unwrap();
    assert!(matches!(single.expr, HopExpr::Nps(ref hops) if hops.len() == 1));
}

#[test]
fn exclusions_short_circuit_complement_checks_and_empty_still_rejects() {
    let maps = fixture();
    // One map + one POM + two predicates + 2*2 prospective negated comparisons.
    let exact = budget(8);
    let result = compile(&maps, Some(&exact), None, Some(&[node("a"), node("b")]));
    assert!(matches!(result, Err(Error::Unsupported(ref s)) if s.contains("complement is empty")));
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), 8);
    assert!(matches!(
        compile(&maps, Some(&budget(7)), None, Some(&[node("a"), node("b")])),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
}

#[test]
fn wrong_graph_fallback_is_separately_paid_and_remains_empty() {
    let mut maps = fixture();
    maps[0].subject.graphs = vec![TermMap::Constant(node("g").into())];
    // Scoped map+POM+graph visit+filter = 4, unscoped map+POM+2pred+object = 5.
    let expected = 9 + copy_work(&maps);
    let exact = budget(expected);
    let hop = compile(&maps, Some(&exact), None, None).unwrap();
    let raw = compile(&maps, None, None, None).unwrap();
    assert_eq!(signature(&hop), signature(&raw));
    assert!(
        matches!(hop.expr, HopExpr::Pred(HopRelation { source: LogicalSource::Query(ref sql), .. }) if sql.contains("WHERE 1 = 0"))
    );
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), expected);
    assert!(matches!(
        compile(&maps, Some(&budget(expected - 1)), None, None),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    let visible = compile(&maps, Some(&budget(100_000)), Some(node("g")), None).unwrap();
    assert!(matches!(
        visible.expr,
        HopExpr::Pred(HopRelation {
            source: LogicalSource::Table(_),
            ..
        })
    ));
}

#[test]
fn graph_filtering_precedes_dynamic_predicate_rejection_but_not_classes() {
    let mut maps = fixture();
    let mut wrong = maps[0].predicate_object_maps[0].clone();
    wrong.graphs = vec![TermMap::Constant(node("other").into())];
    wrong.predicates = vec![maps[0].subject.term.clone()];
    maps[0].predicate_object_maps.push(wrong);
    let metered = compile(&maps, Some(&budget(100_000)), None, Some(&[node("absent")])).unwrap();
    assert_eq!(
        signature(&metered),
        signature(&compile(&maps, None, None, Some(&[node("absent")])).unwrap())
    );
    let error = compile(
        &maps,
        Some(&budget(100_000)),
        Some(node("other")),
        Some(&[node("absent")]),
    );
    assert!(matches!(error, Err(Error::Unsupported(ref s)) if s.contains("non-constant")));
    maps[0].subject.classes.push(node("C"));
    assert!(
        matches!(compile(&maps, Some(&budget(100_000)), Some(node("absent")), Some(&[])),
        Err(Error::Unsupported(ref s)) if s.contains("rr:class"))
    );
}

#[test]
fn zero_object_and_ambiguous_producers_keep_existing_rejections() {
    let mut maps = fixture();
    maps[0].predicate_object_maps[0].objects.clear();
    assert!(
        matches!(compile(&maps, Some(&budget(100_000)), None, Some(&[node("absent")])),
        Err(Error::Unsupported(ref s)) if s.contains("not mapped"))
    );
    let mut maps = fixture();
    let object = maps[0].predicate_object_maps[0].objects[0].clone();
    maps[0].predicate_object_maps[0].objects.push(object);
    assert!(matches!(compile(&maps, Some(&budget(100_000)), None, None),
        Err(Error::Unsupported(ref s)) if s.contains(">1 mapping")));
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
fn cancellation_after_search_reservation_prevents_payload_copy() {
    let maps = fixture();
    let control = CancelAt(budget(100_000), 5);
    assert!(matches!(
        compile(&maps, Some(&control), None, None),
        Err(Error::QueryControl(QueryControlError::Cancelled))
    ));
    assert_eq!(control.0.consumed(QueryCharge::CompilerWork), 5);
    let empty = budget(0);
    empty.terminate(QueryControlError::Cancelled);
    assert!(matches!(
        compile(&[], Some(&empty), None, Some(&[])),
        Err(Error::QueryControl(QueryControlError::Cancelled))
    ));
}

#[test]
fn failed_path_expansion_cannot_cache_and_paid_hits_share_the_plan() {
    use crate::{CompilerBinding, CompilerSchema, Tbox};
    use sf_core::{SourceId, SourceMapping};
    let binding = CompilerBinding::new(
        SourceMapping::new(SourceId::new(0).unwrap(), fixture()),
        sf_sql::Dialect::Sqlite,
        Tbox::default(),
        CompilerSchema::from_unverified_observation(vec![]),
        8,
    );
    let query = "SELECT ?s ?o WHERE { ?s !<urn:absent> ?o }";
    let key_control = budget(u64::MAX);
    crate::cache::bounded_key::plan_key_with_work_control(
        &crate::parse_query(query).unwrap(),
        binding.scope(),
        crate::cache::CompileProfileId::Uncontrolled,
        &key_control,
    )
    .unwrap();
    let key_work = key_control.consumed(QueryCharge::CompilerWork);
    let short = budget(key_work);
    assert!(binding
        .compile_shared_with_work_control(query, &short)
        .is_err());
    assert_eq!(short.consumed(QueryCharge::CompilerWork), key_work);
    assert_eq!(binding.cache_len(), 0);
    let paid = budget(100_000);
    let first = binding
        .compile_shared_with_work_control(query, &paid)
        .unwrap();
    assert!(paid.consumed(QueryCharge::CompilerWork) > 0);
    let second = binding
        .compile_shared_with_work_control(query, &budget(key_work))
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &second));
    assert_eq!(
        format!("{first:?}"),
        format!("{:?}", binding.compile_uncached_shared(query).unwrap())
    );
}
