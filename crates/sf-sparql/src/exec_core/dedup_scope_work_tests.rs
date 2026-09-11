use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::{Scan, SubPlanJoin};
use crate::Plan;
use sf_core::ir::{LogicalSource, TermSpec};
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use sf_sql::Dialect;
use std::sync::atomic::{AtomicUsize, Ordering};

fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX))
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
fn leaf(alias: usize) -> Branch {
    let mut branch = Branch::single(Scan {
        alias,
        source: LogicalSource::Table(format!("source_{alias}")).into(),
    });
    branch.bindings.insert(
        "v".into(),
        TermDef::Derived {
            term_map: TermMap::Column("value".into(), TermSpec::plain_literal()),
            alias,
        },
    );
    branch
}
fn wrap(inner: Branch, alias: usize) -> Branch {
    let mut outer = Branch::empty();
    let def = inner.bindings["v"].clone();
    let projection = inner.projection();
    outer.bindings.insert(
        "v".into(),
        crate::iq::lower::remap_termdef(&def, &projection, alias).unwrap(),
    );
    outer.subplan_joins.push(SubPlanJoin {
        alias,
        left: false,
        on: Vec::new(),
        plan: Box::new(Plan {
            branches: vec![inner],
            form: PlanForm::Select {
                vars: vec!["v".into()],
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
    outer
}
fn markers() -> HashMap<usize, DedupMarker> {
    [1, 2]
        .into_iter()
        .map(|alias| {
            (
                alias,
                DedupMarker {
                    group_id: 7,
                    key_bindings: leaf(alias).bindings,
                },
            )
        })
        .collect()
}

#[test]
fn scope_lifting_matches_raw_exact_budget_and_every_stop() {
    for branches in [
        vec![leaf(1), leaf(2)],
        vec![wrap(wrap(leaf(1), 3), 4), wrap(leaf(2), 5)],
    ] {
        let markers = markers();
        let raw = crate::exec_core::lift_dedup_scopes(&branches, &markers).unwrap();
        let observer = Stop {
            budget: budget(u64::MAX),
            calls: AtomicUsize::new(0),
            at: usize::MAX,
            cause: QueryControlError::Cancelled,
        };
        let run = |control: &dyn QueryControl| {
            lift(
                &branches,
                &markers,
                BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
            )
        };
        assert_eq!(format!("{:?}", run(&observer).unwrap()), format!("{raw:?}"));
        let units = observer.budget.consumed(QueryCharge::CompilerWork);
        assert_eq!(
            format!("{:?}", run(&budget(units)).unwrap()),
            format!("{raw:?}")
        );
        assert!(matches!(
            run(&budget(units - 1)),
            Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
        ));
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
                    "stop {at}"
                );
                assert_eq!(stop.checkpoint(), Err(cause));
            }
        }
    }
}

#[test]
fn scope_lifting_preserves_missing_multiply_owned_and_impure_rejections() {
    let mut impure = wrap(leaf(1), 3);
    impure.subplan_joins[0].left = true;
    for branches in [
        vec![leaf(1)],
        vec![leaf(1), leaf(1), leaf(2)],
        vec![impure, leaf(2)],
    ] {
        let markers = markers();
        let raw = crate::exec_core::lift_dedup_scopes(&branches, &markers).unwrap_err();
        let control = budget(u64::MAX);
        let actual = lift(
            &branches,
            &markers,
            BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&control))),
        )
        .unwrap_err();
        assert_eq!(actual.to_string(), raw.to_string());
    }
}

#[test]
fn marker_order_preserves_exact_work_and_semantic_error() {
    let branches = vec![leaf(1), leaf(2), leaf(3)];
    for malformed in [false, true] {
        let mut expected = None;
        for order in [
            [1, 2, 3],
            [3, 1, 2],
            [2, 3, 1],
            [3, 2, 1],
            [2, 1, 3],
            [1, 3, 2],
        ] {
            let mut markers = HashMap::with_capacity(3);
            for alias in order {
                let mut keys = leaf(alias).bindings;
                if malformed && alias == 3 {
                    keys.clear();
                }
                markers.insert(
                    alias,
                    DedupMarker {
                        group_id: if alias == 3 { 8 } else { 7 },
                        key_bindings: keys,
                    },
                );
            }
            let control = budget(u64::MAX);
            let run = |control: &QueryBudget| {
                lift(
                    &branches,
                    &markers,
                    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
                )
            };
            let error = run(&control).unwrap_err();
            assert!(matches!(error, Error::Unsupported(_)));
            let units = control.consumed(QueryCharge::CompilerWork);
            let observation = (units, error.to_string());
            if let Some(expected) = &expected {
                assert_eq!(&observation, expected);
            } else {
                expected = Some(observation);
            }
            assert!(matches!(run(&budget(units)), Err(Error::Unsupported(_))));
            assert!(matches!(
                run(&budget(units - 1)),
                Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
            ));
        }
    }
}

#[test]
fn recipe_comparison_matches_complete_raw_identity_and_every_stop() {
    use crate::iq::{AggKind, ColRef, R2rmlGraphScope};
    use sf_core::ir::Template;
    let column = leaf(1).bindings["v"].clone();
    let constant = TermDef::Const(sf_core::Literal::new_simple_literal("quote\"\\ café").into());
    let blank = TermDef::R2rmlBlank {
        alias: 1,
        term_map: TermMap::Template(
            Template::parse("b/{value}").unwrap(),
            TermSpec::blank_node(),
        ),
        graph: R2rmlGraphScope::Mapped {
            alias: 2,
            term_map: TermMap::Template(Template::parse("urn:{value}").unwrap(), TermSpec::iri()),
        },
    };
    let aggregate = TermDef::Agg {
        col: ColRef::new(1, "value"),
        kind: AggKind::Avg,
        operand: Some(ColRef::new(2, "value")),
        fixed_type: None,
    };
    let mut recipes = vec![
        constant.clone(),
        column.clone(),
        blank.clone(),
        aggregate.clone(),
        TermDef::Coalesce(Box::new(column.clone()), Box::new(constant.clone())),
        TermDef::Concat(vec![column.clone(), blank.clone()]),
        TermDef::ComposedTriple {
            subject: Box::new(column.clone()),
            predicate: Box::new(constant.clone()),
            object: Box::new(blank.clone()),
        },
    ];
    let mut other = blank;
    if let TermDef::R2rmlBlank { graph, .. } = &mut other {
        *graph = R2rmlGraphScope::Default;
    }
    recipes.push(other);
    let mut other = aggregate;
    if let TermDef::Agg { operand, .. } = &mut other {
        *operand = None;
    }
    recipes.push(other);
    recipes.push(TermDef::Concat(vec![constant, column]));
    for left in &recipes {
        for right in &recipes {
            let expected = format!("{left:?}") == format!("{right:?}");
            let run = |control: &dyn QueryControl| {
                remap::same(
                    left,
                    right,
                    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
                )
            };
            let observer = Stop {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                at: usize::MAX,
                cause: QueryControlError::Cancelled,
            };
            assert_eq!(run(&observer).unwrap(), expected);
            let units = observer.budget.consumed(QueryCharge::CompilerWork);
            assert_eq!(run(&budget(units)).unwrap(), expected);
            assert!(matches!(
                run(&budget(units - 1)),
                Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
            ));
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
                        "comparison failed to stop at {at}: {left:?} versus {right:?}"
                    );
                    assert_eq!(stop.checkpoint(), Err(cause));
                }
            }
        }
    }
}

#[test]
fn nested_recipe_comparison_refuses_depth_on_the_default_test_stack() {
    let mut recipe = leaf(1).bindings["v"].clone();
    for _ in 0..crate::compile_envelope::algebra::MAX_ALGEBRA_DEPTH_V1 {
        recipe = TermDef::Concat(vec![recipe]);
    }
    let control = budget(u64::MAX);
    assert!(matches!(
        remap::same(
            &recipe,
            &recipe,
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
