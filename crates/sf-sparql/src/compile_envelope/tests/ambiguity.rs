use super::*;

#[test]
fn should_not_hide_boolean_chains_inside_an_iri_shaped_expression() {
    let mut expression = String::from("?x<1");
    for _ in 0..MAX_OPERATORS_PER_SCOPE_V1 {
        expression.push_str("&&1");
    }
    expression.push_str(">0");
    let input = format!("SELECT * WHERE {{ ?s ?p ?x . FILTER({expression}) }}");
    spargebra::SparqlParser::new()
        .parse_query(&input)
        .expect("the pinned parser accepts the compact boolean expression");

    let error = CompileEnvelopeV1::scan(&input).expect_err("operator chain must be visible");
    assert_limit(
        error,
        CompileEnvelopeLimit::OperatorsPerScope,
        MAX_OPERATORS_PER_SCOPE_V1 + 1,
        MAX_OPERATORS_PER_SCOPE_V1,
    );
}
