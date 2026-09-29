use sf_core::query_control::UncontrolledQueryControl;
use spargebra::SparqlParser;

use super::*;

type Seen = Vec<(String, ConstantRole)>;

const S: &str = "http://e/s";
const P: &str = "http://e/p";
const Q: &str = "http://e/q";
const R: &str = "http://e/r";
const O: &str = "http://e/o";
const A: &str = "http://e/a";
const B: &str = "http://e/b";
const C: &str = "http://e/C";
const G: &str = "http://e/g";
const X: &str = "http://e/x";
const Y: &str = "http://e/y";
const Z: &str = "http://e/z";
const W: &str = "http://e/w";
const DT: &str = "http://e/dt";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

const BGP: &str = "SELECT * WHERE { <http://e/s> <http://e/p> <http://e/o> }";
const ASK_BGP: &str = "ASK { <http://e/s> <http://e/p> <http://e/o> }";
const TYPED: &str = "ASK { ?x a <http://e/C> . ?x <http://e/p> <http://e/C> }";
const TYPE_LIT: &str = "ASK { ?x a 'v' . ?x a ?c }";
const TYPE_OBJ: &str = "ASK { ?s ?p <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> }";
const GRAPH: &str = "SELECT * WHERE { GRAPH <http://e/g> { ?s <http://e/p> ?o } }";
const GRAPH_SAME: &str = "ASK { GRAPH <http://e/p> { ?s <http://e/p> ?o } }";
const GRAPH_VAR: &str = "ASK { GRAPH ?g { ?s <http://e/p> ?o } }";
const SAME_NODE: &str = "ASK { <http://e/a> <http://e/p> <http://e/a> }";
const TWO_TRIPLES: &str = "ASK { ?s <http://e/p> ?o . ?t <http://e/p> ?u }";
const LIT_TYPED: &str = "ASK { ?s <http://e/p> '5'^^<http://e/dt> }";
const LIT_PLAIN: &str = "ASK { ?s <http://e/p> 'v' . ?s <http://e/q> 'v'@en }";
const FILTER_LIT: &str = "ASK { ?s ?p ?o FILTER(?o = '5'^^<http://e/dt>) }";
const EXPR_EQ: &str = "ASK { ?s ?p ?o FILTER(?o = <http://e/x>) }";
const EXPR_IN: &str = "ASK { ?s ?p ?o FILTER(?s IN (<http://e/y>, <http://e/z>)) }";
const BIND_IRI: &str = "ASK { ?s ?p ?o BIND(<http://e/w> AS ?b) }";
const ORDER_BY: &str = "SELECT * WHERE { ?s ?p ?o } ORDER BY DESC(<http://e/x>)";
const AGG_IRI: &str = "SELECT (MAX(<http://e/x>) AS ?m) WHERE { ?s ?p ?o }";
const EXISTS: &str = "ASK { FILTER EXISTS { GRAPH <http://e/g> { ?s <http://e/q> ?o } } }";
const NESTED: &str = "ASK { ?s <http://e/p> ?o OPTIONAL { ?o <http://e/q> <http://e/r> } }";
const VALUES: &str = "ASK { VALUES ?x { <http://e/a> 1 UNDEF 'v'^^<http://e/dt> } }";
const PATH_PLUS: &str = "ASK { <http://e/a> <http://e/p>+ <http://e/b> }";
const PATH_NEG: &str = "ASK { <http://e/a> !(<http://e/p>|<http://e/q>) <http://e/b> }";
const PATH_REV: &str = "ASK { <http://e/a> (^<http://e/p>)+ <http://e/b> }";
const PATH_ALT: &str = "ASK { <http://e/a> (<http://e/p>|<http://e/q>/<http://e/r>)* ?o }";
const PATH_TYPE: &str = "ASK { ?s (<http://e/p>|a)+ <http://e/C> }";
const MIXED: &str = "ASK { GRAPH <http://e/g> { ?s a <http://e/C> } FILTER(?s != <http://e/x>) }";
const DATASET: &str = "ASK FROM <http://e/g> { ?s ?p ?o }";
const CONSTRUCT_Q: &str = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
const SERVICE_Q: &str = "ASK { SERVICE <http://e/s> { ?s ?p ?o } }";
const CUSTOM_Q: &str = "ASK { ?s ?p ?o FILTER(<http://e/fn>(?o)) }";

struct Want(Seen);

impl Want {
    fn new() -> Self {
        Self(Seen::new())
    }

    fn add(&mut self, iri: &str, role: ConstantRole) {
        self.0.push((iri.to_owned(), role));
    }

    fn done(mut self) -> Seen {
        self.0.sort();
        self.0
    }
}

fn parse(sparql: &str) -> Query {
    SparqlParser::new().parse_query(sparql).unwrap()
}

fn constants(sparql: &str) -> Seen {
    let query = parse(sparql);
    let mut seen = Seen::new();
    let result = visit_parsed_query_constants(&query, &UncontrolledQueryControl, |found| {
        seen.push((found.iri().to_owned(), found.role()));
        Ok(())
    });
    result.unwrap();
    seen.sort();
    seen
}

fn check(sparql: &str, want: Want) {
    assert_eq!(constants(sparql), want.done(), "{sparql}");
}

fn check_unresolved(sparql: &str, iri: &str) {
    let mut want = Want::new();
    want.add(iri, ConstantRole::Unresolved);
    check(sparql, want);
}

fn check_path(sparql: &str, predicates: &[&str]) {
    let mut want = Want::new();
    want.add(A, ConstantRole::Unresolved);
    want.add(B, ConstantRole::Unresolved);
    for iri in predicates {
        want.add(iri, ConstantRole::Predicate);
    }
    check(sparql, want);
}

fn assert_refused(sparql: &str, rule: ShapeRule) {
    let query = parse(sparql);
    let mut calls = 0;
    let result = visit_parsed_query_constants(&query, &UncontrolledQueryControl, |_| {
        calls += 1;
        Ok(())
    });
    assert_eq!(result.unwrap_err(), ShapeRefusal::Rule(rule), "{sparql}");
    assert_eq!(calls, 0, "{sparql}");
}

fn spo() -> Want {
    let mut want = Want::new();
    want.add(S, ConstantRole::Subject);
    want.add(P, ConstantRole::Predicate);
    want.add(O, ConstantRole::Object);
    want
}

#[test]
fn bgp_terms_have_subject_predicate_object_roles() {
    check(BGP, spo());
    check(ASK_BGP, spo());
}

#[test]
fn exact_rdf_type_object_is_class_and_nothing_else_is() {
    let mut want = Want::new();
    want.add(C, ConstantRole::Class);
    want.add(C, ConstantRole::Object);
    want.add(P, ConstantRole::Predicate);
    want.add(RDF_TYPE, ConstantRole::Predicate);
    check(TYPED, want);

    let mut plain = Want::new();
    plain.add(RDF_TYPE, ConstantRole::Predicate);
    plain.add(RDF_TYPE, ConstantRole::Predicate);
    check(TYPE_LIT, plain);

    let mut as_object = Want::new();
    as_object.add(RDF_TYPE, ConstantRole::Object);
    check(TYPE_OBJ, as_object);
}

#[test]
fn graph_name_is_named_graph_and_never_predicate() {
    let mut want = Want::new();
    want.add(G, ConstantRole::NamedGraph);
    want.add(P, ConstantRole::Predicate);
    check(GRAPH, want);

    let mut same = Want::new();
    same.add(P, ConstantRole::NamedGraph);
    same.add(P, ConstantRole::Predicate);
    check(GRAPH_SAME, same);

    let mut variable = Want::new();
    variable.add(P, ConstantRole::Predicate);
    check(GRAPH_VAR, variable);
}

#[test]
fn duplicates_and_same_iri_in_different_roles_stay_observable() {
    let mut once = Want::new();
    once.add(A, ConstantRole::Subject);
    once.add(A, ConstantRole::Object);
    once.add(P, ConstantRole::Predicate);
    check(SAME_NODE, once);

    let mut twice = Want::new();
    twice.add(P, ConstantRole::Predicate);
    twice.add(P, ConstantRole::Predicate);
    check(TWO_TRIPLES, twice);
}

#[test]
fn literal_datatypes_are_reported_only_when_explicit() {
    let mut typed = Want::new();
    typed.add(P, ConstantRole::Predicate);
    typed.add(DT, ConstantRole::LiteralDatatype);
    check(LIT_TYPED, typed);

    let mut plain = Want::new();
    plain.add(P, ConstantRole::Predicate);
    plain.add(Q, ConstantRole::Predicate);
    check(LIT_PLAIN, plain);

    let mut filter = Want::new();
    filter.add(DT, ConstantRole::LiteralDatatype);
    check(FILTER_LIT, filter);
}

#[test]
fn expression_order_and_aggregate_constants_are_unresolved() {
    check_unresolved(EXPR_EQ, X);
    check_unresolved(BIND_IRI, W);
    check_unresolved(ORDER_BY, X);
    check_unresolved(AGG_IRI, X);

    let mut list = Want::new();
    list.add(Y, ConstantRole::Unresolved);
    list.add(Z, ConstantRole::Unresolved);
    check(EXPR_IN, list);
}

#[test]
fn exists_and_optional_patterns_are_visited() {
    let mut exists = Want::new();
    exists.add(G, ConstantRole::NamedGraph);
    exists.add(Q, ConstantRole::Predicate);
    check(EXISTS, exists);

    let mut nested = Want::new();
    nested.add(P, ConstantRole::Predicate);
    nested.add(Q, ConstantRole::Predicate);
    nested.add(R, ConstantRole::Object);
    check(NESTED, nested);
}

#[test]
fn values_constants_are_reported_conservatively() {
    let mut want = Want::new();
    want.add(A, ConstantRole::Unresolved);
    want.add(XSD_INTEGER, ConstantRole::LiteralDatatype);
    want.add(DT, ConstantRole::LiteralDatatype);
    check(VALUES, want);
}

#[test]
fn path_predicates_are_visited_and_endpoints_stay_unresolved() {
    check_path(PATH_PLUS, &[P]);
    check_path(PATH_NEG, &[P, Q]);
    check_path(PATH_REV, &[P]);

    let mut alt = Want::new();
    alt.add(A, ConstantRole::Unresolved);
    for iri in [P, Q, R] {
        alt.add(iri, ConstantRole::Predicate);
    }
    check(PATH_ALT, alt);

    let mut typed = Want::new();
    typed.add(C, ConstantRole::Unresolved);
    typed.add(P, ConstantRole::Predicate);
    typed.add(RDF_TYPE, ConstantRole::Predicate);
    check(PATH_TYPE, typed);
}

#[test]
fn mixed_query_reports_each_role_exactly() {
    let mut want = Want::new();
    want.add(G, ConstantRole::NamedGraph);
    want.add(RDF_TYPE, ConstantRole::Predicate);
    want.add(C, ConstantRole::Class);
    want.add(X, ConstantRole::Unresolved);
    check(MIXED, want);
}

#[test]
fn existing_refusals_precede_any_callback() {
    assert_refused(DATASET, ShapeRule::DatasetClause);
    assert_refused(CONSTRUCT_Q, ShapeRule::ConstructForm);
    assert_refused(SERVICE_Q, ShapeRule::ServiceInPattern);
    assert_refused(CUSTOM_Q, ShapeRule::CustomFunctionUnsupported);
}
