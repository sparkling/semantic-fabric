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

use super::error::QueryWireError;

pub(super) enum Value {
    Query(Query),
    Dataset(QueryDataset),
    Graph(GraphPattern),
    Expression(Expression),
    Function(Function),
    Path(PropertyPathExpression),
    Aggregate(AggregateExpression),
    AggregateFunction(AggregateFunction),
    Order(OrderExpression),
    Triple(TriplePattern),
    Term(TermPattern),
    NamedPattern(NamedNodePattern),
    GroundTerm(GroundTerm),
    GroundTriple(GroundTriple),
    NamedNode(NamedNode),
    Variable(Variable),
    BlankNode(BlankNode),
    Literal(Literal),
    BaseIri(Iri<String>),
    ValuesRow(Vec<Option<GroundTerm>>),
    AggregateBinding(Variable, AggregateExpression),
}

pub(super) struct ChildCursor {
    values: std::vec::IntoIter<Option<Value>>,
}

impl ChildCursor {
    pub fn new(values: Vec<Option<Value>>) -> Self {
        Self {
            values: values.into_iter(),
        }
    }

    pub fn finish(mut self) -> Result<(), QueryWireError> {
        if self.values.next().is_some() {
            Err(QueryWireError::InvalidRecord)
        } else {
            Ok(())
        }
    }

    pub fn next_slot(&mut self) -> Result<Option<Value>, QueryWireError> {
        self.values.next().ok_or(QueryWireError::InvalidRecord)
    }

    fn next_value(&mut self) -> Result<Value, QueryWireError> {
        self.next_slot()?.ok_or(QueryWireError::InvalidIndex)
    }
}

macro_rules! typed_child {
    ($name:ident, $variant:ident, $type:ty) => {
        impl ChildCursor {
            pub fn $name(&mut self) -> Result<$type, QueryWireError> {
                match self.next_value()? {
                    Value::$variant(value) => Ok(value),
                    _ => Err(QueryWireError::TypeMismatch),
                }
            }
        }
    };
}

typed_child!(query, Query, Query);
typed_child!(dataset, Dataset, QueryDataset);
typed_child!(graph, Graph, GraphPattern);
typed_child!(expression, Expression, Expression);
typed_child!(function, Function, Function);
typed_child!(path, Path, PropertyPathExpression);
typed_child!(aggregate, Aggregate, AggregateExpression);
typed_child!(aggregate_function, AggregateFunction, AggregateFunction);
typed_child!(order, Order, OrderExpression);
typed_child!(triple, Triple, TriplePattern);
typed_child!(term, Term, TermPattern);
typed_child!(named_pattern, NamedPattern, NamedNodePattern);
typed_child!(ground_term, GroundTerm, GroundTerm);
typed_child!(ground_triple, GroundTriple, GroundTriple);
typed_child!(named_node, NamedNode, NamedNode);
typed_child!(variable, Variable, Variable);
typed_child!(blank_node, BlankNode, BlankNode);
typed_child!(literal, Literal, Literal);
typed_child!(base_iri, BaseIri, Iri<String>);
typed_child!(values_row, ValuesRow, Vec<Option<GroundTerm>>);

impl ChildCursor {
    pub fn aggregate_binding(&mut self) -> Result<(Variable, AggregateExpression), QueryWireError> {
        match self.next_value()? {
            Value::AggregateBinding(variable, aggregate) => Ok((variable, aggregate)),
            _ => Err(QueryWireError::TypeMismatch),
        }
    }
}
