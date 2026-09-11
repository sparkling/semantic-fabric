use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::{CmpOp, SqlCond, SubPlanJoin, TermDef};
use crate::{CompilerWorkMode, Error, Plan, PlanForm};
use sf_core::ir::{TermMap, TermSpec};
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use sf_sql::Dialect;
use std::sync::atomic::{AtomicUsize, Ordering};

fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX))
}

fn leaf(contradiction: bool) -> Branch {
    let mut branch = Branch::empty();
    branch.bindings.insert(
        "x".into(),
        TermDef::Derived {
            term_map: TermMap::Column("value".into(), TermSpec::plain_literal()),
            alias: 1,
        },
    );
    branch.where_conds.push(SqlCond::Cmp(
        ColRef::new(1, "filter"),
        CmpOp::Eq,
        "one".into(),
    ));
    if contradiction {
        branch.where_conds.push(SqlCond::Cmp(
            ColRef::new(1, "filter"),
            CmpOp::Eq,
            "two".into(),
        ));
    }
    branch
}

fn wrapper(branches: Vec<Branch>) -> Branch {
    let mut branch = Branch::empty();
    branch.subplan_joins.push(SubPlanJoin {
        alias: 2,
        on: Vec::new(),
        left: false,
        plan: Box::new(Plan {
            branches,
            form: PlanForm::Select {
                vars: vec!["x".into()],
            },
            distinct: false,
            limit: None,
            offset: 0,
            order: Vec::new(),
            rust_group: None,
            dialect: Dialect::Sqlite,
            dedup_scopes: Vec::new(),
            construct_drops_some_branch_var: false,
        }),
    });
    branch
}

#[test]
fn paid_projection_preserves_first_occurrence_order_and_distinct_rules() {
    for distinct in [false, true] {
        let mut branch = leaf(false);
        branch.distinct = distinct;
        let observed = budget(u64::MAX);
        let work = BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&observed)));
        assert_eq!(projection(&branch, work).unwrap(), branch.projection());
        let units = observed.consumed(QueryCharge::CompilerWork);
        for (limit, success) in [(units, true), (units - 1, false)] {
            let control = budget(limit);
            assert_eq!(
                projection(
                    &branch,
                    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&control)))
                )
                .is_ok(),
                success
            );
        }
    }
}

#[test]
fn nested_candidate_keeps_multiarm_rollback_and_singlearm_pruning() {
    for arms in [
        vec![leaf(true)],
        vec![leaf(true), leaf(false)],
        vec![leaf(false), leaf(false)],
    ] {
        let input = wrapper(arms.clone());
        let candidate = cascade::run(arms.clone(), &[], &cascade::CascadeCtx::default());
        let widths: Vec<_> = candidate.iter().map(|b| b.projection().len()).collect();
        let safe = arms.len() == 1
            || (arms.len() == candidate.len() && widths.windows(2).all(|p| p[0] == p[1]));
        let expected = wrapper(if safe { candidate } else { arms });
        let mut actual = input.clone();
        let observed = budget(u64::MAX);
        subplans(
            &mut actual,
            &[],
            BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&observed))),
        )
        .unwrap();
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
        let units = observed.consumed(QueryCharge::CompilerWork);
        let short = budget(units - 1);
        assert!(matches!(
            subplans(
                &mut input.clone(),
                &[],
                BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&short)))
            ),
            Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
        ));
        let exact = budget(units);
        let mut actual = input;
        subplans(
            &mut actual,
            &[],
            BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&exact))),
        )
        .unwrap();
        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
    }
}

struct Stop {
    budget: QueryBudget,
    calls: AtomicUsize,
    at: usize,
    cause: QueryControlError,
}
impl QueryControl for Stop {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.budget.checkpoint()
    }
    fn consume(
        &self,
        charge: QueryCharge,
        units: u64,
    ) -> std::result::Result<(), QueryControlError> {
        self.budget.consume(charge, units)?;
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.at {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.budget.terminate(cause)
    }
}

#[test]
fn nested_traversal_observes_every_cancel_and_deadline_charge() {
    for input in [
        wrapper(vec![leaf(true), leaf(false)]),
        wrapper(vec![wrapper(vec![leaf(false)])]),
    ] {
        let run = |control: &dyn QueryControl| {
            subplans(
                &mut input.clone(),
                &[],
                BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
            )
        };
        let observer = Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            at: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        run(&observer).unwrap();
        for cause in [
            QueryControlError::Cancelled,
            QueryControlError::DeadlineExceeded,
        ] {
            for at in 1..=observer.calls.load(Ordering::SeqCst) {
                let stop = Stop {
                    budget: budget(u64::MAX),
                    calls: AtomicUsize::new(0),
                    at,
                    cause,
                };
                assert!(
                    matches!(run(&stop), Err(Error::QueryControl(actual)) if actual == cause),
                    "failed to stop at {at}"
                );
                assert_eq!(stop.checkpoint(), Err(cause));
            }
        }
    }
}

#[test]
fn nested_traversal_refuses_depth_on_the_default_stack() {
    let mut input = leaf(false);
    for _ in 0..crate::compile_envelope::algebra::MAX_ALGEBRA_DEPTH_V1 {
        input = wrapper(vec![input]);
    }
    let control = budget(u64::MAX);
    assert!(matches!(
        subplans(
            &mut input,
            &[],
            BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&control)))
        ),
        Err(Error::QueryControl(
            QueryControlError::CompilerEnvelopeExceeded
        ))
    ));
    assert_eq!(
        control.checkpoint(),
        Err(QueryControlError::CompilerEnvelopeExceeded)
    );
}
