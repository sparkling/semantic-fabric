//! Fixed post-parse structural envelope for staged governed compilation.
//!
//! These provisional V1 limits are intentionally conservative and
//! uncalibrated, and are independent of request/body length. This validator
//! only measures the query-reachable AST after parsing; it does not parse
//! input, cover update-only structures, or make the upstream parser
//! cancellable.

use spargebra::algebra::{
    AggregateExpression, Expression, GraphPattern, OrderExpression, PropertyPathExpression,
    QueryDataset,
};
use spargebra::term::{
    BlankNode, GroundTerm, GroundTriple, Literal, NamedNode, NamedNodePattern, TermPattern,
    TriplePattern, Variable,
};
use spargebra::Query;

use super::{enforce, CompileEnvelopeError, CompileEnvelopeLimit};

mod visit;

// UNCALIBRATED: replace only from checked-in corpus and adversarial evidence.
pub(crate) const MAX_ALGEBRA_NODES_V1: usize = 32 * 1024;
pub(crate) const MAX_ALGEBRA_DEPTH_V1: usize = 128;
pub(crate) const MAX_COLLECTION_SLOTS_V1: usize = 64 * 1024;
pub(crate) const MAX_RETAINED_PAYLOAD_BYTES_V1: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AlgebraEnvelopeV1 {
    pub(crate) algebra_nodes: usize,
    pub(crate) max_depth: usize,
    pub(crate) collection_slots: usize,
    pub(crate) retained_payload_bytes: usize,
}

impl AlgebraEnvelopeV1 {
    /// Measures a parsed query with an iterative, allocation-bounded walk.
    ///
    /// An algebra node is each visited query, dataset, graph-pattern,
    /// expression, path, aggregate, order, triple, term-pattern, ground-term,
    /// or scalar RDF-term/variable value. Collection slots count every direct
    /// `Vec` entry, including VALUES rows and unbound cells. Payload bytes are a
    /// conservative sum of visible retained strings; repeated occurrences and
    /// literal datatype IRIs are deliberately charged again.
    pub(crate) fn validate(query: &Query) -> Result<Self, CompileEnvelopeError> {
        Validator::run(query)
    }
}

#[derive(Clone, Copy)]
struct Frame<'query> {
    depth: usize,
    work: Work<'query>,
}

#[derive(Clone, Copy)]
enum Work<'query> {
    Query(&'query Query),
    Dataset(&'query QueryDataset),
    Graph(&'query GraphPattern),
    Expression(&'query Expression),
    Path(&'query PropertyPathExpression),
    Aggregate(&'query AggregateExpression),
    Order(&'query OrderExpression),
    Triple(&'query TriplePattern),
    Term(&'query TermPattern),
    NamedPattern(&'query NamedNodePattern),
    GroundTerm(&'query GroundTerm),
    GroundTriple(&'query GroundTriple),
    NamedNode(&'query NamedNode),
    Variable(&'query Variable),
    BlankNode(&'query BlankNode),
    Literal(&'query Literal),
}

struct Validator<'query> {
    stack: Vec<Frame<'query>>,
    envelope: AlgebraEnvelopeV1,
}

impl<'query> Validator<'query> {
    fn run(query: &'query Query) -> Result<AlgebraEnvelopeV1, CompileEnvelopeError> {
        let mut validator = Self {
            stack: Vec::new(),
            envelope: AlgebraEnvelopeV1::default(),
        };
        validator.push(1, Work::Query(query))?;
        while let Some(frame) = validator.stack.pop() {
            validator.record_node(frame.depth)?;
            validator.visit(frame)?;
        }
        Ok(validator.envelope)
    }

    fn push(&mut self, depth: usize, work: Work<'query>) -> Result<(), CompileEnvelopeError> {
        enforce(
            CompileEnvelopeLimit::AlgebraDepth,
            depth,
            MAX_ALGEBRA_DEPTH_V1,
        )?;
        let observed = self
            .envelope
            .algebra_nodes
            .checked_add(self.stack.len())
            .and_then(|value| value.checked_add(1))
            .unwrap_or(usize::MAX);
        enforce(
            CompileEnvelopeLimit::AlgebraNodes,
            observed,
            MAX_ALGEBRA_NODES_V1,
        )?;
        if self.stack.len() == self.stack.capacity() {
            self.stack.reserve(1);
        }
        self.stack.push(Frame { depth, work });
        self.envelope.max_depth = self.envelope.max_depth.max(depth);
        Ok(())
    }

    fn record_node(&mut self, depth: usize) -> Result<(), CompileEnvelopeError> {
        let observed = saturated_sum(self.envelope.algebra_nodes, 1);
        enforce(
            CompileEnvelopeLimit::AlgebraNodes,
            observed,
            MAX_ALGEBRA_NODES_V1,
        )?;
        self.envelope.algebra_nodes = observed;
        self.envelope.max_depth = self.envelope.max_depth.max(depth);
        Ok(())
    }

    fn collection(&mut self, slots: usize) -> Result<(), CompileEnvelopeError> {
        let observed = saturated_sum(self.envelope.collection_slots, slots);
        enforce(
            CompileEnvelopeLimit::CollectionSlots,
            observed,
            MAX_COLLECTION_SLOTS_V1,
        )?;
        self.envelope.collection_slots = observed;
        Ok(())
    }

    fn payload(&mut self, bytes: usize) -> Result<(), CompileEnvelopeError> {
        let observed = saturated_sum(self.envelope.retained_payload_bytes, bytes);
        enforce(
            CompileEnvelopeLimit::RetainedPayloadBytes,
            observed,
            MAX_RETAINED_PAYLOAD_BYTES_V1,
        )?;
        self.envelope.retained_payload_bytes = observed;
        Ok(())
    }
}

fn saturated_sum(left: usize, right: usize) -> usize {
    left.saturating_add(right)
}

#[cfg(test)]
mod tests;
