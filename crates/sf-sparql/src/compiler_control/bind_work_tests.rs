use super::*;
use crate::iq::TermDef;
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use crate::plan_measure::clone_root::measure_copy_root;
use spargebra::algebra::{Expression as E, Function};
use spargebra::term::Variable;

fn var(name: &str) -> E {
    E::Variable(Variable::new(name).unwrap())
}
fn literal(text: &str) -> E {
    E::Literal(sf_core::Literal::from(text))
}
fn run(
    expr: &E,
    bindings: &BTreeMap<String, TermDef>,
    control: &dyn QueryControl,
) -> Result<String> {
    Ok(format!(
        "{:?}",
        CompileContext::new(control).bind_definition(expr, bindings)?
    ))
}

#[test]
fn bind_work_supported_and_error_decisions_match_raw_at_every_stop() {
    let bindings = BTreeMap::from([(
        "x".into(),
        TermDef::Concat(vec![TermDef::Const(
            sf_core::Literal::from("東京\n").into(),
        )]),
    )]);
    for expr in [
        literal("café"),
        E::NamedNode(sf_core::NamedNode::new("http://example.test/東京").unwrap()),
        var("x"),
        var("missing"),
        E::FunctionCall(Function::Concat, vec![]),
        E::FunctionCall(Function::Concat, vec![var("x"), literal("!"), var("x")]),
        E::FunctionCall(
            Function::Concat,
            vec![var("missing"), E::Not(Box::new(var("later")))],
        ),
        E::Not(Box::new(literal("\n\r\t東京"))),
        E::If(
            Box::new(var("x")),
            Box::new(var("missing")),
            Box::new(literal("!")),
        ),
    ] {
        let raw = format!("{:?}", crate::unify::bind_term_def(&expr, &bindings));
        exact_and_short(raw, |control| run(&expr, &bindings, control));
        every_stop(|control| run(&expr, &bindings, control));
    }
}

#[test]
fn bind_work_variable_pays_exact_ordered_lookup_and_only_matched_copy() {
    for unused in ["short".to_owned(), "ignored".repeat(1000)] {
        let source = TermDef::Const(sf_core::Literal::from("東京").into());
        let measured = measure_copy_root(CompilerCloneRootV1::TermDef(&source)).unwrap();
        let bindings = BTreeMap::from([
            (
                "a".into(),
                TermDef::Const(sf_core::Literal::from(unused).into()),
            ),
            ("x".into(), source),
            ("z".into(), TermDef::Concat(vec![])),
        ]);
        // node1; keys a and x each pay visit1 + min bytes1; z isn't visited.
        let expected = 5 + measured.measurement_work + measured.deep_clone_work;
        let control = budget(expected);
        run(&var("x"), &bindings, &control).unwrap();
        assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
        let short = budget(expected - 1);
        assert!(matches!(
            run(&var("x"), &bindings, &short),
            Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
        ));
        assert_eq!(
            short.consumed(QueryCharge::CompilerWork),
            5 + measured.measurement_work
        );
    }
}

#[test]
fn bind_work_concat_slots_and_repeated_occurrences_are_exact() {
    let expr = E::FunctionCall(Function::Concat, vec![literal("a"), literal("b")]);
    let leaf =
        measure_copy_root(CompilerCloneRootV1::Literal(&sf_core::Literal::from("a"))).unwrap();
    let expected = 1
        + 2 * (1 + std::mem::size_of::<TermDef>() as u64)
        + 2 * (1 + leaf.measurement_work + leaf.deep_clone_work);
    let control = budget(expected);
    run(&expr, &BTreeMap::new(), &control).unwrap();
    assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
}

#[test]
fn bind_work_unsupported_error_is_prepaid_and_overflow_sticky() {
    let expr = E::Not(Box::new(var("unbound")));
    let measured = measure_copy_root(CompilerCloneRootV1::Expression(&expr)).unwrap();
    let prefix = 1 + measured.measurement_work;
    let total = prefix + 256 + 16 * measured.deep_clone_work;
    let exact = budget(total);
    run(&expr, &BTreeMap::new(), &exact).unwrap();
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), total);
    let short = budget(total - 1);
    assert!(run(&expr, &BTreeMap::new(), &short).is_err());
    assert_eq!(short.consumed(QueryCharge::CompilerWork), prefix);
    for units in [u64::MAX, u64::MAX / 16, u64::MAX / 16 + 1] {
        let control = budget(u64::MAX);
        assert!(matches!(
            CompileContext::new(&control).reserve_bind_error(units),
            Err(Error::QueryControl(QueryControlError::AccountingOverflow))
        ));
        assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
        assert_eq!(
            control.checkpoint(),
            Err(QueryControlError::AccountingOverflow)
        );
    }
}

#[test]
fn bind_work_nested_concat_shares_depth_and_stops_before_later_error() {
    let limit = crate::compile_envelope::algebra::MAX_ALGEBRA_DEPTH_V1;
    for depth in [limit, limit + 1] {
        let mut expr = literal("end");
        for _ in 1..depth {
            expr = E::FunctionCall(Function::Concat, vec![expr]);
        }
        let control = budget(u64::MAX);
        let result = CompileContext::new(&control).bind_definition(&expr, &BTreeMap::new());
        if depth == limit {
            assert!(result.unwrap().is_ok());
        } else {
            assert!(matches!(
                result,
                Err(Error::QueryControl(
                    QueryControlError::CompilerEnvelopeExceeded
                ))
            ));
        }
    }
    let mut deep = literal("unreached");
    for _ in 0..limit {
        deep = E::FunctionCall(Function::Concat, vec![deep]);
    }
    let expr = E::FunctionCall(Function::Concat, vec![var("first"), deep]);
    let result = CompileContext::new(&budget(u64::MAX))
        .bind_definition(&expr, &BTreeMap::new())
        .unwrap();
    assert_eq!(result.unwrap_err(), "BIND references unbound ?first");
}
