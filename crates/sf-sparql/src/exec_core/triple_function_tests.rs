//! `eval_expr`/`eval_function`'s Subject/Predicate/Object/IsTriple/Triple
//! arms, operating on an already-materialized `Term::Triple` — the
//! defense-in-depth fallback for whatever `star::rewrite_expr` did not
//! resolve statically (ORDER BY / post-GROUP-BY expression evaluation
//! only; see those arms' own doc comment for the full None-discipline).
use std::sync::Arc;

use sf_core::{Literal, NamedNode, Term, Triple};
use spargebra::algebra::{Expression, Function};
use spargebra::term::Variable;

use super::expression::{bool_literal, eval_expr};
use super::row::Bindings;

fn iri(s: &str) -> Term {
    Term::NamedNode(NamedNode::new_unchecked(s))
}
fn triple_term(s: &str, p: &str, o: Term) -> Term {
    Term::Triple(Box::new(Triple::new(
        NamedNode::new_unchecked(s),
        NamedNode::new_unchecked(p),
        o,
    )))
}
fn bindings_with(var: &str, t: Term) -> Bindings {
    let mut b = Bindings::new();
    b.insert(Arc::from(var), t);
    b
}
fn call(f: Function, var: &str) -> Expression {
    Expression::FunctionCall(f, vec![Expression::Variable(Variable::new(var).unwrap())])
}

#[test]
fn subject_predicate_object_extract_components_from_a_materialized_triple() {
    let t = triple_term("http://ex/s", "http://ex/p", iri("http://ex/o"));
    let b = bindings_with("t", t);
    assert_eq!(
        eval_expr(&call(Function::Subject, "t"), &b),
        Some(iri("http://ex/s"))
    );
    assert_eq!(
        eval_expr(&call(Function::Predicate, "t"), &b),
        Some(iri("http://ex/p"))
    );
    assert_eq!(
        eval_expr(&call(Function::Object, "t"), &b),
        Some(iri("http://ex/o"))
    );
}

#[test]
fn subject_predicate_object_error_on_a_non_triple_is_none() {
    let b = bindings_with("t", iri("http://ex/plain"));
    assert_eq!(eval_expr(&call(Function::Subject, "t"), &b), None);
    assert_eq!(eval_expr(&call(Function::Predicate, "t"), &b), None);
    assert_eq!(eval_expr(&call(Function::Object, "t"), &b), None);
}

#[test]
fn is_triple_never_errors_true_and_false_cases() {
    let triple_b = bindings_with(
        "t",
        triple_term("http://ex/s", "http://ex/p", iri("http://ex/o")),
    );
    let plain_b = bindings_with("t", iri("http://ex/plain"));
    let true_lit = eval_expr(&call(Function::IsTriple, "t"), &triple_b);
    let false_lit = eval_expr(&call(Function::IsTriple, "t"), &plain_b);
    assert_eq!(true_lit, bool_literal(true));
    assert_eq!(false_lit, bool_literal(false));
}

#[test]
fn triple_function_composes_legal_components_and_drops_illegal_ones() {
    let e = Expression::FunctionCall(
        Function::Triple,
        vec![
            Expression::NamedNode(NamedNode::new_unchecked("http://ex/s")),
            Expression::NamedNode(NamedNode::new_unchecked("http://ex/p")),
            Expression::NamedNode(NamedNode::new_unchecked("http://ex/o")),
        ],
    );
    let b = Bindings::new();
    assert_eq!(
        eval_expr(&e, &b),
        Some(triple_term(
            "http://ex/s",
            "http://ex/p",
            iri("http://ex/o")
        ))
    );

    // A literal in SUBJECT position is illegal (RDF 1.2 §3.1: subject
    // must be IRI/bnode) — `Triple::from_terms` rejects it, so the whole
    // call is `None` (never a malformed Term::Triple).
    let illegal = Expression::FunctionCall(
        Function::Triple,
        vec![
            Expression::Literal(Literal::new_simple_literal("not-a-subject")),
            Expression::NamedNode(NamedNode::new_unchecked("http://ex/p")),
            Expression::NamedNode(NamedNode::new_unchecked("http://ex/o")),
        ],
    );
    assert_eq!(eval_expr(&illegal, &b), None);
}
