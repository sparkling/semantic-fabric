use ::spargebra::algebra::{Expression, GraphPattern, PropertyPathExpression, QueryDataset};
use ::spargebra::term::{NamedNode, NamedNodePattern, TermPattern, TriplePattern, Variable};
use sf_core::ir::{
    Join, LogicalSource, ObjectMap, PredicateObjectMap, RefObjectMap, Segment, SubjectMap,
    Template, TermMap, TermSpec, TriplesMap,
};

use super::*;

fn iri(value: &str) -> NamedNode {
    NamedNode::new(value).unwrap()
}

fn variable(value: &str) -> Variable {
    Variable::new(value).unwrap()
}

fn expected(
    nodes: u64,
    collection_slots: u64,
    payload_bytes: u64,
    max_depth: usize,
    max_pending_items: usize,
) -> PlanMeasureV1 {
    PlanMeasureV1 {
        nodes,
        collection_slots,
        payload_bytes,
        deep_clone_work: nodes + collection_slots + payload_bytes,
        max_depth,
        max_pending_items,
    }
}

#[test]
fn scalar_roots_do_not_invent_collection_work() {
    let branch = Branch::empty();
    assert_eq!(
        measure_compiler_clone_root_v1(CompilerCloneRootV1::Branch(&branch)).unwrap(),
        expected(1, 0, 0, 1, 1)
    );
    assert_eq!(
        measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::Branches(
            std::slice::from_ref(&branch),
        ))
        .unwrap(),
        expected(1, 1, 0, 1, 1)
    );

    let term = TermDef::Coalesce(
        Box::new(TermDef::Const(iri("urn:a").into())),
        Box::new(TermDef::Const(iri("urn:bc").into())),
    );
    assert_eq!(
        measure_compiler_clone_root_v1(CompilerCloneRootV1::TermDef(&term)).unwrap(),
        expected(7, 0, 11, 4, 2)
    );
}

#[test]
fn scalar_condition_expression_path_and_graph_schedules_are_pinned() {
    let condition = SqlCond::NotExists {
        scans: vec![Scan {
            alias: 1,
            source: (LogicalSource::Table("t".to_owned())).into(),
        }],
        conds: vec![SqlCond::IsNull(ColRef::new(1, "x"))],
    };
    assert_eq!(
        measure_compiler_clone_root_v1(CompilerCloneRootV1::SqlCond(&condition)).unwrap(),
        expected(5, 2, 2, 3, 2)
    );

    let expression = Expression::Or(
        Box::new(Expression::Variable(variable("x"))),
        Box::new(Expression::Variable(variable("yz"))),
    );
    assert_eq!(
        measure_compiler_clone_root_v1(CompilerCloneRootV1::Expression(&expression)).unwrap(),
        expected(5, 0, 3, 3, 2)
    );

    let path = PropertyPathExpression::Alternative(
        Box::new(PropertyPathExpression::NamedNode(iri("urn:p"))),
        Box::new(PropertyPathExpression::NamedNode(iri("urn:qr"))),
    );
    assert_eq!(
        measure_compiler_clone_root_v1(CompilerCloneRootV1::PropertyPath(&path)).unwrap(),
        expected(5, 0, 11, 3, 2)
    );

    let graph = GraphPattern::Bgp {
        patterns: vec![TriplePattern {
            subject: TermPattern::Variable(variable("s")),
            predicate: NamedNodePattern::Variable(variable("p")),
            object: TermPattern::Variable(variable("o")),
        }],
    };
    assert_eq!(
        measure_compiler_clone_root_v1(CompilerCloneRootV1::GraphPattern(&graph)).unwrap(),
        expected(8, 1, 3, 4, 3)
    );
}

#[test]
fn dataset_and_mapping_roots_have_hand_calculated_schedules() {
    let dataset = QueryDataset {
        default: vec![iri("urn:p")],
        named: Some(vec![iri("urn:qr")]),
    };
    assert_eq!(
        measure_compiler_clone_root_v1(CompilerCloneRootV1::QueryDataset(&dataset)).unwrap(),
        expected(3, 2, 11, 2, 2)
    );

    let triples_map = TriplesMap {
        id: "m".to_owned(),
        source: LogicalSource::Table("t".to_owned()),
        subject: SubjectMap {
            term: TermMap::Constant(iri("urn:s").into()),
            classes: Vec::new(),
            graphs: Vec::new(),
        },
        predicate_object_maps: Vec::new(),
    };
    assert_eq!(
        measure_compiler_clone_root_v1(CompilerCloneRootV1::TriplesMap(&triples_map)).unwrap(),
        expected(6, 0, 7, 5, 2)
    );
    assert_eq!(
        measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::TriplesMaps(
            std::slice::from_ref(&triples_map),
        ))
        .unwrap(),
        expected(6, 1, 7, 5, 2)
    );
}

#[test]
fn mapping_root_walks_every_owned_variant() {
    let triples_map = TriplesMap {
        id: "m".to_owned(),
        source: LogicalSource::Query("q".to_owned()),
        subject: SubjectMap {
            term: TermMap::Template(
                Template::from_segments(vec![
                    Segment::Literal("a".into()),
                    Segment::Column("b".into()),
                ])
                .unwrap(),
                TermSpec::plain_literal(),
            ),
            classes: vec![iri("urn:c")],
            graphs: vec![TermMap::Column("g".into(), TermSpec::iri())],
        },
        predicate_object_maps: vec![PredicateObjectMap {
            predicates: vec![TermMap::Constant(iri("urn:p").into())],
            objects: vec![
                ObjectMap::Term(TermMap::Column("o".into(), TermSpec::plain_literal())),
                ObjectMap::Ref(RefObjectMap {
                    parent_triples_map: "parent".to_owned(),
                    joins: vec![Join {
                        child: "child".to_owned(),
                        parent: "parent_col".to_owned(),
                    }],
                }),
            ],
            graphs: vec![TermMap::Template(
                Template::from_segments(vec![Segment::Column("pg".into())]).unwrap(),
                TermSpec::iri(),
            )],
        }],
    };

    let measured =
        measure_compiler_clone_root_v1(CompilerCloneRootV1::TriplesMap(&triples_map)).unwrap();
    assert_eq!(measured, expected(25, 11, 39, 5, 7));
    assert_eq!(
        measured,
        measure_compiler_clone_root_v1(CompilerCloneRootV1::TriplesMap(&triples_map.clone()))
            .unwrap()
    );
}

#[test]
fn one_row_and_rows_roots_have_distinct_exact_schedules() {
    let row = vec![None, Some(TermDef::Const(iri("urn:x").into()))];
    assert_eq!(
        measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::TermDefRow(&row)).unwrap(),
        expected(3, 2, 5, 3, 1)
    );
    assert_eq!(
        measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::TermDefRows(
            std::slice::from_ref(&row),
        ))
        .unwrap(),
        expected(3, 3, 5, 3, 1)
    );
}
