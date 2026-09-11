use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::{AggKind, HopRelation, PathClosure, PathKind, ScanSource};
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_sql::Dialect;

fn mode(c: &dyn QueryControl) -> CompilerWorkMode<'_> {
    CompilerWorkMode::Metered(CompileContext::new(c))
}
fn path(alias: usize) -> Branch {
    let mut b = Branch::empty();
    b.path = Some(PathClosure {
        alias,
        kind: PathKind::One,
        hop: HopExpr::Nps(vec![HopExpr::Pred(HopRelation {
            source: LogicalSource::Table("edges".into()),
            subj_col: "s".into(),
            obj_col: "o".into(),
        })]),
    });
    b
}
fn arm(alias: usize) -> Branch {
    let mut b = Branch::single(Scan {
        alias,
        source: LogicalSource::Table("items".into()).into(),
    });
    b.bindings.insert(
        "s".into(),
        TermDef::Derived {
            alias,
            term_map: TermMap::Column("id".into(), TermSpec::plain_literal()),
        },
    );
    b
}
fn pooled(
    inner: &mut Vec<Branch>,
    next: &mut usize,
    work: CompilerWorkMode<'_>,
) -> Result<Option<Branch>> {
    try_sql_group_over_union(
        inner,
        &["s".into()],
        &[AggDef {
            var: "count".into(),
            kind: AggKind::Count,
            arg: None,
            distinct: false,
            fixed_type: None,
        }],
        Dialect::Sqlite,
        next,
        work,
    )
}

#[test]
fn alias_work_path_batch_refusal_preserves_every_branch_and_counter() {
    for (start, allowance, cause) in [
        (usize::MAX - 1, 2, QueryControlError::AccountingOverflow),
        (10, 1, QueryControlError::CompilerWorkExceeded),
    ] {
        let c = budget(allowance);
        let mut branches = vec![path(1), Branch::empty(), path(3)];
        let before = format!("{branches:?}");
        let mut next = start;
        assert!(
            matches!(convert_path_branches(&mut branches, Dialect::Sqlite, &mut next, mode(&c)), Err(Error::QueryControl(e)) if e == cause)
        );
        assert_eq!(next, start);
        assert_eq!(format!("{branches:?}"), before);
        assert_eq!(c.checkpoint(), Err(cause));
    }
    every_stop(|c| {
        let mut branches = vec![path(1), path(3)];
        let before = format!("{branches:?}");
        let mut next = 10;
        let result = convert_path_branches(&mut branches, Dialect::Sqlite, &mut next, mode(c));
        if result.is_err() {
            assert_eq!(next, 10);
            assert_eq!(format!("{branches:?}"), before);
        }
        result
    });
}

#[test]
fn alias_work_path_batch_keeps_order_outer_aliases_and_bag_marker() {
    let mut branches = vec![path(1), Branch::empty(), path(3)];
    let mut raw = branches.clone();
    let start = usize::MAX - 2;
    let mut raw_next = start;
    convert_path_branches(
        &mut raw,
        Dialect::Sqlite,
        &mut raw_next,
        CompilerWorkMode::Uncontrolled,
    )
    .unwrap();
    let c = budget(2);
    let mut next = start;
    convert_path_branches(&mut branches, Dialect::Sqlite, &mut next, mode(&c)).unwrap();
    assert_eq!(format!("{branches:?}"), format!("{raw:?}"));
    assert_eq!(next, usize::MAX);
    assert_eq!(c.consumed(QueryCharge::CompilerWork), 2);
    for (index, alias, cte) in [(0, 1, start), (2, 3, start + 1)] {
        assert!(branches[index].nps);
        assert_eq!(branches[index].core[0].alias, alias);
        assert!(
            matches!(&branches[index].core[0].source, ScanSource::Path { cte_alias, .. } if *cte_alias == cte)
        );
    }
}

#[test]
fn alias_work_sql_pool_reserves_two_before_transfer_without_fallback() {
    for (start, allowance, cause) in [
        (usize::MAX - 1, 2, QueryControlError::AccountingOverflow),
        (10, 1, QueryControlError::CompilerWorkExceeded),
    ] {
        let c = budget(allowance);
        let mut inner = vec![arm(1), arm(2)];
        let before = format!("{inner:?}");
        let address = inner.as_ptr();
        let mut next = start;
        assert!(
            matches!(pooled(&mut inner, &mut next, mode(&c)), Err(Error::QueryControl(e)) if e == cause)
        );
        assert_eq!(next, start);
        assert_eq!(inner.as_ptr(), address);
        assert_eq!(format!("{inner:?}"), before);
        assert_eq!(c.checkpoint(), Err(cause));
    }
    every_stop(|c| {
        let mut inner = vec![arm(1), arm(2)];
        let before = format!("{inner:?}");
        let mut next = 10;
        let result = pooled(&mut inner, &mut next, mode(c));
        if result.is_err() {
            assert_eq!(next, 10);
            assert_eq!(format!("{inner:?}"), before);
        }
        result
    });
    let mut raw = vec![arm(1), arm(2)];
    let expected = pooled(&mut raw, &mut 10, CompilerWorkMode::Uncontrolled).unwrap();
    exact_and_short(expected, |c| {
        let mut inner = vec![arm(1), arm(2)];
        let mut next = 10;
        let out = pooled(&mut inner, &mut next, mode(c))?;
        assert_eq!(next, 12);
        assert!(inner.is_empty());
        Ok(out)
    });
}

#[test]
fn alias_work_ineligible_sql_pool_does_not_reserve_or_consume_arms() {
    let mut inner = vec![arm(1)];
    let before = format!("{inner:?}");
    let mut next = usize::MAX;
    let c = budget(0);
    assert!(pooled(&mut inner, &mut next, mode(&c)).unwrap().is_none());
    assert_eq!(next, usize::MAX);
    assert_eq!(format!("{inner:?}"), before);
    assert_eq!(c.consumed(QueryCharge::CompilerWork), 0);
}

#[test]
fn alias_work_branch_successor_checks_condition_and_binding_aliases() {
    let mut branch = arm(2);
    branch.where_conds.push(SqlCond::Exists {
        scans: vec![Scan {
            alias: 14,
            source: LogicalSource::Table("hidden".into()).into(),
        }],
        conds: vec![SqlCond::IsNull(ColRef::new(14, "x"))],
    });
    let c = budget(1);
    assert_eq!(branch_next_alias(&branch, mode(&c)).unwrap(), 15);
    assert_eq!(c.consumed(QueryCharge::CompilerWork), 1);
    branch
        .where_conds
        .push(SqlCond::IsNull(ColRef::new(usize::MAX, "x")));
    let c = budget(1);
    assert!(matches!(
        branch_next_alias(&branch, mode(&c)),
        Err(Error::QueryControl(QueryControlError::AccountingOverflow))
    ));
    assert_eq!(c.checkpoint(), Err(QueryControlError::AccountingOverflow));
    assert_eq!(
        branch_next_alias(&Branch::empty(), CompilerWorkMode::Uncontrolled).unwrap(),
        0
    );
}

#[test]
fn alias_work_subplan_overflow_and_slice_guard_leave_outer_counter_unchanged() {
    for sliced in [false, true] {
        let data = IqNode::Extensional {
            scan: arm(1).core.remove(0),
            bind: BTreeMap::new(),
        };
        let node = if sliced {
            IqNode::Slice {
                child: Box::new(data),
                offset: 0,
                limit: Some(1),
            }
        } else {
            IqNode::Distinct {
                child: Box::new(data),
            }
        };
        let mut next = usize::MAX;
        let c = budget(u64::MAX);
        let result = lower_as_subplan(
            node,
            Dialect::Sqlite,
            &mut next,
            &Default::default(),
            &Default::default(),
            mode(&c),
        );
        assert_eq!(next, usize::MAX);
        if sliced {
            assert!(matches!(result, Err(Error::Unsupported(_))));
        } else {
            assert!(matches!(
                result,
                Err(Error::QueryControl(QueryControlError::AccountingOverflow))
            ));
        }
    }
}
