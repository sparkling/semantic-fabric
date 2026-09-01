use sf_core::NamedNode;
use spargebra::algebra::GraphPattern;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern, Variable};

use crate::iq::node::Var;

/// Parse a query and return its top-level WHERE graph pattern.
pub(super) fn pattern(q: &str) -> GraphPattern {
    match spargebra::SparqlParser::new().parse_query(q).unwrap() {
        spargebra::Query::Select { pattern, .. } => pattern,
        other => panic!("expected SELECT, got {other:?}"),
    }
}

pub(super) fn var(v: &str) -> Variable {
    Variable::new(v).unwrap()
}

pub(super) fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

pub(super) fn triple(s: &str, p: &str, o: &str) -> TriplePattern {
    TriplePattern {
        subject: TermPattern::Variable(var(s)),
        predicate: NamedNodePattern::Variable(var(p)),
        object: TermPattern::Variable(var(o)),
    }
}

pub(super) fn bgp(tps: Vec<TriplePattern>) -> GraphPattern {
    GraphPattern::Bgp { patterns: tps }
}

/// `Vec<Var>` from a slice of names (test ergonomics).
pub(super) fn vec_var(names: &[&str]) -> Vec<Var> {
    names.iter().map(|s| (*s).into()).collect()
}
