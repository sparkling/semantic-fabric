mod expr;
mod state;
use std::collections::HashMap;

use spargebra::algebra::{
    AggregateExpression, Expression, GraphPattern, OrderExpression, PropertyPathExpression,
    QueryDataset,
};
use spargebra::term::{
    GroundTerm, GroundTriple, NamedNodePattern, TermPattern, TriplePattern, Variable,
};
use spargebra::Query;

use super::AlphaVerdictV1;
use crate::compile_envelope::algebra::AlgebraEnvelopeV1;

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_WORK_RESERVATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[derive(Clone, Copy)]
enum BlankScope {
    Pattern,
    Template,
}

#[derive(Clone, Copy)]
enum Pair<'query> {
    Query(&'query Query, &'query Query),
    Dataset(&'query QueryDataset, &'query QueryDataset),
    Graph(&'query GraphPattern, &'query GraphPattern),
    Expression(&'query Expression, &'query Expression),
    Path(
        &'query PropertyPathExpression,
        &'query PropertyPathExpression,
    ),
    Aggregate(&'query AggregateExpression, &'query AggregateExpression),
    Order(&'query OrderExpression, &'query OrderExpression),
    Triple(&'query TriplePattern, &'query TriplePattern),
    Term(&'query TermPattern, &'query TermPattern),
    NamedPattern(&'query NamedNodePattern, &'query NamedNodePattern),
    GroundTerm(&'query GroundTerm, &'query GroundTerm),
    GroundTriple(&'query GroundTriple, &'query GroundTriple),
    Variable(&'query Variable, &'query Variable),
    Blank(&'query str, &'query str),
}

#[derive(Clone, Copy)]
struct Frame<'query> {
    pair: Pair<'query>,
    depth: usize,
    scope: BlankScope,
}

struct State<'query> {
    work: Vec<Frame<'query>>,
    visited: usize,
    node_limit: usize,
    depth_limit: usize,
    variables_lr: HashMap<&'query str, &'query str>,
    variables_rl: HashMap<&'query str, &'query str>,
    pattern_blanks_lr: HashMap<&'query str, &'query str>,
    pattern_blanks_rl: HashMap<&'query str, &'query str>,
    template_blanks_lr: HashMap<&'query str, &'query str>,
    template_blanks_rl: HashMap<&'query str, &'query str>,
}

pub(super) fn queries(left: &Query, right: &Query) -> AlphaVerdictV1 {
    let Ok(left_envelope) = AlgebraEnvelopeV1::validate(left) else {
        return AlphaVerdictV1::Inconclusive;
    };
    let Ok(right_envelope) = AlgebraEnvelopeV1::validate(right) else {
        return AlphaVerdictV1::Inconclusive;
    };
    compare_with_state(
        left,
        right,
        State::new(
            left_envelope
                .algebra_nodes
                .min(right_envelope.algebra_nodes),
            left_envelope.max_depth.min(right_envelope.max_depth),
        ),
    )
}

fn compare_with_state<'query>(
    left: &'query Query,
    right: &'query Query,
    mut state: State<'query>,
) -> AlphaVerdictV1 {
    let result = state.seed_select_outputs(left, right).and_then(|same| {
        if same {
            state.push(Pair::Query(left, right), 1, BlankScope::Pattern)?;
            state.run()
        } else {
            Ok(false)
        }
    });
    verdict(result)
}

fn verdict(result: Result<bool, ()>) -> AlphaVerdictV1 {
    match result {
        Ok(true) => AlphaVerdictV1::Equivalent,
        Ok(false) => AlphaVerdictV1::Different,
        Err(()) => AlphaVerdictV1::Inconclusive,
    }
}

#[cfg(test)]
pub(super) fn queries_with_limits(
    left: &Query,
    right: &Query,
    node_limit: usize,
    depth_limit: usize,
) -> AlphaVerdictV1 {
    compare_with_state(left, right, State::new(node_limit, depth_limit))
}

#[cfg(test)]
pub(super) fn fail_next_work_reservation() {
    FAIL_NEXT_WORK_RESERVATION.with(|failed| failed.set(true));
}

#[cfg(test)]
pub(super) fn identity_budget_exhaustion_verdict() -> AlphaVerdictV1 {
    let mut forward = HashMap::new();
    let mut reverse = HashMap::new();
    verdict(expr::bind(&mut forward, &mut reverse, "left", "right", 0))
}

#[cfg(test)]
fn take_forced_work_reservation_failure() -> bool {
    FAIL_NEXT_WORK_RESERVATION.with(|failed| failed.replace(false))
}
