use spargebra::algebra::GraphPattern;
use spargebra::term::{NamedNode, Variable};
use spargebra::Query;

use super::*;
use crate::compile_envelope::CompileEnvelopeLimit;

mod bounds;
mod repository;
mod variants;

fn iri(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("https://example.test/{local}"))
}

fn var(name: &str) -> Variable {
    Variable::new_unchecked(name)
}

fn empty() -> GraphPattern {
    GraphPattern::Bgp {
        patterns: Vec::new(),
    }
}

fn select(pattern: GraphPattern) -> Query {
    Query::Select {
        dataset: None,
        pattern,
        base_iri: None,
    }
}

fn assert_limit(
    error: CompileEnvelopeError,
    dimension: CompileEnvelopeLimit,
    observed: usize,
    maximum: usize,
) {
    assert_eq!(
        error,
        CompileEnvelopeError::LimitExceeded {
            dimension,
            observed,
            maximum,
        }
    );
}
