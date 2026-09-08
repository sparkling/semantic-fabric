use super::*;
use crate::compiler_control::CompileContext;
use sf_core::ir::TriplesMap;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};

fn fixture() -> Vec<TriplesMap> {
    sf_mapping::parse_r2rml(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
        <urn:map> a rr:TriplesMap; rr:logicalTable [rr:tableName "edges"];
          rr:subjectMap [rr:template "http://example.test/node/{s}"];
          rr:predicateObjectMap [rr:predicate <urn:p>;
            rr:objectMap [rr:template "http://example.test/node/{o}"]]."#,
    )
    .unwrap()
}

fn node(iri: &str) -> NamedNode {
    NamedNode::new_unchecked(iri)
}

fn graph(iri: &str) -> TermMap {
    TermMap::Constant(node(iri).into())
}

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn with<T>(
    maps: &[TriplesMap],
    control: Option<&dyn QueryControl>,
    f: impl FnOnce(&mut Unfolder<'_>) -> Result<T>,
) -> Result<T> {
    let tbox = crate::Tbox::default();
    let mut unfolder = Unfolder::new(maps, &tbox, sf_sql::Dialect::Sqlite, &[]);
    if let Some(control) = control {
        unfolder = unfolder.with_work_mode(CompilerWorkMode::Metered(CompileContext::new(control)));
    }
    f(&mut unfolder)
}

#[test]
fn mapping_walks_have_inclusive_zero_exact_and_adjacent_bounds() {
    let maps = fixture();
    for (operation, expected) in [(0, 2), (1, 2), (2, 3)] {
        for cap in [0, expected - 1, expected] {
            let control = budget(cap);
            let result = with(&maps, Some(&control), |u| match operation {
                0 => u.has_non_constant_graph_map().map(|v| assert!(!v)),
                1 => u.declared_constant_graphs().map(|v| assert!(v.is_empty())),
                _ => u.graph_is_single_predicate("urn:p").map(|v| assert!(v)),
            });
            if cap == expected {
                result.unwrap();
                assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
            } else {
                assert!(matches!(
                    result,
                    Err(crate::Error::QueryControl(
                        QueryControlError::CompilerWorkExceeded
                    ))
                ));
            }
        }
    }
}

#[test]
fn graph_inventory_preserves_order_default_exclusion_and_exact_copy_work() {
    let mut maps = fixture();
    maps[0].subject.graphs = vec![graph("urn:z"), graph(RR_DEFAULT_GRAPH), graph("urn:z")];
    maps[0].predicate_object_maps[0].graphs = vec![graph("urn:a"), graph("urn:z")];
    // Seven visits, four prospective comparisons, two scalar owners + UTF-8.
    let expected = 11 + 2 * (1 + "urn:z".len() as u64);
    let control = budget(expected);
    let paid = with(&maps, Some(&control), |u| u.declared_constant_graphs()).unwrap();
    assert_eq!(paid, vec![node("urn:z"), node("urn:a")]);
    assert_eq!(
        paid,
        with(&maps, None, |u| u.declared_constant_graphs()).unwrap()
    );
    assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
    assert!(matches!(
        with(&maps, Some(&budget(expected - 1)), |u| u
            .declared_constant_graphs()),
        Err(crate::Error::QueryControl(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
}

#[test]
fn dynamic_graph_and_reflexive_checks_remain_mapping_wide() {
    let mut maps = fixture();
    let mut unrelated = maps[0].clone();
    unrelated.id = "unrelated".into();
    unrelated.predicate_object_maps[0].predicates = vec![graph("urn:other")];
    unrelated.predicate_object_maps[0].graphs = vec![unrelated.subject.term.clone()];
    maps.push(unrelated);
    for control in [None, Some(&budget(10_000) as &dyn QueryControl)] {
        with(&maps, control, |u| {
            assert!(u.has_non_constant_graph_map()?);
            assert!(!u.graph_is_single_predicate("urn:p")?);
            Ok(())
        })
        .unwrap();
    }
    maps.truncate(1);
    maps[0].subject.classes.push(node("urn:C"));
    assert!(!with(&maps, Some(&budget(1)), |u| u
        .graph_is_single_predicate("urn:p"))
    .unwrap());
}

struct CancelAt(QueryBudget, u64);
impl QueryControl for CancelAt {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.0.checkpoint()
    }
    fn consume(
        &self,
        charge: QueryCharge,
        units: u64,
    ) -> std::result::Result<(), QueryControlError> {
        self.0.consume(charge, units)?;
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
fn cancellation_stops_each_mapping_walk_and_empty_inventory() {
    let mut maps = fixture();
    maps[0].subject.graphs = vec![graph("urn:g")];
    for operation in 0..3 {
        let control = CancelAt(budget(10_000), 1);
        let result = with(&maps, Some(&control), |u| match operation {
            0 => u.has_non_constant_graph_map().map(|_| ()),
            1 => u.declared_constant_graphs().map(|_| ()),
            _ => u.graph_is_single_predicate("urn:p").map(|_| ()),
        });
        assert!(matches!(
            result,
            Err(crate::Error::QueryControl(QueryControlError::Cancelled))
        ));
        assert_eq!(control.0.consumed(QueryCharge::CompilerWork), 1);
    }
    let stopped = budget(0);
    stopped.terminate(QueryControlError::Cancelled);
    assert!(matches!(
        with(&[], Some(&stopped), |u| u.declared_constant_graphs()),
        Err(crate::Error::QueryControl(QueryControlError::Cancelled))
    ));
}

#[test]
fn copy_or_branch_failure_preserves_the_previous_graph_scope() {
    use spargebra::{algebra::PropertyPathExpression, term::Variable};
    let mut maps = fixture();
    maps[0].subject.graphs = vec![graph("urn:g")];
    let clone_work = 1 + "urn:g".len() as u64;
    // Two mapping inventories each visit map, subject graph and POM; one name
    // copy and one branch slot. The next copy or first hop must fail in turn.
    for cap in [6 + clone_work + 1, 6 + 2 * clone_work + 1] {
        with(&maps, Some(&budget(cap)), |u| {
            u.current_graph_var = Some("outer".into());
            let result = u.path_branches_for_graph_var(
                &Variable::new_unchecked("s").into(),
                &PropertyPathExpression::OneOrMore(Box::new(node("urn:p").into())),
                &Variable::new_unchecked("o").into(),
                "g",
            );
            assert!(matches!(
                result,
                Err(crate::Error::QueryControl(
                    QueryControlError::CompilerWorkExceeded
                ))
            ));
            assert!(u.current_graph.is_none());
            assert_eq!(u.current_graph_var.as_deref(), Some("outer"));
            Ok(())
        })
        .unwrap();
    }
}
