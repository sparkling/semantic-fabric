use oxiri::Iri;
use oxrdf::BaseDirection;
use spargebra::algebra::{AggregateExpression, OrderExpression, QueryDataset};
use spargebra::term::{
    BlankNode, GroundTerm, GroundTriple, Literal, NamedNode, NamedNodePattern, TermPattern,
    TriplePattern, Variable,
};
use spargebra::Query;

use super::decode_expression::decode_expression;
use super::decode_graph::decode_graph;
use super::decode_term::decode_path;
use super::decode_value::{ChildCursor, Value};
use super::error::QueryWireError;
use super::function::{decode_aggregate_function, decode_function};
use super::model::{tag, Record};

pub(super) fn decode_record(
    record: Record,
    children: Vec<Option<Value>>,
    scalar_bytes: &[u8],
) -> Result<Value, QueryWireError> {
    let cursor = ChildCursor::new(children);
    match record.tag {
        tag::QUERY_SELECT | tag::QUERY_CONSTRUCT | tag::QUERY_DESCRIBE | tag::QUERY_ASK => {
            decode_query(record, cursor)
        }
        tag::DATASET => decode_dataset(record, cursor),
        tag::GRAPH_BGP..=tag::GRAPH_SERVICE => decode_graph(record, cursor),
        tag::EXPR_NAMED_NODE..=tag::EXPR_FUNCTION_CALL => decode_expression(record, cursor),
        tag::FUNCTION => decode_function_record(record, cursor),
        tag::PATH_NAMED_NODE..=tag::PATH_NEGATED_SET => decode_path(record, cursor),
        tag::AGGREGATE_COUNT_SOLUTIONS | tag::AGGREGATE_FUNCTION_CALL => {
            decode_aggregate(record, cursor)
        }
        tag::AGGREGATE_FUNCTION => decode_aggregate_function_record(record, cursor, scalar_bytes),
        tag::ORDER_ASC | tag::ORDER_DESC => decode_order(record, cursor),
        tag::TRIPLE_PATTERN..=tag::GROUND_TRIPLE => decode_term_record(record, cursor),
        tag::NAMED_NODE | tag::VARIABLE | tag::BLANK_NODE | tag::LITERAL | tag::BASE_IRI => {
            decode_scalar_record(record, cursor, scalar_bytes)
        }
        tag::VALUES_ROW => decode_values_row(record, cursor),
        tag::AGGREGATE_BINDING => decode_aggregate_binding(record, cursor),
        _ => Err(QueryWireError::UnknownRecordKind),
    }
}

fn decode_query(record: Record, mut children: ChildCursor) -> Result<Value, QueryWireError> {
    if record.flags & !0b11 != 0 {
        return Err(QueryWireError::InvalidRecord);
    }
    let template_len = usize_from(record.fields[0])?;
    if record.tag != tag::QUERY_CONSTRUCT && template_len != 0 {
        return Err(QueryWireError::InvalidRecord);
    }
    let mut template = Vec::new();
    template.try_reserve_exact(template_len)?;
    for _ in 0..template_len {
        template.push(children.triple()?);
    }
    let dataset = if record.flags & 1 != 0 {
        Some(children.dataset()?)
    } else {
        None
    };
    let pattern = children.graph()?;
    let base_iri = if record.flags & 2 != 0 {
        Some(children.base_iri()?)
    } else {
        None
    };
    children.finish()?;
    Ok(Value::Query(match record.tag {
        tag::QUERY_SELECT => Query::Select {
            dataset,
            pattern,
            base_iri,
        },
        tag::QUERY_CONSTRUCT => Query::Construct {
            template,
            dataset,
            pattern,
            base_iri,
        },
        tag::QUERY_DESCRIBE => Query::Describe {
            dataset,
            pattern,
            base_iri,
        },
        tag::QUERY_ASK => Query::Ask {
            dataset,
            pattern,
            base_iri,
        },
        _ => return Err(QueryWireError::UnknownRecordKind),
    }))
}

fn decode_dataset(record: Record, mut children: ChildCursor) -> Result<Value, QueryWireError> {
    let named_present = bool_flag(record.flags)?;
    let default_len = usize_from(record.fields[0])?;
    let named_len = usize_from(record.fields[1])?;
    if !named_present && named_len != 0 {
        return Err(QueryWireError::InvalidRecord);
    }
    let default = named_nodes(&mut children, default_len)?;
    let named = if named_present {
        Some(named_nodes(&mut children, named_len)?)
    } else {
        None
    };
    children.finish()?;
    Ok(Value::Dataset(QueryDataset { default, named }))
}

fn decode_function_record(
    record: Record,
    mut children: ChildCursor,
) -> Result<Value, QueryWireError> {
    let custom = if record.fields[0] == 0 {
        Some(children.named_node()?)
    } else {
        None
    };
    let function = decode_function(record.fields[0], custom)?;
    children.finish()?;
    Ok(Value::Function(function))
}

fn decode_aggregate(record: Record, mut children: ChildCursor) -> Result<Value, QueryWireError> {
    let distinct = bool_flag(record.flags)?;
    let aggregate = match record.tag {
        tag::AGGREGATE_COUNT_SOLUTIONS => AggregateExpression::CountSolutions { distinct },
        tag::AGGREGATE_FUNCTION_CALL => AggregateExpression::FunctionCall {
            name: children.aggregate_function()?,
            expr: children.expression()?,
            distinct,
        },
        _ => return Err(QueryWireError::UnknownRecordKind),
    };
    children.finish()?;
    Ok(Value::Aggregate(aggregate))
}

fn decode_aggregate_function_record(
    record: Record,
    mut children: ChildCursor,
    scalar_bytes: &[u8],
) -> Result<Value, QueryWireError> {
    let separator_present = bool_flag(record.flags)?;
    let separator = if separator_present {
        Some(copy_scalar(
            scalar_bytes,
            record.fields[1],
            record.fields[2],
        )?)
    } else {
        None
    };
    let custom = if record.fields[0] == 0 {
        Some(children.named_node()?)
    } else {
        None
    };
    let function =
        decode_aggregate_function(record.fields[0], separator_present, separator, custom)?;
    children.finish()?;
    Ok(Value::AggregateFunction(function))
}

fn decode_order(record: Record, mut children: ChildCursor) -> Result<Value, QueryWireError> {
    let expression = children.expression()?;
    children.finish()?;
    Ok(Value::Order(match record.tag {
        tag::ORDER_ASC => OrderExpression::Asc(expression),
        tag::ORDER_DESC => OrderExpression::Desc(expression),
        _ => return Err(QueryWireError::UnknownRecordKind),
    }))
}

fn decode_term_record(record: Record, mut children: ChildCursor) -> Result<Value, QueryWireError> {
    let value = match record.tag {
        tag::TRIPLE_PATTERN => Value::Triple(TriplePattern {
            subject: children.term()?,
            predicate: children.named_pattern()?,
            object: children.term()?,
        }),
        tag::TERM_NAMED_NODE => Value::Term(TermPattern::NamedNode(children.named_node()?)),
        tag::TERM_BLANK_NODE => Value::Term(TermPattern::BlankNode(children.blank_node()?)),
        tag::TERM_LITERAL => Value::Term(TermPattern::Literal(children.literal()?)),
        tag::TERM_TRIPLE => Value::Term(TermPattern::Triple(super::allocation::try_box(
            children.triple()?,
        )?)),
        tag::TERM_VARIABLE => Value::Term(TermPattern::Variable(children.variable()?)),
        tag::NAMED_PATTERN_NODE => {
            Value::NamedPattern(NamedNodePattern::NamedNode(children.named_node()?))
        }
        tag::NAMED_PATTERN_VARIABLE => {
            Value::NamedPattern(NamedNodePattern::Variable(children.variable()?))
        }
        tag::GROUND_TERM_NAMED_NODE => {
            Value::GroundTerm(GroundTerm::NamedNode(children.named_node()?))
        }
        tag::GROUND_TERM_LITERAL => Value::GroundTerm(GroundTerm::Literal(children.literal()?)),
        tag::GROUND_TERM_TRIPLE => Value::GroundTerm(GroundTerm::Triple(
            super::allocation::try_box(children.ground_triple()?)?,
        )),
        tag::GROUND_TRIPLE => Value::GroundTriple(GroundTriple {
            subject: children.named_node()?,
            predicate: children.named_node()?,
            object: children.ground_term()?,
        }),
        _ => return Err(QueryWireError::UnknownRecordKind),
    };
    children.finish()?;
    Ok(value)
}

fn decode_scalar_record(
    record: Record,
    children: ChildCursor,
    scalar_bytes: &[u8],
) -> Result<Value, QueryWireError> {
    children.finish()?;
    let value = copy_scalar(scalar_bytes, record.fields[0], record.fields[1])?;
    Ok(match record.tag {
        tag::NAMED_NODE => {
            Value::NamedNode(NamedNode::new(value).map_err(|_| QueryWireError::InvalidScalar)?)
        }
        tag::VARIABLE => {
            Value::Variable(Variable::new(value).map_err(|_| QueryWireError::InvalidScalar)?)
        }
        tag::BLANK_NODE => {
            Value::BlankNode(BlankNode::new(value).map_err(|_| QueryWireError::InvalidScalar)?)
        }
        tag::LITERAL => Value::Literal(decode_literal(record, value, scalar_bytes)?),
        tag::BASE_IRI => {
            Value::BaseIri(Iri::parse(value).map_err(|_| QueryWireError::InvalidScalar)?)
        }
        _ => return Err(QueryWireError::UnknownRecordKind),
    })
}

fn decode_literal(
    record: Record,
    value: String,
    scalar_bytes: &[u8],
) -> Result<Literal, QueryWireError> {
    let auxiliary = || copy_scalar(scalar_bytes, record.fields[2], record.fields[3]);
    match record.fields[4] {
        0 => Ok(Literal::new_simple_literal(value)),
        1 => Ok(Literal::new_typed_literal(
            value,
            NamedNode::new(auxiliary()?).map_err(|_| QueryWireError::InvalidScalar)?,
        )),
        2 => Literal::new_language_tagged_literal(value, auxiliary()?)
            .map_err(|_| QueryWireError::InvalidScalar),
        3 => Literal::new_directional_language_tagged_literal(
            value,
            auxiliary()?,
            BaseDirection::Ltr,
        )
        .map_err(|_| QueryWireError::InvalidScalar),
        4 => Literal::new_directional_language_tagged_literal(
            value,
            auxiliary()?,
            BaseDirection::Rtl,
        )
        .map_err(|_| QueryWireError::InvalidScalar),
        _ => Err(QueryWireError::InvalidScalar),
    }
}

fn decode_values_row(record: Record, mut children: ChildCursor) -> Result<Value, QueryWireError> {
    let len = usize_from(record.fields[0])?;
    let mut row = Vec::new();
    row.try_reserve_exact(len)?;
    for _ in 0..len {
        row.push(match children.next_slot()? {
            None => None,
            Some(Value::GroundTerm(term)) => Some(term),
            Some(_) => return Err(QueryWireError::TypeMismatch),
        });
    }
    children.finish()?;
    Ok(Value::ValuesRow(row))
}

fn decode_aggregate_binding(
    _record: Record,
    mut children: ChildCursor,
) -> Result<Value, QueryWireError> {
    let variable = children.variable()?;
    let aggregate = children.aggregate()?;
    children.finish()?;
    Ok(Value::AggregateBinding(variable, aggregate))
}

fn named_nodes(children: &mut ChildCursor, len: usize) -> Result<Vec<NamedNode>, QueryWireError> {
    let mut values = Vec::new();
    values.try_reserve_exact(len)?;
    for _ in 0..len {
        values.push(children.named_node()?);
    }
    Ok(values)
}

fn bool_flag(value: u16) -> Result<bool, QueryWireError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(QueryWireError::InvalidRecord),
    }
}

pub(super) fn usize_from(value: u32) -> Result<usize, QueryWireError> {
    usize::try_from(value).map_err(|_| QueryWireError::AccountingOverflow)
}

pub(super) fn copy_scalar(
    scalar_bytes: &[u8],
    offset: u32,
    len: u32,
) -> Result<String, QueryWireError> {
    let offset = usize_from(offset)?;
    let len = usize_from(len)?;
    let end = offset
        .checked_add(len)
        .ok_or(QueryWireError::AccountingOverflow)?;
    let value = std::str::from_utf8(
        scalar_bytes
            .get(offset..end)
            .ok_or(QueryWireError::InvalidScalar)?,
    )
    .map_err(|_| QueryWireError::InvalidScalar)?;
    let mut owned = String::new();
    owned.try_reserve_exact(value.len())?;
    owned.push_str(value);
    Ok(owned)
}
