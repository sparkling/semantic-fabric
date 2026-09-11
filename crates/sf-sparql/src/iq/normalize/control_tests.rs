use oxrdf::Literal;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::normalize_with_work_mode;
use crate::compiler_control::CompileContext;
use crate::iq::node::{BindDef, IqNode};
use crate::iq::{Scan, TermDef};
use crate::{CompilerWorkMode, Error};

fn values() -> IqNode {
    IqNode::Values {
        vars: vec!["x".into()],
        rows: vec![vec![Some(TermDef::Const(Literal::from("one").into()))]; 3],
    }
}

fn rejects_unpaid(node: IqNode) {
    let control = QueryBudget::new(QueryLimits::new(0, u64::MAX, u64::MAX, u64::MAX));
    let result = normalize_with_work_mode(
        node,
        CompilerWorkMode::Metered(CompileContext::new(&control)),
    );
    assert!(matches!(
        result,
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
}

#[test]
fn distinct_values_reject_unpaid_comparisons() {
    rejects_unpaid(IqNode::Distinct {
        child: Box::new(values()),
    });
}

#[test]
fn union_values_reject_unpaid_row_folding() {
    rejects_unpaid(IqNode::Union {
        children: vec![values(), values()],
        project: vec!["x".into()],
    });
}

#[test]
fn slice_values_reject_unpaid_row_movement() {
    rejects_unpaid(IqNode::Slice {
        child: Box::new(values()),
        offset: 1,
        limit: Some(1),
    });
}

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn run(node: IqNode, control: &dyn QueryControl) -> crate::Result<IqNode> {
    normalize_with_work_mode(
        node,
        CompilerWorkMode::Metered(CompileContext::new(control)),
    )
}

fn table(vars: &[&str], rows: &[&[Option<&str>]]) -> IqNode {
    IqNode::Values {
        vars: vars.iter().map(|v| (*v).into()).collect(),
        rows: rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.map(|s| TermDef::Const(Literal::from(s).into())))
                    .collect()
            })
            .collect(),
    }
}

fn data() -> IqNode {
    IqNode::Extensional {
        scan: Scan {
            alias: 1,
            source: sf_core::ir::LogicalSource::Table("items".into()).into(),
        },
        bind: BTreeMap::new(),
    }
}

fn union(children: Vec<IqNode>) -> IqNode {
    IqNode::Union {
        children,
        project: vec!["x".into()],
    }
}

fn fixtures() -> Vec<IqNode> {
    let mixed = union(vec![values(), data(), values(), values()]);
    let two_columns = table(&["x", "y"], &[&[Some("a"), None], &[Some("a"), Some("b")]]);
    let mut out = vec![
        IqNode::Distinct {
            child: Box::new(table(
                &["x"],
                &[&[None], &[Some("a")], &[None], &[Some("b")], &[Some("a")]],
            )),
        },
        IqNode::Distinct {
            child: Box::new(IqNode::Values {
                vars: vec!["x".into()],
                rows: vec![
                    vec![],
                    vec![None],
                    vec![],
                    vec![Some(TermDef::Concat(vec![]))],
                ],
            }),
        },
        IqNode::Distinct {
            child: Box::new(IqNode::Construction {
                child: Box::new(two_columns.clone()),
                subst: BTreeMap::new(),
                project: vec!["x".into()],
            }),
        },
        IqNode::Distinct {
            child: Box::new(mixed.clone()),
        },
        IqNode::Distinct {
            child: Box::new(data()),
        },
        union(vec![]),
        union(vec![values()]),
        union(vec![
            IqNode::Empty {
                vars: vec!["x".into()],
            },
            union(vec![values(), values()]),
        ]),
        IqNode::Union {
            children: vec![two_columns, table(&["y", "x"], &[&[None, Some("c")]])],
            project: vec!["x".into(), "y".into()],
        },
        IqNode::Union {
            children: vec![IqNode::True, IqNode::True],
            project: vec![],
        },
        union(vec![values(), data(), values()]),
        mixed.clone(),
        union(vec![
            IqNode::Construction {
                child: Box::new(IqNode::True),
                subst: BTreeMap::from([(
                    "x".into(),
                    BindDef::Expr(Box::new(spargebra::algebra::Expression::Variable(
                        spargebra::term::Variable::new("missing").unwrap(),
                    ))),
                )]),
                project: vec!["x".into()],
            },
            values(),
        ]),
    ];
    let constant = IqNode::Construction {
        child: Box::new(IqNode::True),
        subst: BTreeMap::from([(
            "x".into(),
            BindDef::Expr(Box::new(spargebra::algebra::Expression::FunctionCall(
                spargebra::algebra::Function::Concat,
                vec![
                    spargebra::algebra::Expression::Literal(Literal::from("東京")),
                    spargebra::algebra::Expression::FunctionCall(
                        spargebra::algebra::Function::Concat,
                        vec![],
                    ),
                ],
            ))),
        )]),
        project: vec!["x".into()],
    };
    out.push(union(vec![constant.clone(), constant]));
    for child in [
        values(),
        mixed,
        union(vec![values(), data()]),
        union(vec![data(), values()]),
    ] {
        for (offset, limit) in [
            (0, None),
            (0, Some(0)),
            (1, Some(1)),
            (4, Some(2)),
            (usize::MAX, Some(usize::MAX)),
        ] {
            out.push(IqNode::Slice {
                child: Box::new(child.clone()),
                offset,
                limit,
            });
        }
    }
    out
}

#[test]
fn constant_normalization_preserves_raw_shape_at_exact_and_n_minus_one_work() {
    for (index, tree) in fixtures().into_iter().enumerate() {
        let raw = format!("{:?}", super::normalize(tree.clone()).unwrap());
        let observed = budget(u64::MAX);
        assert_eq!(
            format!("{:?}", run(tree.clone(), &observed).unwrap()),
            raw,
            "fixture {index}"
        );
        let work = observed.consumed(QueryCharge::CompilerWork);
        assert!(work > 0, "fixture {index} must execute a paid row rule");
        let exact = budget(work);
        assert_eq!(format!("{:?}", run(tree.clone(), &exact).unwrap()), raw);
        assert_eq!(exact.consumed(QueryCharge::CompilerWork), work);
        let short = budget(work - 1);
        assert!(
            matches!(
                run(tree, &short),
                Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
            ),
            "fixture {index}"
        );
        assert_eq!(
            short.checkpoint(),
            Err(QueryControlError::CompilerWorkExceeded)
        );
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
fn every_row_rule_charge_observes_sticky_cancellation_and_deadline_not_decline() {
    // Includes successful folds, early-exit comparisons, dynamic decline, unknown
    // cardinality and residual offsets; every injected stop is deterministic.
    for tree in fixtures() {
        let observed = StopAtCharge {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            stop: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        run(tree.clone(), &observed).unwrap();
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
                    matches!(run(tree.clone(), &control), Err(Error::QueryControl(actual)) if actual == cause),
                    "stop {stop}: {tree:?}"
                );
                assert_eq!(control.calls.load(Ordering::SeqCst), stop);
                assert_eq!(
                    control.terminate(QueryControlError::CompilerWorkExceeded),
                    cause
                );
            }
        }
    }
}

#[test]
fn undef_duplicates_pay_hand_counted_visits_comparisons_and_suffix_movement() {
    let tree = IqNode::Distinct {
        child: Box::new(table(&["x"], &[&[None], &[None], &[None]])),
    };
    let control = budget(u64::MAX);
    let result = run(tree, &control).unwrap();
    // Two structural visits + row-rule entry + 3(row+cell) preflight + 3 unique-loop visits + 2 candidate
    // visits + 2(length+cell) comparisons + one shifted row header/slot.
    let expected = 2 + 1 + 6 + 3 + 2 + 4 + 1 + std::mem::size_of::<Vec<Option<TermDef>>>();
    assert_eq!(control.consumed(QueryCharge::CompilerWork), expected as u64);
    assert_eq!(
        format!("{result:?}"),
        format!("{:?}", table(&["x"], &[&[None]]))
    );
}

#[test]
fn term_equality_prepays_both_exact_payloads_separately_from_measurement() {
    use crate::plan_measure::clone_root::{measure_copy_root, CompilerCloneRootV1};
    let left = TermDef::Const(Literal::from("東京".repeat(1024)).into());
    let right = left.clone();
    let measure = measure_copy_root(CompilerCloneRootV1::TermDef(&left)).unwrap();
    let short = budget(2 * measure.total_work - 1);
    assert!(matches!(
        CompileContext::new(&short).constant_terms_equal(&left, &right),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(
        short.consumed(QueryCharge::CompilerWork),
        2 * measure.measurement_work
    );
    let exact = budget(2 * measure.total_work);
    assert!(CompileContext::new(&exact)
        .constant_terms_equal(&left, &right)
        .unwrap());
    assert_eq!(
        exact.consumed(QueryCharge::CompilerWork),
        2 * measure.total_work
    );
}

#[test]
fn controlled_results_keep_first_occurrence_order_and_projection_guard() {
    let tree = IqNode::Distinct {
        child: Box::new(table(
            &["x"],
            &[&[None], &[Some("b")], &[None], &[Some("a")], &[Some("b")]],
        )),
    };
    assert_eq!(
        format!("{:?}", run(tree, &budget(u64::MAX)).unwrap()),
        format!(
            "{:?}",
            table(&["x"], &[&[None], &[Some("b")], &[Some("a")]])
        )
    );
    let tree = IqNode::Distinct {
        child: Box::new(IqNode::Construction {
            child: Box::new(table(
                &["x", "y"],
                &[&[Some("a"), Some("b")], &[Some("a"), Some("c")]],
            )),
            subst: BTreeMap::new(),
            project: vec!["x".into()],
        }),
    };
    assert!(matches!(
        run(tree, &budget(u64::MAX)).unwrap(),
        IqNode::Distinct { .. }
    ));
}

#[test]
fn controlled_union_reorders_by_name_and_keeps_constant_runs_in_place() {
    let tree = IqNode::Union {
        children: vec![
            table(&["x", "y"], &[&[Some("a"), None]]),
            table(&["y", "x"], &[&[None, Some("b")]]),
        ],
        project: vec!["x".into(), "y".into()],
    };
    assert_eq!(
        format!("{:?}", run(tree, &budget(u64::MAX)).unwrap()),
        format!(
            "{:?}",
            table(&["x", "y"], &[&[Some("a"), None], &[Some("b"), None]])
        )
    );
    let tree = union(vec![
        table(&["x"], &[&[Some("a")]]),
        data(),
        table(&["x"], &[&[Some("b")]]),
        table(&["x"], &[&[Some("c")]]),
    ]);
    let IqNode::Union { children, .. } = run(tree, &budget(u64::MAX)).unwrap() else {
        panic!("must retain data arm")
    };
    assert_eq!(children.len(), 3);
    assert!(matches!(children[1], IqNode::Extensional { .. }));
    assert_eq!(
        format!("{:?}", children[2]),
        format!("{:?}", table(&["x"], &[&[Some("b")], &[Some("c")]]))
    );
}

#[test]
fn controlled_slice_preserves_remaining_offset_before_unknown_arm() {
    let tree = IqNode::Slice {
        child: Box::new(union(vec![values(), data()])),
        offset: 4,
        limit: Some(2),
    };
    let IqNode::Slice {
        child,
        offset,
        limit,
    } = run(tree, &budget(u64::MAX)).unwrap()
    else {
        panic!("residual slice is required")
    };
    assert_eq!((offset, limit), (1, Some(2)));
    assert!(matches!(*child, IqNode::Extensional { .. }));
}

#[test]
fn malformed_headers_and_ragged_rows_decline_union_folding_without_panic() {
    for (vars, project, rows) in [
        (
            vec!["x", "x"],
            vec!["x", "y"],
            vec![vec![Some("a"), Some("b")]],
        ),
        (
            vec!["x", "y"],
            vec!["x", "x"],
            vec![vec![Some("a"), Some("b")]],
        ),
        (
            vec!["x", "x", "y"],
            vec!["x", "y", "y"],
            vec![vec![None, None, None]],
        ),
        (vec!["y", "x"], vec!["x", "y"], vec![vec![Some("a")]]),
    ] {
        let rows = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let arm = table(&vars, &rows);
        let tree = IqNode::Union {
            children: vec![arm.clone(), arm],
            project: project.iter().map(|v| (*v).into()).collect(),
        };
        for mode in [
            CompilerWorkMode::Uncontrolled,
            CompilerWorkMode::Metered(CompileContext::new(&budget(u64::MAX))),
        ] {
            assert!(matches!(
                normalize_with_work_mode(tree.clone(), mode).unwrap(),
                IqNode::Union { .. }
            ));
        }
    }
}
