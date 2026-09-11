use std::sync::atomic::{AtomicUsize, Ordering};

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use spargebra::SparqlParser;

use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::TermDef;
use crate::{CompilerWorkMode, Error};

fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX))
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

fn template(query: &str) -> Vec<TriplePattern> {
    let spargebra::Query::Construct { template, .. } =
        SparqlParser::new().parse_query(query).unwrap()
    else {
        panic!("construct fixture");
    };
    template
}

fn branch() -> Branch {
    let mut branch = Branch::empty();
    for key in ["s", "p", "o", "extra"] {
        branch.bindings.insert(
            key.to_owned(),
            TermDef::Const(
                sf_core::NamedNode::new(format!("http://ex/{key}"))
                    .unwrap()
                    .into(),
            ),
        );
    }
    branch
}

fn prove(raw: String, run: impl Fn(&dyn QueryControl) -> Result<String>) {
    let observer = Stop {
        budget: budget(u64::MAX),
        calls: AtomicUsize::new(0),
        stop: usize::MAX,
        cause: QueryControlError::Cancelled,
    };
    assert_eq!(run(&observer).unwrap(), raw);
    let units = observer.budget.consumed(QueryCharge::CompilerWork);
    assert!(units > 0);
    assert_eq!(run(&budget(units)).unwrap(), raw);
    assert!(matches!(
        run(&budget(units - 1)),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for stop in 1..=observer.calls.load(Ordering::SeqCst) {
            let control = Stop {
                budget: budget(u64::MAX),
                calls: AtomicUsize::new(0),
                stop,
                cause,
            };
            assert!(
                matches!(run(&control), Err(Error::QueryControl(actual)) if actual == cause),
                "stop {stop}"
            );
            assert_eq!(control.checkpoint(), Err(cause));
        }
    }
}

#[test]
fn construct_finalization_matches_raw_with_exact_budget_and_every_stop() {
    for query in [
        "CONSTRUCT { ?s ?p ?o } WHERE {}",
        "CONSTRUCT { _:fresh ?p ?o } WHERE {}",
        "CONSTRUCT { <http://ex/s> <http://ex/p> <http://ex/o> } WHERE {}",
        "CONSTRUCT { ?s <http://ex/p> ?missing } WHERE {}",
    ] {
        let template = template(query);
        let mut input = vec![branch(), branch()];
        input[1].limit = Some(2);
        let mut raw = input.clone();
        let drops = crate::dedup_construct_template_projected_vars(&mut raw, &template);
        prove(format!("{drops}:{raw:?}"), |control| {
            let mut branches = input.clone();
            let drops = construct(
                &mut branches,
                &template,
                BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
            )?;
            Ok(format!("{drops}:{branches:?}"))
        });
    }
}

#[test]
fn describe_finalization_matches_raw_with_exact_budget_and_every_stop() {
    let form = PlanForm::Construct {
        template: template("CONSTRUCT { ?s ?p ?o } WHERE {}"),
    };
    for count in [1, 2] {
        let input = vec![branch(); count];
        let mut raw = input.clone();
        let drops = crate::enforce_describe_graph_set(&mut raw, &form).unwrap();
        prove(format!("{drops}:{raw:?}"), |control| {
            let mut branches = input.clone();
            let drops = describe(
                &mut branches,
                &form,
                BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
            )?;
            Ok(format!("{drops}:{branches:?}"))
        });
    }
}
