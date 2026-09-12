use super::*;
use crate::iq::{Branch, SqlCond, SubPlanJoin, TermDef};
use sf_core::ir::{Template, TermMap, TermSpec};
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn empty() -> Plan {
    crate::parse_and_translate(
        "SELECT ?x WHERE { VALUES ?x { 1 } }",
        &[],
        sf_sql::Dialect::Sqlite,
    )
    .unwrap()
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
    fn consume(&self, kind: QueryCharge, units: u64) -> std::result::Result<(), QueryControlError> {
        self.budget.consume(kind, units)?;
        if self.calls.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.budget.terminate(cause)
    }
}

fn exercise(plan: &Plan, max: usize) {
    let expected = plan.source_sized_states_with_order_window(max);
    let run = |control: &dyn QueryControl| {
        plan.source_sized_states_with_order_window_and_work_control(max, control)
    };
    let measured = Stop {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        at: usize::MAX,
        cause: QueryControlError::Cancelled,
    };
    assert_eq!(run(&measured).unwrap(), expected);
    let units = measured.budget.consumed(QueryCharge::CompilerWork);
    assert_eq!(run(&budget(units)).unwrap(), expected);
    assert!(matches!(
        run(&budget(units - 1)),
        Err(crate::Error::QueryControl(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=measured.calls.load(Ordering::Relaxed) {
            let stop = Stop {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                at,
                cause,
            };
            assert!(
                matches!(run(&stop), Err(crate::Error::QueryControl(actual)) if actual == cause)
            );
            assert_eq!(stop.checkpoint(), Err(cause));
        }
    }
}

#[test]
fn fixed_empty_profile_has_hand_counted_work() {
    let mut plan = empty();
    plan.branches.clear();
    let control = budget(16); // Plan entry + five fixed predicates + five output count/read + five emission visits.
    assert!(plan
        .source_sized_states_with_order_window_and_work_control(0, &control)
        .unwrap()
        .is_empty());
    assert_eq!(control.consumed(QueryCharge::CompilerWork), 16);
    exercise(&plan, 0);
}

#[test]
fn nested_source_and_condition_profiles_preserve_all_states_and_every_stop() {
    use crate::iq::{OrderKey, RustGroup, Scan};
    for variant in 0..7 {
        let mut plan = empty();
        let mut branch = Branch::empty();
        let scan = Scan {
            alias: 1,
            source: sf_core::ir::LogicalSource::Table("items".into()).into(),
        };
        if variant == 0 {
            branch.core.push(scan);
        } else {
            branch
                .where_conds
                .push(SqlCond::Not(Box::new(SqlCond::And(vec![
                    SqlCond::ExpressionError,
                    SqlCond::Or(vec![SqlCond::Exists {
                        scans: vec![scan],
                        conds: vec![],
                    }]),
                ]))));
        }
        branch.distinct = true;
        branch.bindings.insert(
            "value".into(),
            TermDef::Derived {
                alias: 1,
                term_map: TermMap::Template(
                    Template::parse("{left}{right}").unwrap(),
                    if variant == 1 {
                        TermSpec::iri()
                    } else {
                        TermSpec::plain_literal()
                    },
                ),
            },
        );
        plan.branches = vec![branch.clone(), branch];
        plan.distinct = true;
        plan.order = vec![OrderKey {
            var: "value".into(),
            descending: false,
            expr: None,
        }];
        plan.rust_group = Some(RustGroup {
            keys: vec![],
            aggs: vec![],
            post_exprs: vec![],
        });
        if variant == 2 {
            plan.form = PlanForm::Ask;
        }
        if variant == 3 {
            plan.form = PlanForm::Construct { template: vec![] };
            plan.construct_drops_some_branch_var = true;
        }
        if variant == 4 {
            plan.limit = Some(0);
            plan.offset = usize::MAX;
        }
        if variant == 5 {
            plan.limit = Some(1);
            plan.offset = usize::MAX;
        }
        if variant == 6 {
            plan.limit = Some(2);
            plan.offset = 3;
        }
        exercise(&plan, 5);
        let mut outer = empty();
        outer.branches = vec![Branch::empty()];
        outer.branches[0].subplan_joins.push(SubPlanJoin {
            alias: 3,
            plan: Box::new(plan),
            on: vec![],
            left: false,
        });
        exercise(&outer, 5); // finite root allowance never exempts nested ordering.
    }
}

#[test]
fn depth_refusal_precedes_unbounded_condition_or_subplan_recursion() {
    for nested_plan in [false, true] {
        let mut plan = empty();
        if nested_plan {
            for _ in 0..150 {
                let mut outer = empty();
                outer.branches[0].subplan_joins.push(SubPlanJoin {
                    alias: 1,
                    plan: Box::new(plan),
                    on: vec![],
                    left: false,
                });
                plan = outer;
            }
        } else {
            let mut condition = SqlCond::ExpressionError;
            for _ in 0..150 {
                condition = SqlCond::Not(Box::new(condition));
            }
            plan.branches[0].where_conds.push(condition);
        }
        assert!(matches!(
            plan.source_sized_states_with_order_window_and_work_control(0, &budget(u64::MAX)),
            Err(crate::Error::QueryControl(
                QueryControlError::CompilerEnvelopeExceeded
            ))
        ));
    }
}
