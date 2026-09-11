use super::*;
use crate::leftjoin::optional_work_test_support::budget;
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use spargebra::algebra::Expression;

#[test]
fn optional_filter_measurement_precedes_inclusive_product_reservation() {
    let expression = Expression::Bound(spargebra::term::Variable::new("name").unwrap());
    let bindings = BTreeMap::from([(
        "name".into(),
        crate::iq::TermDef::Derived {
            alias: 0,
            term_map: sf_core::ir::TermMap::Column("東京".into(), sf_core::ir::TermSpec::iri()),
        },
    )]);
    let measured = budget(u64::MAX);
    let cx = CompileContext::new(&measured);
    let a = cx
        .measure_root(CompilerCloneRootV1::Expression(&expression))
        .unwrap();
    let b = cx
        .measure_collection(CompilerCloneCollectionV1::Bindings(&bindings))
        .unwrap();
    let prefix = measured.consumed(QueryCharge::CompilerWork);
    let expected = prefix + 1024 + 128 * (a.deep_clone_work + 1) * (b.deep_clone_work + 1);
    let exact = budget(expected);
    let raw = crate::unify::filter_cond(&expression, &bindings, sf_sql::Dialect::Sqlite).unwrap();
    let got = CompileContext::new(&exact)
        .filter_condition(&expression, &bindings, sf_sql::Dialect::Sqlite)
        .unwrap();
    assert_eq!(format!("{got:?}"), format!("{raw:?}"));
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), expected);
    let short = budget(expected - 1);
    assert!(matches!(
        CompileContext::new(&short).filter_condition(
            &expression,
            &bindings,
            sf_sql::Dialect::Sqlite
        ),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(short.consumed(QueryCharge::CompilerWork), prefix);
}

#[test]
fn optional_filter_product_overflow_is_sticky_without_a_reservation() {
    for (a, b) in [
        (u64::MAX, 0),
        (0, u64::MAX),
        (u64::MAX / 2, 3),
        (0, u64::MAX / 128),
    ] {
        let control = budget(u64::MAX);
        assert!(matches!(
            CompileContext::new(&control).reserve_filter_work(a, b),
            Err(Error::QueryControl(QueryControlError::AccountingOverflow))
        ));
        assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
        assert_eq!(
            control.checkpoint(),
            Err(QueryControlError::AccountingOverflow)
        );
    }
}
