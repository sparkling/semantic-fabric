use super::optional_work_test_support::{budget, every_stop, exact_and_short};
use super::*;
use crate::compiler_control::CompileContext;
use crate::plan_measure::clone_root::{measure_copy_root, CompilerCloneRootV1};
use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_sql::Dialect;
use spargebra::algebra::Expression;
use spargebra::term::Variable;

fn branch(alias: usize) -> Branch {
    Branch::single(crate::iq::Scan {
        alias,
        source: LogicalSource::Table(format!("table_{alias}")).into(),
    })
}
fn derived(alias: usize, col: &str) -> TermDef {
    TermDef::Derived {
        alias,
        term_map: TermMap::Column(col.into(), TermSpec::iri()),
    }
}
fn mode(control: &dyn QueryControl) -> CompilerWorkMode<'_> {
    CompilerWorkMode::Metered(CompileContext::new(control))
}
fn copy_work(branch: &Branch) -> u64 {
    measure_copy_root(CompilerCloneRootV1::Branch(branch))
        .unwrap()
        .total_work
}
fn nullable_pair() -> (Branch, Branch) {
    let mut left = branch(0);
    left.opts.push(OptJoin {
        scan: branch(1).core.remove(0),
        on: vec![],
        extra: vec![],
    });
    left.bindings.insert("shared".into(), derived(1, "left"));
    let mut right = branch(2);
    right.bindings.insert("shared".into(), derived(2, "right"));
    right.bindings.insert("extra".into(), derived(2, "extra"));
    right
        .where_conds
        .push(SqlCond::IsNotNull(ColRef::new(2, "extra")));
    (left, right)
}

#[test]
fn optional_work_candidate_schedule_and_overflow_are_prospective() {
    let exact = budget(15);
    work::candidates(mode(&exact), 3, 2, true).unwrap();
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), 15);
    let short = budget(14);
    assert!(matches!(
        work::candidates(mode(&short), 3, 2, true),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 12);
    let fast = budget(3);
    work::candidates(mode(&fast), 3, 1, false).unwrap();
    assert_eq!(fast.consumed(QueryCharge::CompilerWork), 3);
    let overflow = budget(u64::MAX);
    assert!(matches!(
        work::candidates(mode(&overflow), usize::MAX, 2, true),
        Err(Error::QueryControl(QueryControlError::AccountingOverflow))
    ));
    assert_eq!(overflow.consumed(QueryCharge::CompilerWork), 0);
    assert_eq!(
        overflow.checkpoint(),
        Err(QueryControlError::AccountingOverflow)
    );
    work::candidates(mode(&budget(0)), 0, usize::MAX, true).unwrap();
}

#[test]
fn optional_work_all_candidates_are_paid_even_when_unification_prunes() {
    let mut left = Branch::empty();
    left.bindings.insert(
        "x".into(),
        TermDef::Const(sf_core::Literal::from("left").into()),
    );
    let mut right = Branch::empty();
    right.bindings.insert(
        "x".into(),
        TermDef::Const(sf_core::Literal::from("right").into()),
    );
    let control = budget(11); // Three left x two right already needs twelve pair visits.
    assert!(matches!(
        left_join_branches_with_work_mode(
            vec![left; 3],
            vec![right; 2],
            None,
            Dialect::Sqlite,
            mode(&control)
        ),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
}

#[test]
fn optional_work_direct_match_copies_exact_left_and_right_fields() {
    let mut left = branch(1);
    left.bindings.insert(
        "left".into(),
        TermDef::Const(sf_core::Literal::from("left payload".repeat(256)).into()),
    );
    left.where_conds
        .push(SqlCond::IsNull(ColRef::new(1, "left_column")));
    left.distinct = true;
    left.limit = Some(3);
    left.offset = 2;
    let mut right = branch(2);
    right.bindings.insert(
        "right".into(),
        TermDef::Const(sf_core::Literal::from("right payload".repeat(256)).into()),
    );
    right
        .where_conds
        .push(SqlCond::IsNull(ColRef::new(2, "right_column")));
    let expected = copy_work(&left) + copy_work(&right);
    let exact = budget(expected);
    let got =
        inner_join_one_with_work_mode(&left, &right, None, Dialect::Sqlite, mode(&exact)).unwrap();
    let raw = inner_join_one(&left, &right, None, Dialect::Sqlite).unwrap();
    assert_eq!(format!("{got:?}"), format!("{raw:?}"));
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), expected);
    let short = budget(expected - 1);
    assert!(matches!(
        inner_join_one_with_work_mode(&left, &right, None, Dialect::Sqlite, mode(&short)),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(
        short.consumed(QueryCharge::CompilerWork),
        copy_work(&left)
            + measure_copy_root(CompilerCloneRootV1::Branch(&right))
                .unwrap()
                .measurement_work
    );
}

#[test]
fn optional_work_anti_match_copies_left_only_when_filter_needs_it() {
    let mut left = branch(1);
    left.bindings
        .insert("left".into(), derived(1, &"payload".repeat(500)));
    let right = branch(2);
    for expr in [
        None,
        Some(Expression::Bound(Variable::new("left").unwrap())),
    ] {
        let expected = copy_work(&right) + if expr.is_some() { copy_work(&left) } else { 0 };
        let exact = budget(expected);
        let got = not_exists_cond_for_with_work_mode(
            &left,
            &right,
            expr.as_ref(),
            Dialect::Sqlite,
            mode(&exact),
        )
        .unwrap();
        let raw = not_exists_cond_for(&left, &right, expr.as_ref(), Dialect::Sqlite).unwrap();
        assert_eq!(format!("{got:?}"), format!("{raw:?}"));
        assert_eq!(exact.consumed(QueryCharge::CompilerWork), expected);
    }
}

#[test]
fn optional_work_fast_decomposed_nullable_and_filtered_paths_match_raw_at_exact_budget() {
    let (left, right) = nullable_pair();
    for count in [1, 2] {
        for expr in [
            None,
            Some(Expression::Bound(Variable::new("shared").unwrap())),
        ] {
            let left = vec![left.clone(); 2];
            let right = vec![right.clone(); count];
            let raw =
                left_join_branches(left.clone(), right.clone(), expr.as_ref(), Dialect::Sqlite)
                    .unwrap();
            exact_and_short(raw, |control| {
                left_join_branches_with_work_mode(
                    left.clone(),
                    right.clone(),
                    expr.as_ref(),
                    Dialect::Sqlite,
                    mode(control),
                )
            });
        }
    }
}

#[test]
fn optional_work_each_charge_observes_cancel_and_deadline_before_continuing() {
    let (left, right) = nullable_pair();
    for count in [1, 2] {
        every_stop(|control| {
            left_join_branches_with_work_mode(
                vec![left.clone(); 2],
                vec![right.clone(); count],
                None,
                Dialect::Sqlite,
                mode(control),
            )
        });
    }
}

#[test]
fn optional_work_owned_no_match_tail_retains_original_scan_allocation() {
    let left = branch(0);
    let original = left.core.as_ptr();
    let out = left_join_branches_with_work_mode(
        vec![left],
        vec![branch(1), branch(2)],
        None,
        Dialect::Sqlite,
        mode(&budget(u64::MAX)),
    )
    .unwrap();
    assert_eq!(out.len(), 3);
    assert_eq!(out[2].core.as_ptr(), original);
    assert_eq!(out[2].where_conds.len(), 2);
}

#[test]
fn optional_work_output_growth_does_not_borrow_unpaid_allocator_capacity() {
    // A logical prefix of one has the same charge
    // whether its allocator capacity is one or has large spare space.
    let mut observed = Vec::new();
    for capacity in [1, 1024] {
        let control = budget(u64::MAX);
        let mut values = Vec::with_capacity(capacity);
        values.push(7_u64);
        work::push_owned(BuildWork::new(mode(&control)), &mut values, 9).unwrap();
        assert_eq!(values, [7, 9]);
        observed.push(control.consumed(QueryCharge::CompilerWork));
    }
    assert_eq!(observed, [1 + 2 * 8 + 8; 2]);
}

#[test]
fn optional_work_empty_right_is_owned_identity_but_observes_terminal_control() {
    let exact = budget(0);
    let left = branch(1);
    let original = left.core.as_ptr();
    let out =
        left_join_branches_with_work_mode(vec![left], vec![], None, Dialect::Sqlite, mode(&exact))
            .unwrap();
    assert_eq!(out[0].core.as_ptr(), original);
    exact.terminate(QueryControlError::Cancelled);
    assert!(matches!(
        left_join_branches_with_work_mode(vec![], vec![], None, Dialect::Sqlite, mode(&exact)),
        Err(Error::QueryControl(QueryControlError::Cancelled))
    ));
}
