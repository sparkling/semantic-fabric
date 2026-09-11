use std::sync::atomic::{AtomicUsize, Ordering};

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use spargebra::term::{TriplePattern, Variable};

use super::*;
use crate::compiler_control::CompileContext;
use crate::{CompilerWorkMode, Error, Result};

fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX))
}

fn mode(control: &dyn QueryControl) -> CompilerWorkMode<'_> {
    CompilerWorkMode::Metered(CompileContext::new(control))
}

struct Stop {
    budget: QueryBudget,
    calls: AtomicUsize,
    stop: usize,
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
        if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == self.stop {
            self.budget.terminate(self.cause);
        }
        Ok(())
    }
    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.budget.terminate(cause)
    }
}

pub(super) fn prove(raw: String, run: impl Fn(&dyn QueryControl) -> Result<String>) {
    let observed = Stop {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        stop: usize::MAX,
        cause: QueryControlError::Cancelled,
    };
    assert_eq!(run(&observed).unwrap(), raw);
    let units = observed.budget.consumed(QueryCharge::CompilerWork);
    assert!(units > 0);
    assert_eq!(run(&budget(units)).unwrap(), raw);
    assert!(matches!(
        run(&budget(units - 1)),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(observed.budget.consumed(QueryCharge::SourceWork), 0);
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for stop in 1..=observed.calls.load(Ordering::SeqCst) {
            let c = Stop {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                stop,
                cause,
            };
            assert!(
                matches!(run(&c), Err(Error::QueryControl(actual)) if actual == cause),
                "stop={stop}"
            );
            assert_eq!(c.checkpoint(), Err(cause));
        }
    }
}

fn env() -> StarEnv {
    [("z", ["a", "p", "o"]), ("a", ["s", "p", "o"])]
        .into_iter()
        .map(|(name, [s, p, o])| {
            (
                Variable::new_unchecked(name),
                ComposedInfo {
                    s_var: Variable::new_unchecked(s),
                    p_var: Variable::new_unchecked(p),
                    o_var: Variable::new_unchecked(o),
                },
            )
        })
        .collect()
}

#[test]
fn realization_template_and_fixed_point_names_match_raw_at_every_stop() {
    let env = env();
    let template = vec![TriplePattern {
        subject: Variable::new_unchecked("z").into(),
        predicate: Variable::new_unchecked("p").into(),
        object: Variable::new_unchecked("z").into(),
    }];
    prove(
        format!("{:?}", substitute_construct_template(&template, &env)),
        |c| {
            Ok(format!(
                "{:?}",
                substitute_construct_template_with_work_mode(&template, &env, mode(c))?
            ))
        },
    );
    let vars = vec!["z".to_owned(), "z".to_owned(), "unrelated".to_owned()];
    prove(
        format!("{:?}", expand_projection_for_cascade(&vars, &env)),
        |c| {
            Ok(format!(
                "{:?}",
                expand_projection_for_cascade_with_work_mode(&vars, &env, mode(c))?
            ))
        },
    );
    let sorted = |set: std::collections::HashSet<String>| {
        let mut v: Vec<_> = set.into_iter().collect();
        v.sort();
        format!("{v:?}")
    };
    prove(sorted(all_component_var_names(&env)), |c| {
        Ok(sorted(all_component_var_names_with_work_mode(
            &env,
            mode(c),
        )?))
    });
}

#[test]
fn realization_empty_env_projection_has_hand_counted_copies_and_raw_order() {
    let vars = vec!["x".to_owned(), "x".to_owned()];
    let units = 2 + 2 * std::mem::size_of::<String>() as u64 + 4;
    let c = budget(units);
    assert_eq!(
        expand_projection_for_cascade_with_work_mode(&vars, &StarEnv::new(), mode(&c)).unwrap(),
        vars
    );
    assert_eq!(c.consumed(QueryCharge::CompilerWork), units);
    assert!(expand_projection_for_cascade_with_work_mode(
        &vars,
        &StarEnv::new(),
        mode(&budget(units - 1))
    )
    .is_err());
}

#[test]
fn realization_composed_def_is_dynamic_and_missing_component_short_circuits() {
    let mut env = env();
    let bindings = ["s", "p", "o"]
        .into_iter()
        .map(|n| {
            (
                n.to_owned(),
                crate::iq::TermDef::Const(sf_core::Term::NamedNode(
                    sf_core::NamedNode::new_unchecked(format!("urn:{n}")),
                )),
            )
        })
        .collect();
    let var = Variable::new_unchecked("z");
    prove(
        format!("{:?}", super::env::composed_term_def(&var, &env, &bindings)),
        |c| {
            Ok(format!(
                "{:?}",
                composed_term_def_with_work_mode(&var, &env, &bindings, mode(c))?
            ))
        },
    );
    // Missing subject must return None without touching the cyclic object.
    env.get_mut(&var).unwrap().s_var = Variable::new_unchecked("missing");
    env.get_mut(&var).unwrap().o_var = var.clone();
    assert!(
        composed_term_def_with_work_mode(&var, &env, &bindings, mode(&budget(u64::MAX)))
            .unwrap()
            .is_none()
    );
    env.get_mut(&var).unwrap().s_var = var.clone();
    assert!(matches!(
        composed_term_def_with_work_mode(&var, &env, &bindings, mode(&budget(u64::MAX))),
        Err(Error::QueryControl(
            QueryControlError::CompilerEnvelopeExceeded
        ))
    ));
}
