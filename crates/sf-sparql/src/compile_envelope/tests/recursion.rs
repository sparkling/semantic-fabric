use super::*;

fn nested_operator_paths(outer_operators: usize, inner_operators: usize) -> String {
    format!(
        "{}({}",
        operator_chain(outer_operators),
        operator_chain(inner_operators)
    )
}

#[test]
fn should_accept_the_exact_recursion_potential_limit() {
    let input = nested_operator_paths(
        MAX_OPERATORS_PER_SCOPE_V1,
        MAX_RECURSION_POTENTIAL_V1 - MAX_OPERATORS_PER_SCOPE_V1 - 1,
    );
    let envelope = CompileEnvelopeV1::scan(&input).expect("exact recursion potential fits");

    assert_eq!(envelope.max_recursion_potential, MAX_RECURSION_POTENTIAL_V1);
}

#[test]
fn should_reject_one_unit_beyond_the_recursion_potential_limit() {
    let input = nested_operator_paths(
        MAX_OPERATORS_PER_SCOPE_V1,
        MAX_RECURSION_POTENTIAL_V1 - MAX_OPERATORS_PER_SCOPE_V1,
    );
    let error = CompileEnvelopeV1::scan(&input).expect_err("recursive path must be bounded");

    assert_limit(
        error,
        CompileEnvelopeLimit::RecursionPotential,
        MAX_RECURSION_POTENTIAL_V1 + 1,
        MAX_RECURSION_POTENTIAL_V1,
    );
}

#[test]
fn should_measure_a_parser_accepted_nested_boolean_path() {
    let outer = operator_chain(31);
    let inner = operator_chain(31);
    let input = format!("SELECT * WHERE {{ ?s ?p ?o . FILTER({outer} || ({inner})) }}");
    spargebra::SparqlParser::new()
        .parse_query(&input)
        .expect("the pinned parser accepts the bounded nested path");

    let envelope = CompileEnvelopeV1::scan(&input).expect("bounded nested path fits");
    assert!(envelope.max_recursion_potential >= 66);
}

#[test]
fn should_release_closed_and_separated_operator_paths() {
    let chain = operator_chain(MAX_OPERATORS_PER_SCOPE_V1);
    let input = format!("({chain});({chain});({chain})");
    let envelope = CompileEnvelopeV1::scan(&input).expect("siblings do not accumulate");

    assert_eq!(
        envelope.max_recursion_potential,
        MAX_OPERATORS_PER_SCOPE_V1 + 1
    );
}
