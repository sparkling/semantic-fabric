use super::*;

use std::collections::BTreeMap;

use ::spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, Function, GraphPattern, OrderExpression,
};
use ::spargebra::term::{
    BlankNode, GroundTerm, Literal, NamedNode, NamedNodePattern, TermPattern, TriplePattern,
    Variable,
};
use sf_core::datatype::XsdTypeCode;
use sf_core::ir::{LogicalSource, Segment, Template, TermMap, TermSpec};
use sf_sql::Dialect;

use crate::iq::{
    AggCol, AggKind, Aggregation, Branch, CmpOp, ColRef, GroupKey, HopExpr, HopRelation, OptJoin,
    OrderKey, PathClosure, PathKind, R2rmlGraphScope, RustAgg, RustGroup, Scan, SqlCond,
    StrMatchOp, SubPlanJoin, TermDef,
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

fn source(value: &str) -> LogicalSource {
    LogicalSource::Table(value.to_owned())
}

fn empty_plan(form: PlanForm) -> Plan {
    Plan {
        branches: Vec::new(),
        form,
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

fn dimension(error: PlanMeasureError) -> PlanMeasureLimit {
    match error {
        PlanMeasureError::LimitExceeded { dimension, .. } => dimension,
        other => panic!("expected a limit error, got {other:?}"),
    }
}

#[test]
fn empty_ask_has_a_small_exact_schedule() {
    let measure = PlanMeasureV1::measure(&empty_plan(PlanForm::Ask)).unwrap();

    assert_eq!(measure.nodes, 2); // Plan + PlanForm
    assert_eq!(measure.collection_slots, 0);
    assert_eq!(measure.payload_bytes, 0);
    assert_eq!(measure.deep_clone_work, 2);
    assert_eq!(measure.max_depth, 2);
    assert_eq!(measure.max_pending_items, 1);
}

#[test]
fn every_limit_is_inclusive_and_rejects_n_plus_one() {
    let mut plan = empty_plan(PlanForm::Select {
        vars: vec!["alpha".to_owned(), "beta".to_owned()],
    });
    plan.branches.push(Branch::empty());
    let measured = PlanMeasureV1::measure(&plan).unwrap();

    assert_eq!(
        Walker::new(exact_limits(measured)).run(Work::Plan(&plan)),
        Ok(measured)
    );

    let mut limits = exact_limits(measured);
    limits.max_nodes -= 1;
    assert_eq!(
        dimension(Walker::new(limits).run(Work::Plan(&plan)).unwrap_err()),
        PlanMeasureLimit::Nodes
    );

    let mut limits = exact_limits(measured);
    limits.max_collection_slots -= 1;
    assert_eq!(
        dimension(Walker::new(limits).run(Work::Plan(&plan)).unwrap_err()),
        PlanMeasureLimit::CollectionSlots
    );

    let mut limits = exact_limits(measured);
    limits.max_payload_bytes -= 1;
    assert_eq!(
        dimension(Walker::new(limits).run(Work::Plan(&plan)).unwrap_err()),
        PlanMeasureLimit::PayloadBytes
    );

    let mut limits = exact_limits(measured);
    limits.max_depth -= 1;
    assert_eq!(
        dimension(Walker::new(limits).run(Work::Plan(&plan)).unwrap_err()),
        PlanMeasureLimit::Depth
    );

    let mut limits = exact_limits(measured);
    limits.max_pending_items -= 1;
    assert_eq!(
        dimension(Walker::new(limits).run(Work::Plan(&plan)).unwrap_err()),
        PlanMeasureLimit::PendingItems
    );
}

#[test]
fn representative_plan_graph_is_deterministic_across_deep_clone() {
    let spec = TermSpec::typed_literal(iri("http://example.test/type"))
        .with_base("http://example.test/base/");
    let template = Template::from_segments(vec![
        Segment::Literal("http://example.test/id/".into()),
        Segment::Column("id".into()),
    ])
    .unwrap();
    let template_map = TermMap::Template(template, spec.clone());
    let column_map = TermMap::Column("label".into(), TermSpec::lang_literal("en"));
    let constant = || TermDef::Const(iri("http://example.test/value").into());
    let composed = TermDef::ComposedTriple {
        subject: Box::new(constant()),
        predicate: Box::new(constant()),
        object: Box::new(TermDef::Coalesce(
            Box::new(TermDef::Derived {
                term_map: template_map.clone(),
                alias: 1,
            }),
            Box::new(TermDef::Concat(vec![constant()])),
        )),
    };
    let mut bindings = BTreeMap::new();
    bindings.insert("triple".to_owned(), composed);
    bindings.insert(
        "blank".to_owned(),
        TermDef::R2rmlBlank {
            term_map: column_map.clone(),
            alias: 1,
            graph: R2rmlGraphScope::Mapped {
                term_map: template_map,
                alias: 1,
            },
        },
    );
    bindings.insert(
        "aggregate".to_owned(),
        TermDef::Agg {
            col: col("agg"),
            kind: AggKind::Count,
            operand: Some(col("value")),
            fixed_type: Some(XsdTypeCode::Integer),
        },
    );

    let relation = HopRelation {
        source: LogicalSource::Query("select s, o from edge".to_owned()),
        subj_col: "s".into(),
        obj_col: "o".into(),
    };
    let path = PathClosure {
        alias: 7,
        kind: PathKind::OneOrMore,
        hop: HopExpr::Seq(
            Box::new(HopExpr::Inverse(Box::new(HopExpr::Pred(relation.clone())))),
            Box::new(HopExpr::Alt(vec![
                HopExpr::Pred(relation.clone()),
                HopExpr::Nps(vec![HopExpr::Pred(relation)]),
            ])),
        ),
    };
    let scan = Scan {
        alias: 1,
        source: (source("things")).into(),
    };
    let nested = empty_plan(PlanForm::Select {
        vars: vec!["inner".to_owned()],
    });
    let conditions = vec![
        SqlCond::ColEq(col("a"), col("b")),
        SqlCond::NullSafeEq(col("c"), col("d")),
        SqlCond::Cmp(col("e"), CmpOp::Eq, "bound".to_owned()),
        SqlCond::StrMatch {
            col: col("f"),
            op: StrMatchOp::RegexMatch,
            param: "^safe$".to_owned(),
        },
        SqlCond::Not(Box::new(SqlCond::And(vec![
            SqlCond::IsNotNull(col("g")),
            SqlCond::IsNull(col("h")),
        ]))),
        SqlCond::NotExists {
            scans: vec![scan.clone()],
            conds: vec![SqlCond::Or(vec![SqlCond::IsNull(col("i"))])],
        },
        SqlCond::Exists {
            scans: vec![scan.clone()],
            conds: Vec::new(),
        },
        SqlCond::PathExists {
            pc: path.clone(),
            conds: vec![SqlCond::IsNotNull(col("j"))],
            negated: true,
        },
        SqlCond::TemplateEq(
            vec![Segment::Literal("left".into())],
            1,
            vec![Segment::Column("right".into())],
            2,
            true,
        ),
    ];
    let branch = Branch {
        core: vec![scan.clone()],
        opts: vec![OptJoin {
            scan,
            on: vec![SqlCond::IsNotNull(col("optional"))],
            extra: Vec::new(),
        }],
        bindings,
        where_conds: conditions,
        distinct: true,
        limit: Some(10),
        offset: 2,
        order: vec![OrderKey {
            var: "branch_order".to_owned(),
            descending: true,
            expr: Some(Box::new(Expression::FunctionCall(
                Function::Custom(iri("http://example.test/function")),
                vec![Expression::Literal(Literal::new_typed_literal(
                    "42",
                    iri("http://www.w3.org/2001/XMLSchema#integer"),
                ))],
            ))),
        }],
        path: Some(path),
        agg: Some(Aggregation {
            keys: vec![GroupKey {
                var: "key".to_owned(),
                cols: vec![col("key_col")],
            }],
            aggs: vec![AggCol {
                var: "count".to_owned(),
                kind: AggKind::Count,
                arg: Some(col("arg")),
                distinct: true,
                out: col("out"),
                fixed_type: Some(XsdTypeCode::Integer),
            }],
        }),
        subplan_joins: vec![SubPlanJoin {
            alias: 9,
            plan: Box::new(nested),
            on: vec![SqlCond::ColEq(col("outer"), col("inner"))],
            left: false,
        }],
        nps: true,
    };

    let x = variable("x");
    let exists = GraphPattern::Group {
        inner: Box::new(GraphPattern::OrderBy {
            inner: Box::new(GraphPattern::Values {
                variables: vec![x.clone()],
                bindings: vec![vec![Some(GroundTerm::Literal(
                    Literal::new_simple_literal("value"),
                ))]],
            }),
            expression: vec![OrderExpression::Desc(Expression::Variable(x.clone()))],
        }),
        variables: vec![x.clone()],
        aggregates: vec![(
            variable("joined"),
            AggregateExpression::FunctionCall {
                name: AggregateFunction::GroupConcat {
                    separator: Some(" | ".to_owned()),
                },
                expr: Expression::Variable(x),
                distinct: true,
            },
        )],
    };
    let embedded = TriplePattern {
        subject: TermPattern::BlankNode(BlankNode::new("nested").unwrap()),
        predicate: NamedNodePattern::NamedNode(iri("http://example.test/p")),
        object: TermPattern::Literal(Literal::new_simple_literal("object")),
    };
    let mut plan = empty_plan(PlanForm::Construct {
        template: vec![TriplePattern {
            subject: TermPattern::Triple(Box::new(embedded)),
            predicate: NamedNodePattern::Variable(variable("predicate")),
            object: TermPattern::Variable(variable("object")),
        }],
    });
    plan.branches.push(branch);
    plan.order.push(OrderKey {
        var: "global_order".to_owned(),
        descending: false,
        expr: Some(Box::new(Expression::Exists(Box::new(exists)))),
    });
    plan.rust_group = Some(RustGroup {
        keys: vec!["key".to_owned()],
        aggs: vec![RustAgg {
            out_var: "count".to_owned(),
            kind: AggKind::Count,
            arg_var: Some("value".to_owned()),
            distinct: true,
            fixed_type: Some(XsdTypeCode::Integer),
        }],
        post_exprs: vec![(
            "post".to_owned(),
            Expression::If(
                Box::new(Expression::Bound(variable("count"))),
                Box::new(Expression::Literal(Literal::new_simple_literal("yes"))),
                Box::new(Expression::Literal(Literal::new_simple_literal("no"))),
            ),
        )],
    });
    plan.dedup_scopes.push(Some(DedupScope {
        group_id: 1,
        key_bindings: BTreeMap::from([("key".to_owned(), constant())]),
    }));

    let original = PlanMeasureV1::measure(&plan).unwrap();
    let cloned = PlanMeasureV1::measure(&plan.clone()).unwrap();
    assert_eq!(original, cloned);
    assert_eq!(
        original.deep_clone_work,
        original.nodes + original.collection_slots + original.payload_bytes
    );
    assert!(original.nodes > 100);
    assert!(original.collection_slots > 40);
    assert!(original.payload_bytes > 200);
}

#[test]
fn nested_expression_is_walked_iteratively_but_depth_is_fail_closed() {
    fn nested_not(levels: usize) -> Expression {
        let mut expr = Expression::Variable(variable("leaf"));
        for _ in 0..levels {
            expr = Expression::Not(Box::new(expr));
        }
        expr
    }

    let mut plan = empty_plan(PlanForm::Ask);
    plan.order.push(OrderKey {
        var: "deep".to_owned(),
        descending: false,
        expr: Some(Box::new(nested_not(80))),
    });
    let measured = PlanMeasureV1::measure(&plan).expect("80 levels fit the V1 envelope");
    assert!(measured.max_depth > 80);

    let mut limits = PlanMeasureLimits::V1;
    limits.max_depth = 16;
    assert_eq!(
        dimension(Walker::new(limits).run(Work::Plan(&plan)).unwrap_err()),
        PlanMeasureLimit::Depth
    );
}

#[test]
fn wide_graph_hits_pending_bound_before_another_stack_growth() {
    let mut plan = empty_plan(PlanForm::Ask);
    plan.branches = vec![Branch::empty(), Branch::empty(), Branch::empty()];
    let mut limits = PlanMeasureLimits::V1;
    limits.max_pending_items = 2;

    assert_eq!(
        dimension(Walker::new(limits).run(Work::Plan(&plan)).unwrap_err()),
        PlanMeasureLimit::PendingItems
    );
}

#[test]
fn arithmetic_overflow_fails_closed_without_wrapping_work() {
    let mut walker = Walker::new(PlanMeasureLimits {
        max_nodes: u64::MAX,
        max_collection_slots: u64::MAX,
        max_payload_bytes: u64::MAX,
        max_depth: usize::MAX,
        max_pending_items: usize::MAX,
    });
    walker.measure.deep_clone_work = u64::MAX;

    assert_eq!(
        walker.record_node(),
        Err(PlanMeasureError::AccountingOverflow)
    );
    assert_eq!(walker.measure.nodes, 0);
    assert_eq!(walker.measure.deep_clone_work, u64::MAX);
}

#[test]
fn typed_literal_accounts_for_hidden_datatype_clone_without_display() {
    let datatype = "http://example.test/a-long-datatype";
    let simple = Literal::new_simple_literal("value");
    let typed = Literal::new_typed_literal("value", iri(datatype));
    let mut simple_plan = empty_plan(PlanForm::Ask);
    let mut typed_plan = empty_plan(PlanForm::Ask);
    simple_plan.order.push(OrderKey {
        var: String::new(),
        descending: false,
        expr: Some(Box::new(Expression::Literal(simple))),
    });
    typed_plan.order.push(OrderKey {
        var: String::new(),
        descending: false,
        expr: Some(Box::new(Expression::Literal(typed))),
    });

    let simple = PlanMeasureV1::measure(&simple_plan).unwrap();
    let typed = PlanMeasureV1::measure(&typed_plan).unwrap();
    assert_eq!(typed.nodes, simple.nodes + 1);
    assert_eq!(
        typed.payload_bytes,
        simple.payload_bytes + datatype.len() as u64
    );
}
