use super::*;

use std::collections::BTreeMap;

use ::spargebra::algebra::{Expression, PropertyPathExpression};
use ::spargebra::term::{
    Literal, NamedNode, NamedNodePattern, TermPattern, TriplePattern, Variable,
};
use sf_core::datatype::XsdTypeCode;
use sf_core::ir::LogicalSource;
use sf_sql::Dialect;

use crate::iq::node::{AggArg, AggDef, BindDef, ColOrConst, IqCond, IqNode, Var};
use crate::iq::{
    AggKind, Branch, ColRef, HopExpr, HopRelation, OrderKey, PathClosure, PathKind, Scan, SqlCond,
    SubPlanJoin, TermDef,
};

fn iri(value: &str) -> NamedNode {
    NamedNode::new(value).unwrap()
}

fn variable(value: &str) -> Variable {
    Variable::new(value).unwrap()
}

fn col(value: &str) -> ColRef {
    ColRef::new(1, value)
}

fn constant(value: &str) -> TermDef {
    TermDef::Const(iri(value).into())
}

fn empty_plan(branches: Vec<Branch>) -> Plan {
    Plan {
        branches,
        form: PlanForm::Ask,
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        rust_group: None,
        dialect: Dialect::Sqlite,
        dedup_scopes: Vec::new(),
        construct_drops_some_branch_var: false,
    }
}

fn exact_limits(measure: PlanMeasureV1) -> PlanMeasureLimits {
    PlanMeasureLimits {
        max_nodes: measure.nodes,
        max_collection_slots: measure.collection_slots,
        max_payload_bytes: measure.payload_bytes,
        max_depth: measure.max_depth,
        max_pending_items: measure.max_pending_items,
    }
}

fn assert_dimension(error: PlanMeasureError, expected: PlanMeasureLimit) {
    match error {
        PlanMeasureError::LimitExceeded { dimension, .. } => assert_eq!(dimension, expected),
        other => panic!("expected {expected:?} limit error, got {other:?}"),
    }
}

fn assert_n_and_n_plus_one(
    measure: PlanMeasureV1,
    mut run: impl FnMut(PlanMeasureLimits) -> Result<PlanMeasureV1, PlanMeasureError>,
) {
    assert_eq!(
        measure.deep_clone_work,
        measure.nodes + measure.collection_slots + measure.payload_bytes
    );
    assert_eq!(run(exact_limits(measure)), Ok(measure));

    let mut limits = exact_limits(measure);
    limits.max_nodes -= 1;
    assert_dimension(run(limits).unwrap_err(), PlanMeasureLimit::Nodes);

    let mut limits = exact_limits(measure);
    limits.max_collection_slots -= 1;
    assert_dimension(run(limits).unwrap_err(), PlanMeasureLimit::CollectionSlots);

    let mut limits = exact_limits(measure);
    limits.max_payload_bytes -= 1;
    assert_dimension(run(limits).unwrap_err(), PlanMeasureLimit::PayloadBytes);

    let mut limits = exact_limits(measure);
    limits.max_depth -= 1;
    assert_dimension(run(limits).unwrap_err(), PlanMeasureLimit::Depth);

    let mut limits = exact_limits(measure);
    limits.max_pending_items -= 1;
    assert_dimension(run(limits).unwrap_err(), PlanMeasureLimit::PendingItems);
}

#[test]
fn branch_forest_clone_root_is_exact_through_nested_subplan() {
    let mut nested_branch = Branch::empty();
    nested_branch.bindings.insert(
        "nested".to_owned(),
        constant("http://example.test/nested-value"),
    );
    let mut outer = Branch::empty();
    outer.subplan_joins.push(SubPlanJoin {
        alias: 2,
        plan: Box::new(empty_plan(vec![nested_branch])),
        on: vec![SqlCond::IsNotNull(col("joined"))],
        left: true,
    });
    let forest = vec![outer];

    let measured = measure_branch_forest_clone_v1(&forest).unwrap();
    assert_eq!(
        measured,
        measure_branch_forest_clone_v1(&forest.clone()).unwrap()
    );
    assert_n_and_n_plus_one(measured, |limits| {
        measure_branch_forest_clone_with_limits(&forest, limits)
    });
    assert!(measured.max_depth >= 5);
}

#[test]
fn iq_node_clone_root_is_exact_through_nested_exists_and_not_exists() {
    let node = IqNode::Filter {
        child: Box::new(IqNode::True),
        cond: vec![
            IqCond::Exists(Box::new(IqNode::Filter {
                child: Box::new(IqNode::True),
                cond: vec![IqCond::NotExists {
                    inner: Box::new(IqNode::Values {
                        vars: vec!["inside".into()],
                        rows: vec![vec![Some(constant("http://example.test/inside"))]],
                    }),
                    is_minus: false,
                }],
            })),
            IqCond::NotExists {
                inner: Box::new(IqNode::Empty {
                    vars: vec!["minus".into()],
                }),
                is_minus: true,
            },
        ],
    };

    let measured = measure_iq_node_clone_v1(&node).unwrap();
    assert_eq!(measured, measure_iq_node_clone_v1(&node.clone()).unwrap());
    assert_n_and_n_plus_one(measured, |limits| {
        Walker::new(limits).run(Work::IqNode(&node))
    });
    assert!(measured.max_depth >= 6);
}

fn all_iq_variants() -> Vec<IqNode> {
    let mut substitution = BTreeMap::new();
    substitution.insert(
        Var::from("resolved"),
        BindDef::Resolved(constant("http://example.test/resolved")),
    );
    substitution.insert(
        Var::from("expression"),
        BindDef::Expr(Box::new(Expression::Variable(variable("source")))),
    );
    let conditions = vec![
        IqCond::Expr(Box::new(Expression::Variable(variable("expr")))),
        IqCond::Sql(SqlCond::IsNull(col("sql"))),
        IqCond::And(vec![IqCond::Sql(SqlCond::IsNotNull(col("and")))]),
        IqCond::Or(vec![IqCond::Sql(SqlCond::IsNull(col("or")))]),
        IqCond::Not(Box::new(IqCond::Sql(SqlCond::IsNull(col("not"))))),
        IqCond::Exists(Box::new(IqNode::True)),
        IqCond::NotExists {
            inner: Box::new(IqNode::Empty {
                vars: vec!["not_exists".into()],
            }),
            is_minus: true,
        },
    ];
    let scan = Scan {
        alias: 1,
        source: LogicalSource::Table("items".to_owned()),
    };
    let relation = HopRelation {
        source: LogicalSource::Query("select s, o from edges".to_owned()),
        subj_col: "s".into(),
        obj_col: "o".into(),
    };
    let closure = PathClosure {
        alias: 3,
        kind: PathKind::OneOrMore,
        hop: HopExpr::Pred(relation),
    };
    let pattern = TriplePattern {
        subject: TermPattern::Variable(variable("s")),
        predicate: NamedNodePattern::NamedNode(iri("http://example.test/p")),
        object: TermPattern::Literal(Literal::new_simple_literal("object")),
    };
    let mut bind = BTreeMap::new();
    bind.insert("raw".into(), ColOrConst::Col("column".into()));
    bind.insert(
        "fixed".into(),
        ColOrConst::Const(iri("http://example.test/fixed").into()),
    );

    vec![
        IqNode::Construction {
            child: Box::new(IqNode::True),
            subst: substitution,
            project: vec!["resolved".into()],
        },
        IqNode::Filter {
            child: Box::new(IqNode::True),
            cond: conditions,
        },
        IqNode::InnerJoin {
            children: vec![IqNode::True, IqNode::Empty { vars: Vec::new() }],
            cond: Vec::new(),
        },
        IqNode::LeftJoin {
            left: Box::new(IqNode::True),
            right: Box::new(IqNode::True),
            cond: Vec::new(),
        },
        IqNode::Union {
            children: vec![IqNode::True],
            project: vec!["union".into()],
        },
        IqNode::Aggregation {
            child: Box::new(IqNode::True),
            grouping: vec!["group".into()],
            aggs: vec![
                AggDef {
                    var: "count".into(),
                    kind: AggKind::Count,
                    arg: Some(AggArg::Var("value".into())),
                    distinct: true,
                    fixed_type: Some(XsdTypeCode::Integer),
                },
                AggDef {
                    var: "sum".into(),
                    kind: AggKind::Sum,
                    arg: Some(AggArg::Expr(constant("http://example.test/arg"))),
                    distinct: false,
                    fixed_type: None,
                },
            ],
        },
        IqNode::Distinct {
            child: Box::new(IqNode::True),
        },
        IqNode::Slice {
            child: Box::new(IqNode::True),
            offset: 1,
            limit: Some(2),
        },
        IqNode::OrderBy {
            child: Box::new(IqNode::True),
            keys: vec![OrderKey {
                var: "ordered".to_owned(),
                descending: true,
                expr: Some(Box::new(Expression::Variable(variable("ordered")))),
            }],
        },
        IqNode::Values {
            vars: vec!["value".into()],
            rows: vec![vec![Some(constant("http://example.test/value"))]],
        },
        IqNode::Extensional { scan, bind },
        IqNode::Intensional {
            pattern: pattern.clone(),
            graph: Some(NamedNodePattern::Variable(variable("graph"))),
        },
        IqNode::UnresolvedPath {
            subject: pattern.subject,
            path: PropertyPathExpression::NamedNode(iri("http://example.test/path")),
            object: pattern.object,
            graph: Some(NamedNodePattern::NamedNode(iri(
                "http://example.test/graph",
            ))),
        },
        IqNode::Empty {
            vars: vec!["empty".into()],
        },
        IqNode::True,
        IqNode::Path { closure },
    ]
}

#[test]
fn every_iq_node_condition_and_payload_variant_is_covered() {
    let nodes = all_iq_variants();
    let measured = measure_iq_fragment_clone_v1(IqCloneFragmentV1::Nodes(&nodes)).unwrap();
    let cloned = nodes.clone();
    assert_eq!(
        measured,
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::Nodes(&cloned)).unwrap()
    );
    assert!(measured.nodes > nodes.len() as u64);
    assert!(measured.collection_slots > nodes.len() as u64);
    assert!(measured.payload_bytes > 100);

    assert_n_and_n_plus_one(measured, |limits| {
        measure_iq_fragment_clone_with_limits(IqCloneFragmentV1::Nodes(&nodes), limits)
    });
}

#[test]
fn every_exact_iq_fragment_root_matches_its_clone() {
    let nodes = vec![IqNode::True];
    let node_clones = nodes.clone();
    assert_eq!(
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::Nodes(&nodes)).unwrap(),
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::Nodes(&node_clones)).unwrap()
    );

    let conditions = vec![IqCond::NotExists {
        inner: Box::new(IqNode::True),
        is_minus: false,
    }];
    let condition_clones = conditions.clone();
    assert_eq!(
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::Conditions(&conditions)).unwrap(),
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::Conditions(&condition_clones)).unwrap()
    );

    let substitution = BTreeMap::from([(
        Var::from("bound"),
        BindDef::Resolved(constant("http://example.test/bound")),
    )]);
    let substitution_clone = substitution.clone();
    assert_eq!(
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::Substitution(&substitution)).unwrap(),
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::Substitution(&substitution_clone)).unwrap()
    );

    let variables: Vec<Var> = vec!["alpha".into(), "beta".into()];
    let variable_clones = variables.clone();
    assert_eq!(
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::Variables(&variables)).unwrap(),
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::Variables(&variable_clones)).unwrap()
    );

    let keys = vec![OrderKey {
        var: "key".to_owned(),
        descending: false,
        expr: Some(Box::new(Expression::Variable(variable("key")))),
    }];
    let key_clones = keys.clone();
    assert_eq!(
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::OrderKeys(&keys)).unwrap(),
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::OrderKeys(&key_clones)).unwrap()
    );

    let rows = vec![vec![Some(constant("http://example.test/cell")), None]];
    let row_clones = rows.clone();
    assert_eq!(
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::ValueRows(&rows)).unwrap(),
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::ValueRows(&row_clones)).unwrap()
    );
}

#[test]
fn exact_clone_roots_have_hand_calculated_v1_schedules() {
    let branches = vec![Branch::empty()];
    assert_eq!(
        measure_branch_forest_clone_v1(&branches).unwrap(),
        PlanMeasureV1 {
            nodes: 1,
            collection_slots: 1,
            payload_bytes: 0,
            deep_clone_work: 2,
            max_depth: 1,
            max_pending_items: 1,
        }
    );

    let nodes = vec![IqNode::True];
    assert_eq!(
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::Nodes(&nodes)).unwrap(),
        PlanMeasureV1 {
            nodes: 1,
            collection_slots: 1,
            payload_bytes: 0,
            deep_clone_work: 2,
            max_depth: 1,
            max_pending_items: 1,
        }
    );

    let conditions = vec![IqCond::Sql(SqlCond::IsNull(col("x")))];
    assert_eq!(
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::Conditions(&conditions)).unwrap(),
        PlanMeasureV1 {
            nodes: 3,
            collection_slots: 1,
            payload_bytes: 1,
            deep_clone_work: 5,
            max_depth: 3,
            max_pending_items: 1,
        }
    );

    let substitution = BTreeMap::from([(Var::from("v"), BindDef::Resolved(constant("http://x")))]);
    assert_eq!(
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::Substitution(&substitution)).unwrap(),
        PlanMeasureV1 {
            nodes: 4,
            collection_slots: 1,
            payload_bytes: 9,
            deep_clone_work: 14,
            max_depth: 4,
            max_pending_items: 1,
        }
    );

    let variables: Vec<Var> = vec!["a".into(), "β".into()];
    assert_eq!(
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::Variables(&variables)).unwrap(),
        PlanMeasureV1 {
            nodes: 0,
            collection_slots: 2,
            payload_bytes: 3,
            deep_clone_work: 5,
            max_depth: 0,
            max_pending_items: 0,
        }
    );

    let keys = vec![OrderKey {
        var: "key".to_owned(),
        descending: false,
        expr: None,
    }];
    assert_eq!(
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::OrderKeys(&keys)).unwrap(),
        PlanMeasureV1 {
            nodes: 1,
            collection_slots: 1,
            payload_bytes: 3,
            deep_clone_work: 5,
            max_depth: 1,
            max_pending_items: 1,
        }
    );

    let rows = vec![vec![None, Some(constant("http://x"))]];
    assert_eq!(
        measure_iq_fragment_clone_v1(IqCloneFragmentV1::ValueRows(&rows)).unwrap(),
        PlanMeasureV1 {
            nodes: 3,
            collection_slots: 3,
            payload_bytes: 8,
            deep_clone_work: 14,
            max_depth: 3,
            max_pending_items: 1,
        }
    );
}
