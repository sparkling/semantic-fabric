use spargebra::algebra::{AggregateExpression, AggregateFunction, Expression, GraphPattern};
use spargebra::term::{GroundTerm, Literal};

use crate::iq::node::{AggArg, AggDef, IqNode};
use crate::iq::{AggKind, TermDef};
use crate::Error;

use super::build_tree;
use super::test_support::{bgp, pattern, triple, var, vec_var};

#[test]
fn group_builds_aggregation_with_scope() {
    let g = GraphPattern::Group {
        inner: Box::new(bgp(vec![triple("s", "p", "o")])),
        variables: vec![var("s")],
        aggregates: vec![(
            var("c"),
            AggregateExpression::CountSolutions { distinct: false },
        )],
    };
    let t = build_tree(&g, None).unwrap();
    let IqNode::Aggregation { grouping, aggs, .. } = &t else {
        panic!("expected Aggregation, got {t:?}");
    };
    assert_eq!(*grouping, vec_var(&["s"]));
    assert!(matches!(
        aggs.as_slice(),
        [AggDef {
            kind: AggKind::Count,
            arg: None,
            distinct: false,
            ..
        }]
    ));
    // the node owns its scope: grouping keys ++ aggregate output vars.
    assert_eq!(t.output_vars(), vec_var(&["s", "c"]));
}

#[test]
fn group_sum_of_variable_builds_agg_arg_var() {
    // SUM(?o): a bare-variable argument resolves context-free to AggArg::Var, and
    // SUM pins no result type (fixed_type None — unlike COUNT's xsd:integer).
    let g = GraphPattern::Group {
        inner: Box::new(bgp(vec![triple("s", "p", "o")])),
        variables: vec![var("s")],
        aggregates: vec![(
            var("t"),
            AggregateExpression::FunctionCall {
                name: AggregateFunction::Sum,
                expr: Expression::Variable(var("o")),
                distinct: false,
            },
        )],
    };
    let t = build_tree(&g, None).unwrap();
    let IqNode::Aggregation { aggs, .. } = &t else {
        panic!("expected Aggregation, got {t:?}");
    };
    assert!(matches!(
        aggs.as_slice(),
        [AggDef {
            kind: AggKind::Sum,
            arg: Some(AggArg::Var(_)),
            distinct: false,
            fixed_type: None,
            ..
        }]
    ));
    // grouping key ++ the aggregate output var.
    assert_eq!(t.output_vars(), vec_var(&["s", "t"]));
}

#[test]
fn count_distinct_star_rides_distinct_flag() {
    // The tree expresses COUNT(DISTINCT *) (the flat model 501'd it).
    let g = GraphPattern::Group {
        inner: Box::new(bgp(vec![triple("s", "p", "o")])),
        variables: vec![],
        aggregates: vec![(
            var("c"),
            AggregateExpression::CountSolutions { distinct: true },
        )],
    };
    let t = build_tree(&g, None).unwrap();
    let IqNode::Aggregation { aggs, .. } = &t else {
        panic!("expected Aggregation");
    };
    assert!(matches!(
        aggs.as_slice(),
        [AggDef {
            kind: AggKind::Count,
            arg: None,
            distinct: true,
            ..
        }]
    ));
}

#[test]
fn group_concat_and_sample_are_tracked_501() {
    for f in [
        AggregateFunction::GroupConcat { separator: None },
        AggregateFunction::Sample,
    ] {
        let g = GraphPattern::Group {
            inner: Box::new(bgp(vec![triple("s", "p", "o")])),
            variables: vec![],
            aggregates: vec![(
                var("c"),
                AggregateExpression::FunctionCall {
                    name: f,
                    expr: Expression::Variable(var("o")),
                    distinct: false,
                },
            )],
        };
        assert!(matches!(build_tree(&g, None), Err(Error::Unsupported(_))));
    }
}

#[test]
fn project_builds_construction_with_empty_subst() {
    // Project(P, vars) → a Construction that adds NO substitution, carrying only
    // the declared projection (design §2 Project arm); its output scope is exactly
    // that projection.
    let p = GraphPattern::Project {
        inner: Box::new(bgp(vec![triple("s", "p", "o")])),
        variables: vec![var("s"), var("o")],
    };
    let t = build_tree(&p, None).unwrap();
    let IqNode::Construction {
        subst,
        project,
        child,
    } = &t
    else {
        panic!("expected Construction, got {t:?}");
    };
    assert!(subst.is_empty(), "Project adds no substitution");
    assert_eq!(*project, vec_var(&["s", "o"]));
    assert!(matches!(child.as_ref(), IqNode::Intensional { .. }));
    assert_eq!(t.output_vars(), vec_var(&["s", "o"]));
}

#[test]
fn distinct_and_reduced_both_build_distinct() {
    // spargebra wraps the projection: `SELECT DISTINCT *` ⇒ Distinct{ Project{…} },
    // so the built tree is Distinct over a Construction.
    let d = build_tree(&pattern("SELECT DISTINCT * WHERE { ?s ?p ?o }"), None).unwrap();
    let IqNode::Distinct { child } = &d else {
        panic!("expected Distinct, got {d:?}");
    };
    assert!(matches!(child.as_ref(), IqNode::Construction { .. }));

    // REDUCED also builds a Distinct (REDUCED may dedup — sound).
    let r = build_tree(&pattern("SELECT REDUCED * WHERE { ?s ?p ?o }"), None).unwrap();
    let IqNode::Distinct { child } = &r else {
        panic!("expected Distinct, got {r:?}");
    };
    assert!(matches!(child.as_ref(), IqNode::Construction { .. }));
}

#[test]
fn slice_carries_offset_and_limit() {
    let t = build_tree(
        &pattern("SELECT * WHERE { ?s ?p ?o } LIMIT 5 OFFSET 2"),
        None,
    )
    .unwrap();
    // Slice sits over the Project (spargebra: Slice{ Project{ ... } }).
    let IqNode::Slice { offset, limit, .. } = &t else {
        panic!("expected Slice, got {t:?}");
    };
    assert_eq!(*offset, 2);
    assert_eq!(*limit, Some(5));
}

#[test]
fn order_by_variable_and_expression_keys() {
    let t = build_tree(
        &pattern("SELECT * WHERE { ?s ?p ?o } ORDER BY ?o DESC(STRLEN(?o))"),
        None,
    )
    .unwrap();
    let IqNode::Construction { child, .. } = &t else {
        panic!("expected Project, got {t:?}");
    };
    let IqNode::OrderBy { keys, .. } = child.as_ref() else {
        panic!("expected OrderBy, got {child:?}");
    };
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].var, "o");
    assert!(keys[0].expr.is_none() && !keys[0].descending);
    assert!(keys[1].expr.is_some() && keys[1].descending);
}

#[test]
fn values_lowers_const_and_undef_cells() {
    let v = GraphPattern::Values {
        variables: vec![var("x")],
        bindings: vec![
            vec![Some(GroundTerm::Literal(Literal::new_simple_literal("a")))],
            vec![None],
        ],
    };
    let t = build_tree(&v, None).unwrap();
    let IqNode::Values { vars, rows } = &t else {
        panic!("expected Values, got {t:?}");
    };
    assert_eq!(*vars, vec_var(&["x"]));
    assert!(matches!(rows[0].as_slice(), [Some(TermDef::Const(_))]));
    assert!(matches!(rows[1].as_slice(), [None]));
    assert_eq!(t.output_vars(), vec_var(&["x"]));
}

#[test]
fn unsupported_pattern_is_501() {
    // SERVICE is out of v1 coverage → 501 (never silently dropped).
    let r = build_tree(
        &pattern("SELECT * WHERE { SERVICE <http://x/> { ?s ?p ?o } }"),
        None,
    );
    assert!(matches!(r, Err(Error::Unsupported(_))), "{r:?}");
}

#[test]
fn output_vars_flows_through_a_representative_tree() {
    // Capstone over IqNode::output_vars (design §1 scope rules): Slice/Distinct are
    // scope-transparent, and a LeftJoin keeps right-only vars in scope (nullable),
    // de-duplicating the shared ?o. Assemble Slice{ Distinct{ LeftJoin{ {s,p,o},
    // {o,p2,x} } } } from builder-produced leaves and assert the merged scope.
    let tree = IqNode::Slice {
        child: Box::new(IqNode::Distinct {
            child: Box::new(IqNode::LeftJoin {
                left: Box::new(build_tree(&bgp(vec![triple("s", "p", "o")]), None).unwrap()),
                right: Box::new(build_tree(&bgp(vec![triple("o", "p2", "x")]), None).unwrap()),
                cond: Vec::new(),
            }),
        }),
        offset: 0,
        limit: Some(10),
    };
    assert_eq!(tree.output_vars(), vec_var(&["s", "p", "o", "p2", "x"]));
}
