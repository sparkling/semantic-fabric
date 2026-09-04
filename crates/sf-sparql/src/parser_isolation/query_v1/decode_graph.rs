use spargebra::algebra::GraphPattern;

use super::allocation::try_box;
use super::decode_record::usize_from;
use super::decode_value::{ChildCursor, Value};
use super::error::QueryWireError;
use super::model::{tag, Record};

pub(super) fn decode_graph(
    record: Record,
    mut children: ChildCursor,
) -> Result<Value, QueryWireError> {
    let graph = match record.tag {
        tag::GRAPH_BGP => GraphPattern::Bgp {
            patterns: triples(&mut children, usize_from(record.fields[0])?)?,
        },
        tag::GRAPH_PATH => GraphPattern::Path {
            subject: children.term()?,
            path: children.path()?,
            object: children.term()?,
        },
        tag::GRAPH_JOIN => GraphPattern::Join {
            left: try_box(children.graph()?)?,
            right: try_box(children.graph()?)?,
        },
        tag::GRAPH_LEFT_JOIN => {
            let left = try_box(children.graph()?)?;
            let right = try_box(children.graph()?)?;
            let expression = if bool_flag(record.flags)? {
                Some(children.expression()?)
            } else {
                None
            };
            GraphPattern::LeftJoin {
                left,
                right,
                expression,
            }
        }
        tag::GRAPH_LATERAL => GraphPattern::Lateral {
            left: try_box(children.graph()?)?,
            right: try_box(children.graph()?)?,
        },
        tag::GRAPH_FILTER => GraphPattern::Filter {
            expr: children.expression()?,
            inner: try_box(children.graph()?)?,
        },
        tag::GRAPH_UNION => GraphPattern::Union {
            left: try_box(children.graph()?)?,
            right: try_box(children.graph()?)?,
        },
        tag::GRAPH_NAMED => GraphPattern::Graph {
            name: children.named_pattern()?,
            inner: try_box(children.graph()?)?,
        },
        tag::GRAPH_EXTEND => GraphPattern::Extend {
            inner: try_box(children.graph()?)?,
            variable: children.variable()?,
            expression: children.expression()?,
        },
        tag::GRAPH_MINUS => GraphPattern::Minus {
            left: try_box(children.graph()?)?,
            right: try_box(children.graph()?)?,
        },
        tag::GRAPH_VALUES => decode_values(record, &mut children)?,
        tag::GRAPH_ORDER_BY => GraphPattern::OrderBy {
            inner: try_box(children.graph()?)?,
            expression: orders(&mut children, usize_from(record.fields[0])?)?,
        },
        tag::GRAPH_PROJECT => GraphPattern::Project {
            inner: try_box(children.graph()?)?,
            variables: variables(&mut children, usize_from(record.fields[0])?)?,
        },
        tag::GRAPH_DISTINCT => GraphPattern::Distinct {
            inner: try_box(children.graph()?)?,
        },
        tag::GRAPH_REDUCED => GraphPattern::Reduced {
            inner: try_box(children.graph()?)?,
        },
        tag::GRAPH_SLICE => GraphPattern::Slice {
            inner: try_box(children.graph()?)?,
            start: joined_usize(record.fields[0], record.fields[1])?,
            length: if bool_flag(record.flags)? {
                Some(joined_usize(record.fields[2], record.fields[3])?)
            } else {
                None
            },
        },
        tag::GRAPH_GROUP => decode_group(record, &mut children)?,
        tag::GRAPH_SERVICE => GraphPattern::Service {
            name: children.named_pattern()?,
            inner: try_box(children.graph()?)?,
            silent: bool_flag(record.flags)?,
        },
        _ => return Err(QueryWireError::UnknownRecordKind),
    };
    children.finish()?;
    Ok(Value::Graph(graph))
}

fn decode_values(
    record: Record,
    children: &mut ChildCursor,
) -> Result<GraphPattern, QueryWireError> {
    let variable_len = usize_from(record.fields[0])?;
    let binding_len = usize_from(record.fields[1])?;
    let variables = variables(children, variable_len)?;
    let mut bindings = Vec::new();
    bindings.try_reserve_exact(binding_len)?;
    for _ in 0..binding_len {
        bindings.push(children.values_row()?);
    }
    Ok(GraphPattern::Values {
        variables,
        bindings,
    })
}

fn decode_group(
    record: Record,
    children: &mut ChildCursor,
) -> Result<GraphPattern, QueryWireError> {
    let inner = try_box(children.graph()?)?;
    let variables = variables(children, usize_from(record.fields[0])?)?;
    let aggregate_len = usize_from(record.fields[1])?;
    let mut aggregates = Vec::new();
    aggregates.try_reserve_exact(aggregate_len)?;
    for _ in 0..aggregate_len {
        aggregates.push(children.aggregate_binding()?);
    }
    Ok(GraphPattern::Group {
        inner,
        variables,
        aggregates,
    })
}

fn triples(
    children: &mut ChildCursor,
    len: usize,
) -> Result<Vec<spargebra::term::TriplePattern>, QueryWireError> {
    let mut values = Vec::new();
    values.try_reserve_exact(len)?;
    for _ in 0..len {
        values.push(children.triple()?);
    }
    Ok(values)
}

fn variables(
    children: &mut ChildCursor,
    len: usize,
) -> Result<Vec<spargebra::term::Variable>, QueryWireError> {
    let mut values = Vec::new();
    values.try_reserve_exact(len)?;
    for _ in 0..len {
        values.push(children.variable()?);
    }
    Ok(values)
}

fn orders(
    children: &mut ChildCursor,
    len: usize,
) -> Result<Vec<spargebra::algebra::OrderExpression>, QueryWireError> {
    let mut values = Vec::new();
    values.try_reserve_exact(len)?;
    for _ in 0..len {
        values.push(children.order()?);
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

fn joined_usize(high: u32, low: u32) -> Result<usize, QueryWireError> {
    let value = (u64::from(high) << 32) | u64::from(low);
    usize::try_from(value).map_err(|_| QueryWireError::InvalidRecord)
}
