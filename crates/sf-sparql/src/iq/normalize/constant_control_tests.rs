use std::collections::BTreeMap;

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use spargebra::algebra::{Expression, Function};
use spargebra::term::{Literal, NamedNode, Variable};

use super::control::RowWork;
use super::values::{constant_row_from_subst, constant_subst_can_form_row, same_var_set};
use crate::compiler_control::CompileContext;
use crate::iq::node::{BindDef, Var};
use crate::iq::TermDef;
use crate::{CompilerWorkMode, Error};

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn mode(control: &dyn QueryControl) -> CompilerWorkMode<'_> {
    CompilerWorkMode::Metered(CompileContext::new(control))
}

fn expressions() -> Vec<Expression> {
    vec![
        Expression::NamedNode(NamedNode::new("http://example.test/東京").unwrap()),
        Expression::Literal(Literal::from("café 東京")),
        Expression::Literal(Literal::new_language_tagged_literal("hello", "en").unwrap()),
        Expression::Literal(Literal::new_typed_literal(
            "01",
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        )),
        Expression::FunctionCall(Function::Concat, vec![]),
        Expression::FunctionCall(
            Function::Concat,
            vec![
                Expression::Literal(Literal::from("a")),
                Expression::FunctionCall(
                    Function::Concat,
                    vec![Expression::Literal(Literal::from("b"))],
                ),
            ],
        ),
        Expression::Variable(Variable::new("missing").unwrap()),
        Expression::FunctionCall(Function::Str, vec![Expression::Literal(Literal::from("a"))]),
        Expression::FunctionCall(
            Function::Concat,
            vec![
                Expression::Literal(Literal::from("a")),
                Expression::Variable(Variable::new("missing").unwrap()),
            ],
        ),
    ]
}

#[test]
fn constant_probe_and_materialization_match_the_empty_binding_unifier() {
    for expr in expressions() {
        let expected = crate::unify::bind_term_def(&expr, &BTreeMap::new()).ok();
        let subst = BTreeMap::from([("x".into(), BindDef::Expr(Box::new(expr)))]);
        let project = vec![Var::from("x")];
        let control = budget(u64::MAX);
        let work = RowWork::new(mode(&control));
        assert_eq!(
            constant_subst_can_form_row(&subst, &project, work).unwrap(),
            expected.is_some()
        );
        let row = constant_row_from_subst(subst.clone(), &project, work).unwrap();
        assert_eq!(
            format!("{row:?}"),
            format!("{:?}", expected.map(|term| vec![Some(term)]))
        );
        let total = control.consumed(QueryCharge::CompilerWork);
        let exact = budget(total);
        let work = RowWork::new(mode(&exact));
        constant_subst_can_form_row(&subst, &project, work).unwrap();
        constant_row_from_subst(subst.clone(), &project, work).unwrap();
        assert_eq!(exact.consumed(QueryCharge::CompilerWork), total);
        let short = budget(total - 1);
        let work = RowWork::new(mode(&short));
        let result = constant_subst_can_form_row(&subst, &project, work)
            .and_then(|_| constant_row_from_subst(subst, &project, work));
        assert!(matches!(
            result,
            Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
        ));
    }
}

#[test]
fn sorted_lookup_preserves_missing_repeat_and_unprojected_binding_semantics() {
    let subst = BTreeMap::from([
        (
            Var::from("東京"),
            BindDef::Resolved(TermDef::Const(Literal::from("tokyo").into())),
        ),
        (
            Var::from("京都"),
            BindDef::Resolved(TermDef::Const(Literal::from("kyoto").into())),
        ),
        (
            Var::from("not-projected"),
            BindDef::Expr(Box::new(Expression::Variable(
                Variable::new("missing").unwrap(),
            ))),
        ),
    ]);
    let project: Vec<Var> = ["missing", "東京", "京都", "東京"].map(Into::into).into();
    let control = budget(u64::MAX);
    let work = RowWork::new(mode(&control));
    assert!(constant_subst_can_form_row(&subst, &project, work).unwrap());
    let row = constant_row_from_subst(subst, &project, work)
        .unwrap()
        .unwrap();
    assert_eq!(
        format!("{row:?}"),
        format!(
            "{:?}",
            vec![
                None,
                Some(TermDef::Const(Literal::from("tokyo").into())),
                Some(TermDef::Const(Literal::from("kyoto").into())),
                None
            ]
        )
    );
    let zero = budget(0);
    assert!(matches!(
        same_var_set(&[], &[], RowWork::new(mode(&zero))),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
}

#[test]
fn constant_expression_depth_is_bounded_on_the_default_test_stack() {
    let limit = crate::compile_envelope::algebra::MAX_ALGEBRA_DEPTH_V1;
    let mut expr = Expression::Literal(Literal::from("leaf"));
    for _ in 1..limit {
        expr = Expression::FunctionCall(Function::Concat, vec![expr]);
    }
    for exceeds in [false, true] {
        if exceeds {
            expr = Expression::FunctionCall(Function::Concat, vec![expr]);
        }
        let control = budget(u64::MAX);
        let subst = BTreeMap::from([("x".into(), BindDef::Expr(Box::new(expr.clone())))]);
        let project = vec!["x".into()];
        assert!(
            constant_subst_can_form_row(&subst, &project, RowWork::new(mode(&control))).unwrap()
        );
        let result = constant_row_from_subst(subst, &project, RowWork::new(mode(&control)));
        if exceeds {
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
        } else {
            assert!(result.unwrap().is_some());
        }
    }
}

#[test]
fn terminal_control_cannot_be_swallowed_by_an_unsupported_constant_probe() {
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        let control = budget(u64::MAX);
        control.terminate(cause);
        let subst = BTreeMap::from([(
            "x".into(),
            BindDef::Expr(Box::new(Expression::Variable(
                Variable::new("missing").unwrap(),
            ))),
        )]);
        let project = vec!["x".into()];
        assert!(
            matches!(constant_subst_can_form_row(&subst, &project, RowWork::new(mode(&control))), Err(Error::QueryControl(actual)) if actual == cause)
        );
        assert!(
            matches!(constant_row_from_subst(subst, &project, RowWork::new(mode(&control))), Err(Error::QueryControl(actual)) if actual == cause)
        );
    }
}
