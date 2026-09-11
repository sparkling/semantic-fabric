use super::conditions::*;
use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::CmpOp;
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use crate::plan_measure::clone_root::{measure_copy_root, CompilerCloneRootV1};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_sql::Dialect;

/// Independent schedule for Construction(Filter(three VALUES rows, one EXISTS)).
/// No whole-LOWER observation: peeling, per-branch visit and visible WHERE growth.
/// The returned prefix/between/tail exclude both actual subtree clone charges.
pub(crate) fn construction_exists_condition_work() -> (u64, u64, u64) {
    let group = 2 + std::mem::size_of::<Vec<IqCond>>() as u64;
    let append = 1 + std::mem::size_of::<SqlCond>() as u64;
    // Each of the three branches now also pays its empty substitution visit.
    (group + 3, append + 3, 2 * append + 3)
}

fn mode(control: &dyn QueryControl) -> CompilerWorkMode<'_> {
    CompilerWorkMode::Metered(CompileContext::new(control))
}
fn sql() -> SqlCond {
    SqlCond::Cmp(ColRef::new(1, "café"), CmpOp::Eq, "東京".repeat(100))
}
fn pointer(sql: &SqlCond) -> *const u8 {
    let SqlCond::Cmp(_, _, text) = sql else {
        panic!()
    };
    text.as_ptr()
}

#[test]
fn condition_work_borrowed_sql_copy_and_owned_move_have_independent_exact_costs() {
    let source = sql();
    let source_pointer = pointer(&source);
    let copy = measure_copy_root(CompilerCloneRootV1::SqlCond(&source)).unwrap();
    let control = budget(1 + copy.total_work);
    let cond = IqCond::Sql(source);
    let copied = lower_iq_cond(&cond, &Branch::empty(), Dialect::Sqlite, mode(&control)).unwrap();
    assert_ne!(pointer(&copied), source_pointer);
    assert_eq!(
        control.consumed(QueryCharge::CompilerWork),
        1 + copy.total_work
    );
    let short = budget(copy.total_work);
    assert!(matches!(
        lower_iq_cond(&cond, &Branch::empty(), Dialect::Sqlite, mode(&short)),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(
        short.consumed(QueryCharge::CompilerWork),
        1 + copy.measurement_work
    );
    let control = budget(1);
    let moved =
        lower_owned_iq_cond(cond, &Branch::empty(), Dialect::Sqlite, mode(&control)).unwrap();
    assert_eq!(pointer(&moved), source_pointer);
    assert_eq!(control.consumed(QueryCharge::CompilerWork), 1);
}

#[test]
fn condition_work_boolean_vectors_and_boxes_are_prepaid_before_allocation() {
    let cond = IqCond::Not(Box::new(IqCond::And(vec![IqCond::Or(vec![IqCond::Sql(
        SqlCond::ExpressionError,
    )])])));
    let size = std::mem::size_of::<SqlCond>() as u64;
    let expected = 4 + 2 * (1 + size) + size;
    let control = budget(expected);
    lower_owned_iq_cond(
        cond.clone(),
        &Branch::empty(),
        Dialect::Sqlite,
        mode(&control),
    )
    .unwrap();
    assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
    let short = budget(expected - 1);
    assert!(matches!(
        lower_owned_iq_cond(
            cond.clone(),
            &Branch::empty(),
            Dialect::Sqlite,
            mode(&short)
        ),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(short.consumed(QueryCharge::CompilerWork), expected - size);
    // NOT visit + AND visit + its slot precede the first vector allocation.
    let vector_short = budget(3 + size - 1);
    assert!(lower_owned_iq_cond(
        cond.clone(),
        &Branch::empty(),
        Dialect::Sqlite,
        mode(&vector_short)
    )
    .is_err());
    assert_eq!(vector_short.consumed(QueryCharge::CompilerWork), 3);
    every_stop(|control| {
        lower_owned_iq_cond(
            cond.clone(),
            &Branch::empty(),
            Dialect::Sqlite,
            mode(control),
        )
    });
}

#[test]
fn condition_work_branch_append_pays_logical_growth_even_with_spare_capacity() {
    let size = std::mem::size_of::<SqlCond>() as u64;
    for existing in [0, 2] {
        for spare in [0, 100] {
            let source = sql();
            let source_pointer = pointer(&source);
            let copy = measure_copy_root(CompilerCloneRootV1::SqlCond(&source))
                .unwrap()
                .total_work;
            let mut branches: Vec<_> = (0..3)
                .map(|_| {
                    let mut branch = Branch::empty();
                    branch.where_conds = Vec::with_capacity(existing + spare);
                    branch
                        .where_conds
                        .extend(vec![SqlCond::ExpressionError; existing]);
                    branch
                })
                .collect();
            let append = 1 + (2 * existing).max(1) as u64 * size + existing as u64 * size;
            let expected = 2 + 3 * (2 + append) + 2 * copy;
            let control = budget(expected);
            apply_conds_to_branches(
                vec![IqCond::Sql(source)],
                &mut branches,
                Dialect::Sqlite,
                mode(&control),
            )
            .unwrap();
            assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
            assert_eq!(branches.len(), 3);
            for branch in &branches {
                assert_eq!(branch.where_conds.len(), existing + 1);
            }
            assert_ne!(
                pointer(branches[0].where_conds.last().unwrap()),
                source_pointer
            );
            assert_eq!(
                pointer(branches[2].where_conds.last().unwrap()),
                source_pointer
            );
        }
    }
    let run = |control: &dyn QueryControl| {
        let mut branches = vec![Branch::empty(); 3];
        apply_conds_to_branches(
            vec![IqCond::Sql(sql())],
            &mut branches,
            Dialect::Sqlite,
            mode(control),
        )?;
        Ok(branches)
    };
    every_stop(run);
    exact_and_short(run(&budget(u64::MAX)).unwrap(), run);
}

#[test]
fn condition_work_peeling_is_iterative_paid_and_preserves_group_order_and_ownership() {
    let mut node = IqNode::True;
    let mut pointers = Vec::new();
    for _ in 0..3 {
        let cond = vec![IqCond::Sql(sql())];
        pointers.push(cond.as_ptr());
        node = IqNode::Filter {
            child: Box::new(node),
            cond,
        };
    }
    let expected = 6 + 10 * std::mem::size_of::<Vec<IqCond>>() as u64;
    let control = budget(expected);
    let (body, groups) = peel_filters(node.clone(), mode(&control)).unwrap();
    assert!(matches!(body, IqNode::True));
    assert_eq!(groups.len(), 3);
    assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
    let (_, owned) = peel_filters(node.clone(), CompilerWorkMode::Uncontrolled).unwrap();
    exact_and_short((IqNode::True, owned), |control| {
        peel_filters(node.clone(), mode(control))
    });
    every_stop(|control| peel_filters(node.clone(), mode(control)));
    let (_, moved) = peel_filters(node, mode(&budget(expected))).unwrap();
    for (group, pointer) in moved.iter().zip(pointers.into_iter().rev()) {
        assert_eq!(group.as_ptr(), pointer);
    }
}
