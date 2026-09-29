use std::sync::atomic::{AtomicU64, Ordering};

use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
    UncontrolledQueryControl,
};
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, Function, GraphPattern,
    PropertyPathExpression,
};
use spargebra::term::{
    GroundTerm, GroundTriple, NamedNode, NamedNodePattern, TermPattern, TriplePattern, Variable,
};
use spargebra::{Query, SparqlParser};

use super::{screen_parsed_query_structure, ShapeRefusal, ShapeRule, StructuralOnlyScreen};

const OVER_BUDGET: QueryControlError = QueryControlError::CompilerWorkExceeded;
const EXCEEDED: ShapeRefusal = ShapeRefusal::Control(OVER_BUDGET);
const CANCELLED: ShapeRefusal = ShapeRefusal::Control(QueryControlError::Cancelled);
const DEPTH: usize = 50_000;
const BUDGET_QUERY: &str =
    "SELECT ?s WHERE { ?s <http://e/p> ?o OPTIONAL { ?o <http://e/q> 'v' } FILTER(?o != 3) }";
const SECRET_QUERY: &str =
    "SELECT * WHERE { SERVICE <http://secret.example/svc-xyz> { ?s ?p 'hidden-literal' } }";

const SAFE: [&str; 13] = [
    "SELECT ?s ?o WHERE { ?s <http://e/p> ?o }",
    "ASK { ?s <http://e/p> ?o }",
    "SELECT * WHERE { { ?s ?p ?o } UNION { ?o ?p ?s } }",
    "SELECT * WHERE { ?s ?p ?o OPTIONAL { ?o <http://e/q> ?z } }",
    "SELECT * WHERE { ?s ?p ?o BIND(STR(?s) AS ?t) }",
    "SELECT * WHERE { ?s ?p ?o FILTER(REGEX(STR(?o), 'a') && ?s IN (<http://e/a>)) }",
    "SELECT * WHERE { ?s ?p ?o FILTER NOT EXISTS { ?s <http://e/q> ?z } }",
    "SELECT ?s WHERE { ?s <http://e/p>/<http://e/q>* ?o } ORDER BY DESC(?s) LIMIT 10",
    "SELECT ?s (COUNT(?o) AS ?c) WHERE { ?s <http://e/p> ?o } GROUP BY ?s",
    "SELECT (GROUP_CONCAT(?o; SEPARATOR=',') AS ?g) WHERE { ?s ?p ?o }",
    "SELECT * WHERE { VALUES ?x { 1 'a' <http://e/i> UNDEF } ?x ?p ?o }",
    "SELECT * WHERE { ?s !(<http://e/p>|<http://e/q>) ?o }",
    "SELECT DISTINCT ?s WHERE { { SELECT ?s WHERE { ?s ?p ?o } } }",
];

const SERVICE_FIXTURES: [&str; 12] = [
    "SELECT * WHERE { SERVICE <http://e/s> { ?s ?p ?o } }",
    "SELECT * WHERE { SERVICE SILENT <http://e/s> { ?s ?p ?o } }",
    "SELECT * WHERE { ?s ?p ?o OPTIONAL { SERVICE <http://e/s> { ?s ?p ?x } } }",
    "SELECT * WHERE { { ?s ?p ?o } UNION { SERVICE <http://e/s> { ?s ?p ?o } } }",
    "SELECT * WHERE { ?s ?p ?o MINUS { SERVICE <http://e/s> { ?s ?p ?x } } }",
    "SELECT * WHERE { ?s ?p ?o FILTER EXISTS { SERVICE <http://e/s> {} } }",
    "SELECT * WHERE { ?s ?p ?o FILTER NOT EXISTS { SERVICE <http://e/s> {} } }",
    "SELECT * WHERE { ?s ?p ?o BIND(EXISTS { SERVICE <http://e/s> {} } AS ?b) }",
    "SELECT * WHERE { { SELECT ?s WHERE { SERVICE <http://e/s> { ?s ?p ?o } } } }",
    "SELECT * WHERE { GRAPH ?g { SERVICE <http://e/s> { ?s ?p ?o } } }",
    "ASK { SERVICE <http://e/s> { ?s ?p ?o } }",
    "SELECT * WHERE { ?s ?p ?o OPTIONAL { ?o ?q ?x FILTER EXISTS { SERVICE <http://e/s> {} } } }",
];

const KEYWORD_ONLY: [&str; 4] = [
    "SELECT * WHERE { ?s ?p ?o FILTER(?o = 'SERVICE <http://e/s> { ?s ?p ?x }') }",
    "SELECT * WHERE { ?s <http://e/service> ?o }",
    "SELECT * WHERE { ?service <http://e/p> ?o }",
    "SELECT * WHERE { ?s ?p ?o BIND('SERVICE' AS ?service) }",
];

const DATASET_FIXTURES: [&str; 3] = [
    "SELECT * FROM <http://e/g> WHERE { ?s ?p ?o }",
    "SELECT * FROM NAMED <http://e/g> WHERE { ?s ?p ?o }",
    "ASK FROM <http://e/g> { ?s ?p ?o }",
];

const CUSTOM_FUNCTION_FIXTURES: [&str; 2] = [
    "SELECT * WHERE { ?s ?p ?o FILTER(<http://e/fn>(?o)) }",
    "SELECT * WHERE { ?s ?p ?o FILTER EXISTS { ?o ?q ?x FILTER(<http://e/fn>(?x)) } }",
];

struct CancelOnConsume {
    budget: QueryBudget,
    remaining: AtomicU64,
}

impl QueryControl for CancelOnConsume {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        self.budget.checkpoint()
    }

    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        if self.remaining.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.budget.terminate(QueryControlError::Cancelled);
        }
        self.budget.consume(charge, amount)
    }

    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }
}

fn parse(sparql: &str) -> Query {
    SparqlParser::new().parse_query(sparql).unwrap()
}

fn budget(limit: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(limit, u64::MAX, u64::MAX, u64::MAX))
}

fn compiler_work(control: &QueryBudget) -> u64 {
    control.consumed(QueryCharge::CompilerWork)
}

fn screen(sparql: &str) -> Result<StructuralOnlyScreen, ShapeRefusal> {
    screen_parsed_query_structure(&parse(sparql), &UncontrolledQueryControl)
}

fn screen_query(query: &Query) -> Result<StructuralOnlyScreen, ShapeRefusal> {
    screen_parsed_query_structure(query, &UncontrolledQueryControl)
}

// Measure the same parsed instance later screened: the parser mints random
// hex names for aggregate variables, so separate parses may differ in bytes.
fn cost(query: &Query) -> u64 {
    let measuring = budget(u64::MAX);
    let charged = screen_parsed_query_structure(query, &measuring)
        .unwrap()
        .charged_compiler_work();
    assert_eq!(compiler_work(&measuring), charged);
    charged
}

fn refused_rule(result: Result<StructuralOnlyScreen, ShapeRefusal>) -> ShapeRule {
    match result {
        Err(ShapeRefusal::Rule(rule)) => rule,
        other => panic!("expected rule refusal, got {other:?}"),
    }
}

fn assert_rule(query: &Query, rule: ShapeRule) {
    assert_eq!(refused_rule(screen_query(query)), rule);
}

fn assert_refused_before_work(sparql: &str, rule: ShapeRule) {
    let control = budget(u64::MAX);
    let result = screen_parsed_query_structure(&parse(sparql), &control);
    assert_eq!(refused_rule(result), rule, "{sparql}");
    assert_eq!(compiler_work(&control), 0, "{sparql}");
}

fn variable(name: &str) -> Variable {
    Variable::new_unchecked(name)
}

fn named(iri: &str) -> NamedNode {
    NamedNode::new_unchecked(iri)
}

fn select(pattern: GraphPattern) -> Query {
    Query::Select {
        dataset: None,
        pattern,
        base_iri: None,
    }
}

fn quoted() -> TermPattern {
    TermPattern::Triple(Box::new(TriplePattern {
        subject: TermPattern::Variable(variable("s")),
        predicate: NamedNodePattern::NamedNode(named("http://e/q")),
        object: TermPattern::Variable(variable("o")),
    }))
}

fn wrap_distinct(mut pattern: GraphPattern, depth: usize) -> GraphPattern {
    for _ in 0..depth {
        pattern = GraphPattern::Distinct {
            inner: Box::new(pattern),
        };
    }
    pattern
}

fn dismantle(mut pattern: GraphPattern) {
    while let GraphPattern::Distinct { inner } = &mut pattern {
        let next = std::mem::take(&mut **inner);
        pattern = next;
    }
}

fn dismantle_query(query: Query) {
    if let Query::Select { pattern, .. } = query {
        dismantle(pattern);
    }
}

fn dismantle_expression(mut expression: Expression) {
    while let Expression::Not(inner) = expression {
        expression = *inner;
    }
}

fn dismantle_filter(query: Query) {
    if let Query::Select { pattern, .. } = query {
        if let GraphPattern::Filter { expr, .. } = pattern {
            dismantle_expression(expr);
        }
    }
}

fn walk_deep_nesting() {
    let safe = select(wrap_distinct(GraphPattern::default(), DEPTH));
    assert!(screen_query(&safe).is_ok());
    dismantle_query(safe);

    let service = GraphPattern::Service {
        name: NamedNodePattern::NamedNode(named("http://e/s")),
        inner: Box::new(GraphPattern::default()),
        silent: false,
    };
    let deep = select(wrap_distinct(service, DEPTH));
    assert_rule(&deep, ShapeRule::ServiceInPattern);
    dismantle_query(deep);

    let mut expression = Expression::Variable(variable("x"));
    for _ in 0..DEPTH {
        expression = Expression::Not(Box::new(expression));
    }
    let filter = select(GraphPattern::Filter {
        expr: expression,
        inner: Box::new(GraphPattern::default()),
    });
    assert!(screen_query(&filter).is_ok());
    dismantle_filter(filter);
}

#[test]
fn safe_select_and_ask_pass_structurally() {
    for sparql in SAFE {
        let result = screen(sparql);
        assert!(result.is_ok(), "safe refused: {sparql}: {result:?}");
    }
}

#[test]
fn construct_and_describe_refused_by_named_rule_before_any_work() {
    assert_refused_before_work(
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
        ShapeRule::ConstructForm,
    );
    assert_refused_before_work("DESCRIBE <http://e/x>", ShapeRule::DescribeForm);
}

#[test]
fn service_refused_wherever_it_is_nested() {
    for sparql in SERVICE_FIXTURES {
        let rule = refused_rule(screen(sparql));
        assert_eq!(rule, ShapeRule::ServiceInPattern, "{sparql}");
    }
}

#[test]
fn dataset_clauses_are_typed_refusals_before_any_work() {
    for sparql in DATASET_FIXTURES {
        assert_refused_before_work(sparql, ShapeRule::DatasetClause);
    }
}

#[test]
fn service_keyword_in_literal_variable_or_iri_stays_allowed() {
    for sparql in KEYWORD_ONLY {
        let result = screen(sparql);
        assert!(result.is_ok(), "keyword refused: {sparql}: {result:?}");
    }
}

#[test]
fn rdf_star_terms_are_refused_in_bgp_path_and_values() {
    let bgp = select(GraphPattern::Bgp {
        patterns: vec![TriplePattern {
            subject: TermPattern::Variable(variable("x")),
            predicate: NamedNodePattern::NamedNode(named("http://e/p")),
            object: quoted(),
        }],
    });
    assert_rule(&bgp, ShapeRule::RdfStarUnsupported);

    let path = select(GraphPattern::Path {
        subject: quoted(),
        path: PropertyPathExpression::NamedNode(named("http://e/p")),
        object: TermPattern::Variable(variable("o")),
    });
    assert_rule(&path, ShapeRule::RdfStarUnsupported);

    let ground = GroundTerm::Triple(Box::new(GroundTriple {
        subject: named("http://e/s"),
        predicate: named("http://e/p"),
        object: GroundTerm::NamedNode(named("http://e/o")),
    }));
    let values = select(GraphPattern::Values {
        variables: vec![variable("x")],
        bindings: vec![vec![Some(ground)]],
    });
    assert_rule(&values, ShapeRule::RdfStarUnsupported);

    let text = "SELECT * WHERE { ?x <http://e/p> <<( ?s <http://e/q> ?o )>> }";
    if let Ok(parsed) = SparqlParser::new().parse_query(text) {
        assert_rule(&parsed, ShapeRule::RdfStarUnsupported);
    }
}

#[test]
fn rdf_star_function_is_refused() {
    let argument = Expression::Variable(variable("o"));
    let filter = select(GraphPattern::Filter {
        expr: Expression::FunctionCall(Function::IsTriple, vec![argument]),
        inner: Box::new(GraphPattern::default()),
    });
    assert_rule(&filter, ShapeRule::RdfStarUnsupported);

    let text = "SELECT * WHERE { ?s ?p ?o FILTER(isTRIPLE(?o)) }";
    if let Ok(parsed) = SparqlParser::new().parse_query(text) {
        assert_rule(&parsed, ShapeRule::RdfStarUnsupported);
    }
}

#[test]
fn lateral_is_refused_when_the_parser_accepts_it() {
    let text = "SELECT * WHERE { ?s ?p ?o LATERAL { ?o ?q ?x } }";
    if let Ok(parsed) = SparqlParser::new().parse_query(text) {
        assert_rule(&parsed, ShapeRule::UnclassifiedForm);
    }
}

#[test]
fn custom_function_is_refused_even_when_nested() {
    for sparql in CUSTOM_FUNCTION_FIXTURES {
        let rule = refused_rule(screen(sparql));
        assert_eq!(rule, ShapeRule::CustomFunctionUnsupported, "{sparql}");
    }
}

#[test]
fn custom_aggregate_is_refused() {
    let aggregate = AggregateExpression::FunctionCall {
        name: AggregateFunction::Custom(named("http://e/agg")),
        expr: Expression::Variable(variable("x")),
        distinct: false,
    };
    let query = select(GraphPattern::Group {
        inner: Box::new(GraphPattern::default()),
        variables: Vec::new(),
        aggregates: vec![(variable("c"), aggregate)],
    });
    assert_rule(&query, ShapeRule::CustomAggregateUnsupported);
}

#[test]
fn budget_exact_passes_and_one_less_fails_with_sticky_cause() {
    for sparql in SAFE.iter().copied().chain([BUDGET_QUERY]) {
        let query = parse(sparql);
        let full = cost(&query);
        assert!(full > 1, "{sparql}");

        let exact = budget(full);
        let screened = screen_parsed_query_structure(&query, &exact).unwrap();
        assert_eq!(screened.charged_compiler_work(), full, "{sparql}");
        assert_eq!(compiler_work(&exact), full, "{sparql}");
        assert_eq!(exact.terminal(), None, "{sparql}");

        let short = budget(full - 1);
        let result = screen_parsed_query_structure(&query, &short);
        assert_eq!(result.unwrap_err(), EXCEEDED, "{sparql}");
        assert_eq!(short.terminal(), Some(OVER_BUDGET), "{sparql}");
        let consumed = compiler_work(&short);
        assert!(consumed < full, "{sparql}");
        let later = parse("ASK { ?s ?p ?o }");
        let result = screen_parsed_query_structure(&later, &short);
        assert_eq!(result.unwrap_err(), EXCEEDED, "{sparql}");
        assert_eq!(compiler_work(&short), consumed, "{sparql}");
    }
}

#[test]
fn constant_bytes_are_charged_per_byte() {
    let short = cost(&parse("SELECT * WHERE { ?s ?p ?o FILTER(?o = 'a') }"));
    let long = cost(&parse(
        "SELECT * WHERE { ?s ?p ?o FILTER(?o = 'aaaaaaaaaa') }",
    ));
    assert_eq!(long - short, 9);
}

#[test]
fn cancelled_control_stops_before_any_work() {
    let control = budget(u64::MAX);
    control.terminate(QueryControlError::Cancelled);
    let result = screen_parsed_query_structure(&parse(BUDGET_QUERY), &control);
    assert_eq!(result.unwrap_err(), CANCELLED);
    assert_eq!(compiler_work(&control), 0);
}

#[test]
fn cancellation_mid_walk_is_sticky() {
    let query = parse(BUDGET_QUERY);
    let full = cost(&query);
    let control = CancelOnConsume {
        budget: budget(u64::MAX),
        remaining: AtomicU64::new(3),
    };
    let result = screen_parsed_query_structure(&query, &control);
    assert_eq!(result.unwrap_err(), CANCELLED);
    assert!(compiler_work(&control.budget) < full);
    assert_eq!(control.checkpoint(), Err(QueryControlError::Cancelled));
    let again = screen_parsed_query_structure(&query, &control);
    assert_eq!(again.unwrap_err(), CANCELLED);
}

#[test]
fn uncontrolled_control_passes() {
    assert!(screen(BUDGET_QUERY).is_ok());
}

#[test]
fn input_query_is_unchanged() {
    for sparql in SAFE.iter().chain(SERVICE_FIXTURES.iter()) {
        let query = parse(sparql);
        let before = query.clone();
        let _ = screen_query(&query);
        assert_eq!(query, before, "{sparql}");
    }
}

#[test]
fn refusal_text_is_redacted() {
    let refusal = screen(SECRET_QUERY).unwrap_err();
    let rendered = format!("{refusal} {refusal:?}");
    for secret in ["secret.example", "svc-xyz", "hidden-literal"] {
        assert!(!rendered.contains(secret), "leaked {secret}: {rendered}");
    }
}

#[test]
fn deep_nesting_is_walked_iteratively() {
    let worker = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(walk_deep_nesting)
        .unwrap();
    worker.join().unwrap();
}
