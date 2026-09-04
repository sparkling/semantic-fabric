use oxiri::Iri;
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, Function, GraphPattern, OrderExpression,
    PropertyPathExpression, QueryDataset,
};
use spargebra::term::{
    BlankNode, GroundTerm, GroundTriple, Literal, NamedNode, NamedNodePattern, TermPattern,
    TriplePattern, Variable,
};
use spargebra::Query;

use super::binary::encode_arena;
use super::encode_children::populate_children;
use super::encode_record::append_record;
use super::error::QueryWireError;
use super::model::Arena;
use crate::compile_envelope::algebra::AlgebraEnvelopeV1;

#[derive(Clone, Copy)]
pub(super) enum NodeRef<'query> {
    Query(&'query Query),
    Dataset(&'query QueryDataset),
    Graph(&'query GraphPattern),
    Expression(&'query Expression),
    Function(&'query Function),
    Path(&'query PropertyPathExpression),
    Aggregate(&'query AggregateExpression),
    AggregateFunction(&'query AggregateFunction),
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
    BaseIri(&'query Iri<String>),
    ValuesRow(&'query [Option<GroundTerm>]),
    AggregateBinding(&'query Variable, &'query AggregateExpression),
}

enum Frame<'query> {
    Enter(NodeRef<'query>),
    Exit {
        node: NodeRef<'query>,
        child_count: usize,
    },
}

pub(crate) fn encode(query: &Query) -> Result<Vec<u8>, QueryWireError> {
    AlgebraEnvelopeV1::validate(query).map_err(QueryWireError::from)?;
    let arena = build_arena(query)?;
    encode_arena(&arena)
}

fn build_arena(query: &Query) -> Result<Arena, QueryWireError> {
    let mut arena = Arena {
        records: Vec::new(),
        edges: Vec::new(),
        scalar_bytes: Vec::new(),
    };
    let mut frames = Vec::new();
    let mut completed = Vec::new();
    let mut children = Vec::new();
    frames.try_reserve(1)?;
    frames.push(Frame::Enter(NodeRef::Query(query)));

    while let Some(frame) = frames.pop() {
        match frame {
            Frame::Enter(node) => {
                populate_children(node, &mut children)?;
                let child_count = children.len();
                frames.try_reserve(
                    child_count
                        .checked_add(1)
                        .ok_or(QueryWireError::AccountingOverflow)?,
                )?;
                frames.push(Frame::Exit { node, child_count });
                frames.extend(children.drain(..).rev().map(Frame::Enter));
            }
            Frame::Exit { node, child_count } => {
                let split = completed
                    .len()
                    .checked_sub(child_count)
                    .ok_or(QueryWireError::InvalidRecord)?;
                let index = append_record(&mut arena, node, &completed[split..])?;
                completed.truncate(split);
                completed.try_reserve(1)?;
                completed.push(index);
            }
        }
    }

    if completed.len() != 1 || completed[0] as usize != arena.records.len().saturating_sub(1) {
        return Err(QueryWireError::InvalidRecord);
    }
    Ok(arena)
}
