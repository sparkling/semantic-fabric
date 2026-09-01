//! CONSTRUCT-template instantiation, including fresh per-solution blank nodes.

/// Instantiate a CONSTRUCT-template triple against a solution; `None` if any
/// variable is unbound or the triple would be ill-formed (SPARQL §16.2: an
/// ill-formed instantiation is silently dropped, never an error). `pub(crate)`
/// so the PostgreSQL executor instantiates CONSTRUCT templates identically.
/// `solution_id` is a monotonic counter identifying the CURRENT solution
/// within this one CONSTRUCT execution — see [`instantiate_term`]'s
/// `TermPattern::BlankNode` arm for why it must be threaded all the way
/// through: every triple pattern instantiated for the SAME solution is called
/// with the SAME `solution_id`.
pub(crate) fn instantiate(
    tp: &spargebra::term::TriplePattern,
    bindings: &Bindings,
    solution_id: u64,
) -> Option<Triple> {
    use spargebra::term::NamedNodePattern;
    let subject = instantiate_term(&tp.subject, bindings, solution_id)?;
    let predicate = match &tp.predicate {
        NamedNodePattern::NamedNode(n) => Term::NamedNode(n.clone()),
        NamedNodePattern::Variable(v) => bindings.get(v.as_str()).cloned()?,
    };
    let object = instantiate_term(&tp.object, bindings, solution_id)?;
    Triple::from_terms(subject, predicate, object).ok()
}

/// A CONSTRUCT-template term slot → its bound `Term`, or `None` if unbound /
/// ill-formed. `TermPattern::Triple` (ADR-0032 D2) recurses — a nested quoted
/// triple in a template (`star::substitute_construct_template` is the ONLY
/// producer of this shape in a template today, but the arm is general) builds
/// its own s/p/o first, bottom-up, then composes via `Triple::from_terms`,
/// whose fallibility naturally enforces RDF 1.2 §3.1 position legality — an
/// illegal-position nested triple silently drops (§16.2), never errors. A
/// standalone (non-closure) function so it can recurse into itself.
///
/// `TermPattern::BlankNode` (Run 5 W6 fix): SPARQL §16.2 requires a template
/// blank node to denote a FRESH node per solution ("blank nodes created from
/// the same label in different solutions will be different") while the SAME
/// label used across MULTIPLE triples of the SAME solution's instantiation
/// must resolve to the SAME node ("blank nodes... within a single copy of the
/// template shall not be split") — this arm used to clone the parsed-AST
/// label verbatim (`Some(Term::BlankNode(b.clone()))`), so every solution
/// shared ONE identity, never freshened. Now it derives a fresh label from
/// the AST label plus `solution_id`: deterministic (so repeated triples of
/// ONE solution's instantiation agree) and distinct across solutions (the
/// caller advances `solution_id` once per solution, never once per triple).
fn instantiate_term(
    p: &spargebra::term::TermPattern,
    bindings: &Bindings,
    solution_id: u64,
) -> Option<Term> {
    use spargebra::term::TermPattern;
    match p {
        TermPattern::Variable(v) => bindings.get(v.as_str()).cloned(),
        TermPattern::NamedNode(n) => Some(Term::NamedNode(n.clone())),
        TermPattern::Literal(l) => Some(Term::Literal(l.clone())),
        TermPattern::BlankNode(b) => {
            // A dedicated, versioned domain keeps template-minted nodes disjoint
            // from R2RML-generated nodes even for adversarially chosen labels.
            let mut label = String::from("sfc1_");
            super::push_hex(&mut label, b.as_str().as_bytes());
            label.push('_');
            use std::fmt::Write as _;
            write!(&mut label, "{solution_id:016x}").expect("write to String");
            Some(Term::BlankNode(sf_core::BlankNode::new_unchecked(label)))
        }
        TermPattern::Triple(inner) => {
            let s = instantiate_term(&inner.subject, bindings, solution_id)?;
            let p = match &inner.predicate {
                spargebra::term::NamedNodePattern::NamedNode(n) => Term::NamedNode(n.clone()),
                spargebra::term::NamedNodePattern::Variable(v) => {
                    bindings.get(v.as_str()).cloned()?
                }
            };
            let o = instantiate_term(&inner.object, bindings, solution_id)?;
            Triple::from_terms(s, p, o).ok().map(Term::from)
        }
    }
}
use sf_core::{Term, Triple};

use super::row::Bindings;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construct_blank_nodes_use_a_fresh_disjoint_label_domain() {
        let pattern = spargebra::term::TermPattern::BlankNode(
            spargebra::term::BlankNode::new_unchecked("sfr1d_736861726564"),
        );
        let bindings = Bindings::new();
        let first = instantiate_term(&pattern, &bindings, 7).unwrap();
        let same_solution = instantiate_term(&pattern, &bindings, 7).unwrap();
        let next_solution = instantiate_term(&pattern, &bindings, 8).unwrap();

        assert_eq!(first, same_solution);
        assert_ne!(first, next_solution);
        assert!(matches!(first, Term::BlankNode(ref b) if b.as_str().starts_with("sfc1_")));
    }
}
