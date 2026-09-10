use std::sync::atomic::{AtomicUsize, Ordering};

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, GraphPattern, PropertyPathExpression,
};
use spargebra::term::{GroundTerm, Literal, NamedNodePattern, TermPattern};

use super::control::{BuildVec, BuildWork};
use super::test_support::{bgp, iri, pattern, triple, var};
use super::{build_tree, build_tree_with_work_control};
use crate::compiler_control::CompileContext;
use crate::plan_measure::clone_root::{measure_compiler_clone_root_v1, CompilerCloneRootV1};
use crate::{CompilerWorkMode, Error};

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn metered(control: &dyn QueryControl) -> BuildWork<'_> {
    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control)))
}

fn fixtures() -> Vec<GraphPattern> {
    let mut out: Vec<_> = [
        "SELECT * WHERE { ?s ?p ?o }",
        "SELECT * WHERE { ?s ?p ?o . ?s ?q ?z }",
        "SELECT DISTINCT ?s WHERE { ?s ?p ?o } ORDER BY ?s LIMIT 2 OFFSET 1",
        "SELECT REDUCED ?s WHERE { ?s ?p ?o } ORDER BY DESC(STR(?s))",
        "SELECT * WHERE { { ?s ?p ?o } UNION { ?s ?p ?other } }",
        "SELECT * WHERE { ?s ?p ?o OPTIONAL { ?s ?q ?x FILTER(?x > 1) } }",
        "SELECT * WHERE { ?s ?p ?o MINUS { ?s ?p ?x } }",
        "SELECT * WHERE { GRAPH ?g { ?s <http://example.test/p>+ ?o } }",
        "SELECT * WHERE { GRAPH <http://example.test/g> { ?s ?p ?o FILTER EXISTS { ?s ?q ?x } } }",
        "SELECT * WHERE { ?s ?p ?o FILTER((?o > 1 && ?o < 9) || !(?o = 2)) }",
        "SELECT * WHERE { ?s ?p ?o FILTER(?o > 1 && !EXISTS { ?s ?q ?x }) }",
        "SELECT * WHERE { ?s ?p ?o BIND(CONCAT(STR(?o), 'café 東京') AS ?label) }",
        "SELECT ?s (COUNT(*) AS ?n) (SUM(?o) AS ?sum) (AVG(2) AS ?avg) WHERE { ?s ?p ?o } GROUP BY ?s",
        "SELECT * WHERE { VALUES (?x ?y) { (UNDEF 2) ('café 東京' UNDEF) } }",
    ].iter().map(|q| pattern(q)).collect();
    out.extend([
        bgp(vec![]),
        GraphPattern::Join {
            left: Box::new(bgp(vec![])),
            right: Box::new(bgp(vec![triple("s", "p", "o")])),
        },
        GraphPattern::LeftJoin {
            left: Box::new(bgp(vec![])),
            right: Box::new(bgp(vec![])),
            expression: None,
        },
        GraphPattern::Path {
            subject: TermPattern::Variable(var("s")),
            path: PropertyPathExpression::NamedNode(iri("http://example.test/p")),
            object: TermPattern::Variable(var("o")),
        },
        // Direct callers may provide ragged rows: BUILD preserves the old structure.
        GraphPattern::Values {
            variables: vec![var("x"), var("y")],
            bindings: vec![
                vec![],
                vec![None],
                vec![Some(GroundTerm::Literal(Literal::from("東京"))), None],
            ],
        },
    ]);
    out
}

#[test]
fn every_supported_build_arm_preserves_raw_tree_at_exact_work_boundary() {
    for (index, gp) in fixtures().iter().enumerate() {
        let raw = format!("{:?}", build_tree(gp, None).unwrap());
        let observed = budget(u64::MAX);
        let actual = build_tree_with_work_control(gp, None, &observed).unwrap();
        assert_eq!(format!("{actual:?}"), raw, "fixture {index}");
        let work = observed.consumed(QueryCharge::CompilerWork);
        assert!(work > 0);
        let exact = budget(work);
        assert_eq!(
            format!(
                "{:?}",
                build_tree_with_work_control(gp, None, &exact).unwrap()
            ),
            raw
        );
        assert_eq!(exact.consumed(QueryCharge::CompilerWork), work);
        let short = budget(work - 1);
        assert!(
            matches!(
                build_tree_with_work_control(gp, None, &short),
                Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
            ),
            "fixture {index}"
        );
        assert!(short.consumed(QueryCharge::CompilerWork) < work);
        assert_eq!(observed.consumed(QueryCharge::SourceWork), 0);
    }
}

struct StopAtCharge {
    budget: QueryBudget,
    calls: AtomicUsize,
    stop: usize,
    cause: QueryControlError,
}

impl QueryControl for StopAtCharge {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        self.budget.checkpoint()
    }
    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        self.budget.consume(charge, amount)?;
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.stop {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
}

#[test]
fn nested_graph_exists_and_not_exists_observe_one_sticky_control_at_every_charge() {
    let gp = pattern("SELECT * WHERE { GRAPH ?g { ?s ?p ?o FILTER(EXISTS { GRAPH ?h { ?s ?p ?x } } && !EXISTS { ?x ?p ?o }) } }");
    let observed = StopAtCharge {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        stop: usize::MAX,
        cause: QueryControlError::Cancelled,
    };
    build_tree_with_work_control(&gp, None, &observed).unwrap();
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for stop in 1..=observed.calls.load(Ordering::SeqCst) {
            let control = StopAtCharge {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                stop,
                cause,
            };
            assert!(
                matches!(build_tree_with_work_control(&gp, None, &control), Err(Error::QueryControl(actual)) if actual == cause)
            );
            assert_eq!(
                control.calls.load(Ordering::SeqCst),
                stop,
                "must stop at the first observation"
            );
            assert_eq!(
                control.terminate(QueryControlError::CompilerWorkExceeded),
                cause
            );
        }
    }
}

#[test]
fn stable_scope_pays_utf8_equality_duplicates_and_logical_growth() {
    let gp = pattern(
        "SELECT * WHERE { { ?café ?東京 ?same } UNION { ?東京 ?café ?else } BIND(1 AS ?extra) }",
    );
    let tree = build_tree(&gp, None).unwrap();
    let control = budget(u64::MAX);
    assert_eq!(
        super::scope::output_vars(&tree, metered(&control)).unwrap(),
        tree.output_vars()
    );
    // Isolate stable scope merging, including different names with equal byte length.
    let tree = crate::iq::node::IqNode::InnerJoin {
        children: vec![
            crate::iq::node::IqNode::Values {
                vars: vec!["東京".into(), "café".into(), "東京".into()],
                rows: vec![],
            },
            crate::iq::node::IqNode::Values {
                vars: vec!["京都".into(), "café".into()],
                rows: vec![],
            },
        ],
        cond: vec![],
    };
    let control = budget(u64::MAX);
    let vars = super::scope::output_vars(&tree, metered(&control)).unwrap();
    assert_eq!(vars, tree.output_vars());
    assert_eq!(
        vars.iter().map(|v| v.as_ref()).collect::<Vec<_>>(),
        ["東京", "café", "京都"]
    );
    let short = budget(control.consumed(QueryCharge::CompilerWork) - 1);
    assert!(super::scope::output_vars(&tree, metered(&short)).is_err());

    let mut measured = Vec::new();
    for physical in [0, 64] {
        let control = budget(u64::MAX);
        let mut out = BuildVec::new(Vec::<u64>::with_capacity(physical));
        for value in 0..9 {
            metered(&control).push(&mut out, value).unwrap();
        }
        assert_eq!(out.values, (0..9).collect::<Vec<_>>());
        measured.push(control.consumed(QueryCharge::CompilerWork));
    }
    assert_eq!(
        measured[0], measured[1],
        "allocator slack cannot grant unpaid logical growth"
    );
}

#[test]
fn prospective_growth_and_source_bound_copy_fail_before_mutation() {
    let mut values = Vec::with_capacity(64);
    values.push(7_u64);
    let mut out = BuildVec::new(values);
    let control = budget(1);
    assert!(metered(&control).push(&mut out, 8).is_err());
    assert_eq!(out.values, [7]);
    assert_eq!(control.consumed(QueryCharge::CompilerWork), 1);
    let source = Literal::from("東京".repeat(1024));
    let copy = measure_compiler_clone_root_v1(CompilerCloneRootV1::Literal(&source))
        .unwrap()
        .deep_clone_work;
    let short = budget(copy);
    assert!(metered(&short).copied(&source).is_err());
    assert_eq!(
        short.consumed(QueryCharge::CompilerWork),
        1,
        "measurement precedes the refused copy"
    );
    let exact = budget(1 + copy);
    assert_eq!(metered(&exact).copied(&source).unwrap(), source);
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), 1 + copy);
}

#[test]
fn controlled_uncached_build_checks_depth_and_preexisting_terminal_causes() {
    fn wrap(gp: GraphPattern, shape: usize) -> GraphPattern {
        match shape {
            0 => GraphPattern::Graph {
                name: NamedNodePattern::Variable(var("g")),
                inner: Box::new(gp),
            },
            1 => GraphPattern::Distinct {
                inner: Box::new(gp),
            },
            2 => GraphPattern::Join {
                left: Box::new(gp),
                right: Box::new(bgp(vec![])),
            },
            3 => GraphPattern::LeftJoin {
                left: Box::new(gp),
                right: Box::new(bgp(vec![])),
                expression: None,
            },
            4 => GraphPattern::Union {
                left: Box::new(gp),
                right: Box::new(bgp(vec![])),
            },
            5 => GraphPattern::Filter {
                expr: Expression::Literal(Literal::from(true)),
                inner: Box::new(gp),
            },
            6 => GraphPattern::Minus {
                left: Box::new(gp),
                right: Box::new(bgp(vec![])),
            },
            7 => GraphPattern::Extend {
                inner: Box::new(gp),
                variable: var("x"),
                expression: Expression::Literal(Literal::from(1)),
            },
            8 => GraphPattern::Group {
                inner: Box::new(gp),
                variables: vec![],
                aggregates: vec![],
            },
            9 => GraphPattern::Project {
                inner: Box::new(gp),
                variables: vec![],
            },
            10 => GraphPattern::OrderBy {
                inner: Box::new(gp),
                expression: vec![],
            },
            _ => unreachable!(),
        }
    }
    for shape in 0..11 {
        let mut gp = bgp(vec![]);
        for _ in 1..crate::compile_envelope::algebra::MAX_ALGEBRA_DEPTH_V1 {
            gp = wrap(gp, shape);
        }
        build_tree_with_work_control(&gp, None, &budget(u64::MAX)).unwrap();
        gp = wrap(gp, shape);
        assert!(matches!(
            build_tree_with_work_control(&gp, None, &budget(u64::MAX)),
            Err(Error::QueryControl(
                QueryControlError::CompilerEnvelopeExceeded
            ))
        ));
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            let control = budget(0);
            control.terminate(cause);
            assert!(
                matches!(build_tree_with_work_control(&gp, None, &control), Err(Error::QueryControl(actual)) if actual == cause)
            );
            assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
        }
    }
}

#[test]
fn unsupported_build_keeps_the_error_class_without_formatting_private_payload() {
    let gp = GraphPattern::Group {
        inner: Box::new(bgp(vec![])),
        variables: vec![],
        aggregates: vec![(
            var("n"),
            AggregateExpression::FunctionCall {
                name: AggregateFunction::Sum,
                distinct: false,
                expr: Expression::Add(
                    Box::new(Expression::Variable(var("private_payload"))),
                    Box::new(Expression::Literal(Literal::from(1))),
                ),
            },
        )],
    };
    assert!(
        matches!(build_tree(&gp, None), Err(Error::Unsupported(message)) if message.contains("private_payload"))
    );
    assert!(
        matches!(build_tree_with_work_control(&gp, None, &budget(u64::MAX)), Err(Error::Unsupported(message)) if !message.contains("private_payload"))
    );
}
