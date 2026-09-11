use super::conditions::*;
use super::*;
use crate::compiler_control::CompileContext;
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_sql::Dialect;
use spargebra::algebra::Expression as E;
use spargebra::term::Variable;

fn branch() -> Branch {
    let mut branch = Branch::single(Scan {
        alias: 1,
        source: LogicalSource::Table("items".into()).into(),
    });
    branch.bindings.insert(
        "iri".into(),
        TermDef::Derived {
            alias: 1,
            term_map: TermMap::Column("id".into(), TermSpec::iri()),
        },
    );
    branch
}
fn decision<T: std::fmt::Debug>(result: Result<T>) -> Result<String> {
    match result {
        Err(Error::QueryControl(cause)) => Err(Error::QueryControl(cause)),
        other => Ok(format!("{other:?}")),
    }
}
fn mode(control: &dyn QueryControl) -> CompilerWorkMode<'_> {
    CompilerWorkMode::Metered(CompileContext::new(control))
}

#[test]
fn condition_work_expression_and_boolean_decisions_match_raw_across_dialects() {
    let outer = branch();
    let iri = E::Variable(Variable::new("iri").unwrap());
    let equal = E::Equal(
        Box::new(iri.clone()),
        Box::new(E::NamedNode(
            sf_core::NamedNode::new("http://example.test/東京").unwrap(),
        )),
    );
    let invalid = E::Add(Box::new(iri.clone()), Box::new(iri));
    for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
        for expr in [
            equal.clone(),
            invalid.clone(),
            E::Bound(Variable::new("missing").unwrap()),
        ] {
            let expected =
                decision(filter_cond(&expr, &outer, dialect).map_err(Error::Unsupported)).unwrap();
            let cond = IqCond::Expr(Box::new(expr));
            for owned in [false, true] {
                let run = |control: &dyn QueryControl| {
                    decision(if owned {
                        lower_owned_iq_cond(cond.clone(), &outer, dialect, mode(control))
                    } else {
                        lower_iq_cond(&cond, &outer, dialect, mode(control))
                    })
                };
                exact_and_short(expected.clone(), run);
                every_stop(run);
            }
        }
        let not = IqCond::Not(Box::new(IqCond::Sql(SqlCond::ExpressionError)));
        let cond = IqCond::And(vec![
            IqCond::Or(vec![not]),
            IqCond::Expr(Box::new(equal.clone())),
        ]);
        let raw = lower_iq_cond(&cond, &outer, dialect, CompilerWorkMode::Uncontrolled).unwrap();
        let expected = format!("{raw:?}");
        for owned in [false, true] {
            let run = |control: &dyn QueryControl| {
                if owned {
                    lower_owned_iq_cond(cond.clone(), &outer, dialect, mode(control))
                } else {
                    lower_iq_cond(&cond, &outer, dialect, mode(control))
                }
            };
            exact_and_short(raw.clone(), run);
            assert_eq!(format!("{:?}", run(&budget(u64::MAX)).unwrap()), expected);
            every_stop(run);
        }
    }
}

#[test]
fn condition_work_exists_not_exists_and_disjoint_minus_preserve_delegation() {
    let outer = branch();
    for inner in [IqNode::True, IqNode::Empty { vars: vec![] }] {
        for (negated, is_minus) in [(false, false), (true, false), (true, true)] {
            let cond = if negated {
                IqCond::NotExists {
                    inner: Box::new(inner.clone()),
                    is_minus,
                }
            } else {
                IqCond::Exists(Box::new(inner.clone()))
            };
            let expected = lower_owned_iq_exists(
                inner.clone(),
                &outer,
                negated,
                is_minus,
                Dialect::Sqlite,
                CompilerWorkMode::Uncontrolled,
            )
            .unwrap();
            for owned in [false, true] {
                let run = |control: &dyn QueryControl| {
                    if owned {
                        lower_owned_iq_cond(cond.clone(), &outer, Dialect::Sqlite, mode(control))
                    } else {
                        lower_iq_cond(&cond, &outer, Dialect::Sqlite, mode(control))
                    }
                };
                exact_and_short(expected.clone(), run);
                every_stop(run);
            }
        }
    }
}

#[test]
fn condition_work_depth_is_shared_through_mixed_boolean_recursion() {
    let make = |depth| {
        let mut cond = IqCond::Sql(SqlCond::ExpressionError);
        for n in 1..depth {
            cond = match n % 3 {
                0 => IqCond::Not(Box::new(cond)),
                1 => IqCond::And(vec![cond]),
                _ => IqCond::Or(vec![cond]),
            };
        }
        cond
    };
    let limit = crate::compile_envelope::algebra::MAX_ALGEBRA_DEPTH_V1;
    for owned in [false, true] {
        for depth in [limit, limit + 1] {
            let cond = make(depth);
            let control = budget(u64::MAX);
            let result = if owned {
                lower_owned_iq_cond(cond, &branch(), Dialect::Sqlite, mode(&control))
            } else {
                lower_iq_cond(&cond, &branch(), Dialect::Sqlite, mode(&control))
            };
            if depth == limit {
                assert!(result.is_ok());
            } else {
                assert!(matches!(
                    result,
                    Err(Error::QueryControl(
                        QueryControlError::CompilerEnvelopeExceeded
                    ))
                ));
                assert_eq!(
                    control.checkpoint(),
                    Err(QueryControlError::CompilerEnvelopeExceeded)
                );
            }
        }
    }
}

#[test]
fn condition_work_empty_paths_do_not_evaluate_and_first_error_stays_first() {
    let invalid = IqCond::Expr(Box::new(E::Add(
        Box::new(E::Literal(sf_core::Literal::from(true))),
        Box::new(E::Literal(sf_core::Literal::from(false))),
    )));
    let control = budget(0);
    apply_conds_to_branches(
        vec![invalid.clone()],
        &mut vec![],
        Dialect::Sqlite,
        mode(&control),
    )
    .unwrap();
    apply_conds_to_branches(
        vec![],
        &mut vec![branch(); 10],
        Dialect::Sqlite,
        mode(&control),
    )
    .unwrap();
    assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
    let mut actual = branch();
    let conds = vec![
        IqCond::Sql(SqlCond::ExpressionError),
        invalid.clone(),
        IqCond::Exists(Box::new(IqNode::Empty { vars: vec![] })),
    ];
    let expected = lower_iq_cond(
        &invalid,
        &actual,
        Dialect::Sqlite,
        CompilerWorkMode::Uncontrolled,
    )
    .unwrap_err();
    let error = apply_owned_conds(conds, &mut actual, Dialect::Sqlite, mode(&budget(u64::MAX)))
        .unwrap_err();
    assert_eq!(error.to_string(), expected.to_string());
    assert_eq!(actual.where_conds.len(), 1);
    control.terminate(QueryControlError::Cancelled);
    assert!(matches!(
        apply_conds_to_branches(vec![], &mut vec![], Dialect::Sqlite, mode(&control)),
        Err(Error::QueryControl(QueryControlError::Cancelled))
    ));
}

#[test]
fn condition_work_empty_peeled_groups_still_pay_each_branch_visit() {
    let mut branch = Branch::empty();
    let control = budget(2);
    apply_conds(&[], &mut branch, Dialect::Sqlite, mode(&control)).unwrap();
    apply_owned_conds(vec![], &mut branch, Dialect::Sqlite, mode(&control)).unwrap();
    assert_eq!(control.consumed(QueryCharge::CompilerWork), 2);
    assert!(branch.where_conds.is_empty());
    assert!(matches!(
        apply_conds(&[], &mut branch, Dialect::Sqlite, mode(&control)),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
}
