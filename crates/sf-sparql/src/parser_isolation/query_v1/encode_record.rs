use oxrdf::BaseDirection;
use spargebra::algebra::{
    AggregateExpression, GraphPattern, OrderExpression, PropertyPathExpression,
};
use spargebra::term::{GroundTerm, NamedNodePattern, TermPattern};
use spargebra::Query;

use super::binary::usize_to_u32;
use super::encode::NodeRef;
use super::error::{QueryWireError, QueryWireLimit};
use super::function::{encode_aggregate_function, encode_function, AggregateFunctionRef};
use super::model::{
    enforce_limit, tag, Arena, Record, MAX_EDGES_V1, MAX_RECORDS_V1, MAX_SCALAR_BYTES_V1, NO_INDEX,
};
use super::schema::validate_scalar_values;

pub(super) fn append_record(
    arena: &mut Arena,
    node: NodeRef<'_>,
    children: &[u32],
) -> Result<u32, QueryWireError> {
    enforce_limit(
        arena
            .records
            .len()
            .checked_add(1)
            .ok_or(QueryWireError::AccountingOverflow)?,
        MAX_RECORDS_V1,
        QueryWireLimit::Records,
    )?;
    let mut edges = Vec::new();
    if let NodeRef::ValuesRow(row) = node {
        edges.try_reserve_exact(row.len())?;
        let mut present = children.iter().copied();
        for value in row {
            edges.push(if value.is_some() {
                present.next().ok_or(QueryWireError::InvalidRecord)?
            } else {
                NO_INDEX
            });
        }
        if present.next().is_some() {
            return Err(QueryWireError::InvalidRecord);
        }
    } else {
        edges.try_reserve_exact(children.len())?;
        edges.extend_from_slice(children);
    }

    let mut record = record_for(arena, node, edges.len())?;
    validate_scalar_values(record, &arena.scalar_bytes)?;
    validate_child_count(node, children.len(), edges.len())?;
    let edge_start = arena.edges.len();
    let new_edge_count = edge_start
        .checked_add(edges.len())
        .ok_or(QueryWireError::AccountingOverflow)?;
    enforce_limit(new_edge_count, MAX_EDGES_V1, QueryWireLimit::Edges)?;
    arena.edges.try_reserve_exact(edges.len())?;
    arena.edges.extend_from_slice(&edges);
    record.edge_start = usize_to_u32(edge_start)?;
    record.edge_len = usize_to_u32(edges.len())?;
    let index = usize_to_u32(arena.records.len())?;
    arena.records.try_reserve(1)?;
    arena.records.push(record);
    Ok(index)
}

fn record_for(
    arena: &mut Arena,
    node: NodeRef<'_>,
    edge_len: usize,
) -> Result<Record, QueryWireError> {
    let zero = [0; 5];
    Ok(match node {
        NodeRef::Query(query) => query_record(query)?,
        NodeRef::Dataset(dataset) => Record::new(
            tag::DATASET,
            u16::from(dataset.named.is_some()),
            [
                usize_to_u32(dataset.default.len())?,
                usize_to_u32(dataset.named.as_ref().map_or(0, Vec::len))?,
                0,
                0,
                0,
            ],
        ),
        NodeRef::Graph(graph) => graph_record(graph)?,
        NodeRef::Expression(expression) => expression_record(expression)?,
        NodeRef::Function(function) => {
            let (code, _) = encode_function(function);
            Record::new(tag::FUNCTION, 0, [code, 0, 0, 0, 0])
        }
        NodeRef::Path(path) => path_record(path)?,
        NodeRef::Aggregate(aggregate) => match aggregate {
            AggregateExpression::CountSolutions { distinct } => {
                Record::new(tag::AGGREGATE_COUNT_SOLUTIONS, u16::from(*distinct), zero)
            }
            AggregateExpression::FunctionCall { distinct, .. } => {
                Record::new(tag::AGGREGATE_FUNCTION_CALL, u16::from(*distinct), zero)
            }
        },
        NodeRef::AggregateFunction(function) => aggregate_function_record(arena, function)?,
        NodeRef::Order(order) => Record::new(
            match order {
                OrderExpression::Asc(_) => tag::ORDER_ASC,
                OrderExpression::Desc(_) => tag::ORDER_DESC,
            },
            0,
            zero,
        ),
        NodeRef::Triple(_) => Record::new(tag::TRIPLE_PATTERN, 0, zero),
        NodeRef::Term(term) => Record::new(
            match term {
                TermPattern::NamedNode(_) => tag::TERM_NAMED_NODE,
                TermPattern::BlankNode(_) => tag::TERM_BLANK_NODE,
                TermPattern::Literal(_) => tag::TERM_LITERAL,
                TermPattern::Triple(_) => tag::TERM_TRIPLE,
                TermPattern::Variable(_) => tag::TERM_VARIABLE,
            },
            0,
            zero,
        ),
        NodeRef::NamedPattern(pattern) => Record::new(
            match pattern {
                NamedNodePattern::NamedNode(_) => tag::NAMED_PATTERN_NODE,
                NamedNodePattern::Variable(_) => tag::NAMED_PATTERN_VARIABLE,
            },
            0,
            zero,
        ),
        NodeRef::GroundTerm(term) => Record::new(
            match term {
                GroundTerm::NamedNode(_) => tag::GROUND_TERM_NAMED_NODE,
                GroundTerm::Literal(_) => tag::GROUND_TERM_LITERAL,
                GroundTerm::Triple(_) => tag::GROUND_TERM_TRIPLE,
            },
            0,
            zero,
        ),
        NodeRef::GroundTriple(_) => Record::new(tag::GROUND_TRIPLE, 0, zero),
        NodeRef::NamedNode(node) => scalar_record(arena, tag::NAMED_NODE, node.as_str())?,
        NodeRef::Variable(variable) => scalar_record(arena, tag::VARIABLE, variable.as_str())?,
        NodeRef::BlankNode(node) => scalar_record(arena, tag::BLANK_NODE, node.as_str())?,
        NodeRef::Literal(literal) => literal_record(arena, literal)?,
        NodeRef::BaseIri(iri) => scalar_record(arena, tag::BASE_IRI, iri.as_str())?,
        NodeRef::ValuesRow(_) => {
            Record::new(tag::VALUES_ROW, 0, [usize_to_u32(edge_len)?, 0, 0, 0, 0])
        }
        NodeRef::AggregateBinding(_, _) => Record::new(tag::AGGREGATE_BINDING, 0, zero),
    })
}

fn query_record(query: &Query) -> Result<Record, QueryWireError> {
    let (tag, template_len, dataset, base) = match query {
        Query::Select {
            dataset, base_iri, ..
        } => (tag::QUERY_SELECT, 0, dataset.is_some(), base_iri.is_some()),
        Query::Construct {
            template,
            dataset,
            base_iri,
            ..
        } => (
            tag::QUERY_CONSTRUCT,
            template.len(),
            dataset.is_some(),
            base_iri.is_some(),
        ),
        Query::Describe {
            dataset, base_iri, ..
        } => (
            tag::QUERY_DESCRIBE,
            0,
            dataset.is_some(),
            base_iri.is_some(),
        ),
        Query::Ask {
            dataset, base_iri, ..
        } => (tag::QUERY_ASK, 0, dataset.is_some(), base_iri.is_some()),
    };
    Ok(Record::new(
        tag,
        u16::from(dataset) | (u16::from(base) << 1),
        [usize_to_u32(template_len)?, 0, 0, 0, 0],
    ))
}

fn graph_record(graph: &GraphPattern) -> Result<Record, QueryWireError> {
    let zero = [0; 5];
    Ok(match graph {
        GraphPattern::Bgp { patterns } => Record::new(
            tag::GRAPH_BGP,
            0,
            [usize_to_u32(patterns.len())?, 0, 0, 0, 0],
        ),
        GraphPattern::Path { .. } => Record::new(tag::GRAPH_PATH, 0, zero),
        GraphPattern::Join { .. } => Record::new(tag::GRAPH_JOIN, 0, zero),
        GraphPattern::LeftJoin { expression, .. } => {
            Record::new(tag::GRAPH_LEFT_JOIN, u16::from(expression.is_some()), zero)
        }
        GraphPattern::Lateral { .. } => Record::new(tag::GRAPH_LATERAL, 0, zero),
        GraphPattern::Filter { .. } => Record::new(tag::GRAPH_FILTER, 0, zero),
        GraphPattern::Union { .. } => Record::new(tag::GRAPH_UNION, 0, zero),
        GraphPattern::Graph { .. } => Record::new(tag::GRAPH_NAMED, 0, zero),
        GraphPattern::Extend { .. } => Record::new(tag::GRAPH_EXTEND, 0, zero),
        GraphPattern::Minus { .. } => Record::new(tag::GRAPH_MINUS, 0, zero),
        GraphPattern::Values {
            variables,
            bindings,
        } => Record::new(
            tag::GRAPH_VALUES,
            0,
            [
                usize_to_u32(variables.len())?,
                usize_to_u32(bindings.len())?,
                0,
                0,
                0,
            ],
        ),
        GraphPattern::OrderBy { expression, .. } => Record::new(
            tag::GRAPH_ORDER_BY,
            0,
            [usize_to_u32(expression.len())?, 0, 0, 0, 0],
        ),
        GraphPattern::Project { variables, .. } => Record::new(
            tag::GRAPH_PROJECT,
            0,
            [usize_to_u32(variables.len())?, 0, 0, 0, 0],
        ),
        GraphPattern::Distinct { .. } => Record::new(tag::GRAPH_DISTINCT, 0, zero),
        GraphPattern::Reduced { .. } => Record::new(tag::GRAPH_REDUCED, 0, zero),
        GraphPattern::Slice { start, length, .. } => {
            let start = *start as u64;
            let length_value = length.unwrap_or_default() as u64;
            Record::new(
                tag::GRAPH_SLICE,
                u16::from(length.is_some()),
                [
                    (start >> 32) as u32,
                    start as u32,
                    (length_value >> 32) as u32,
                    length_value as u32,
                    0,
                ],
            )
        }
        GraphPattern::Group {
            variables,
            aggregates,
            ..
        } => Record::new(
            tag::GRAPH_GROUP,
            0,
            [
                usize_to_u32(variables.len())?,
                usize_to_u32(aggregates.len())?,
                0,
                0,
                0,
            ],
        ),
        GraphPattern::Service { silent, .. } => {
            Record::new(tag::GRAPH_SERVICE, u16::from(*silent), zero)
        }
    })
}

fn expression_record(
    expression: &spargebra::algebra::Expression,
) -> Result<Record, QueryWireError> {
    use spargebra::algebra::Expression as E;
    let (tag, count) = match expression {
        E::NamedNode(_) => (tag::EXPR_NAMED_NODE, 0),
        E::Literal(_) => (tag::EXPR_LITERAL, 0),
        E::Variable(_) => (tag::EXPR_VARIABLE, 0),
        E::Or(_, _) => (tag::EXPR_OR, 0),
        E::And(_, _) => (tag::EXPR_AND, 0),
        E::Equal(_, _) => (tag::EXPR_EQUAL, 0),
        E::SameTerm(_, _) => (tag::EXPR_SAME_TERM, 0),
        E::Greater(_, _) => (tag::EXPR_GREATER, 0),
        E::GreaterOrEqual(_, _) => (tag::EXPR_GREATER_EQUAL, 0),
        E::Less(_, _) => (tag::EXPR_LESS, 0),
        E::LessOrEqual(_, _) => (tag::EXPR_LESS_EQUAL, 0),
        E::In(_, values) => (tag::EXPR_IN, values.len()),
        E::Add(_, _) => (tag::EXPR_ADD, 0),
        E::Subtract(_, _) => (tag::EXPR_SUBTRACT, 0),
        E::Multiply(_, _) => (tag::EXPR_MULTIPLY, 0),
        E::Divide(_, _) => (tag::EXPR_DIVIDE, 0),
        E::UnaryPlus(_) => (tag::EXPR_UNARY_PLUS, 0),
        E::UnaryMinus(_) => (tag::EXPR_UNARY_MINUS, 0),
        E::Not(_) => (tag::EXPR_NOT, 0),
        E::Exists(_) => (tag::EXPR_EXISTS, 0),
        E::Bound(_) => (tag::EXPR_BOUND, 0),
        E::If(_, _, _) => (tag::EXPR_IF, 0),
        E::Coalesce(values) => (tag::EXPR_COALESCE, values.len()),
        E::FunctionCall(_, values) => (tag::EXPR_FUNCTION_CALL, values.len()),
    };
    Ok(Record::new(tag, 0, [usize_to_u32(count)?, 0, 0, 0, 0]))
}

fn path_record(path: &PropertyPathExpression) -> Result<Record, QueryWireError> {
    let (tag, count) = match path {
        PropertyPathExpression::NamedNode(_) => (tag::PATH_NAMED_NODE, 0),
        PropertyPathExpression::Reverse(_) => (tag::PATH_REVERSE, 0),
        PropertyPathExpression::Sequence(_, _) => (tag::PATH_SEQUENCE, 0),
        PropertyPathExpression::Alternative(_, _) => (tag::PATH_ALTERNATIVE, 0),
        PropertyPathExpression::ZeroOrMore(_) => (tag::PATH_ZERO_OR_MORE, 0),
        PropertyPathExpression::OneOrMore(_) => (tag::PATH_ONE_OR_MORE, 0),
        PropertyPathExpression::ZeroOrOne(_) => (tag::PATH_ZERO_OR_ONE, 0),
        PropertyPathExpression::NegatedPropertySet(nodes) => (tag::PATH_NEGATED_SET, nodes.len()),
    };
    Ok(Record::new(tag, 0, [usize_to_u32(count)?, 0, 0, 0, 0]))
}

fn aggregate_function_record(
    arena: &mut Arena,
    function: &spargebra::algebra::AggregateFunction,
) -> Result<Record, QueryWireError> {
    let (code, flags, offset, len) = match encode_aggregate_function(function) {
        AggregateFunctionRef::Builtin(code) => (code, 0, 0, 0),
        AggregateFunctionRef::GroupConcat(None) => (6, 0, 0, 0),
        AggregateFunctionRef::GroupConcat(Some(separator)) => {
            let (offset, len) = append_scalar(arena, separator)?;
            (6, 1, offset, len)
        }
        AggregateFunctionRef::Custom(_) => (0, 0, 0, 0),
    };
    Ok(Record::new(
        tag::AGGREGATE_FUNCTION,
        flags,
        [code, offset, len, 0, 0],
    ))
}

fn scalar_record(arena: &mut Arena, tag: u16, value: &str) -> Result<Record, QueryWireError> {
    let (offset, len) = append_scalar(arena, value)?;
    Ok(Record::new(tag, 0, [offset, len, 0, 0, 0]))
}

fn literal_record(
    arena: &mut Arena,
    literal: &spargebra::term::Literal,
) -> Result<Record, QueryWireError> {
    let (value_offset, value_len) = append_scalar(arena, literal.value())?;
    let (kind, aux_offset, aux_len) = if let Some(language) = literal.language() {
        let (offset, len) = append_scalar(arena, language)?;
        let kind = match literal.direction() {
            None => 2,
            Some(BaseDirection::Ltr) => 3,
            Some(BaseDirection::Rtl) => 4,
        };
        (kind, offset, len)
    } else if literal.datatype() == oxrdf::vocab::xsd::STRING {
        (0, 0, 0)
    } else {
        let (offset, len) = append_scalar(arena, literal.datatype().as_str())?;
        (1, offset, len)
    };
    Ok(Record::new(
        tag::LITERAL,
        0,
        [value_offset, value_len, aux_offset, aux_len, kind],
    ))
}

fn append_scalar(arena: &mut Arena, value: &str) -> Result<(u32, u32), QueryWireError> {
    let offset = arena.scalar_bytes.len();
    let next = offset
        .checked_add(value.len())
        .ok_or(QueryWireError::AccountingOverflow)?;
    enforce_limit(next, MAX_SCALAR_BYTES_V1, QueryWireLimit::ScalarBytes)?;
    arena.scalar_bytes.try_reserve_exact(value.len())?;
    arena.scalar_bytes.extend_from_slice(value.as_bytes());
    Ok((usize_to_u32(offset)?, usize_to_u32(value.len())?))
}

fn validate_child_count(
    node: NodeRef<'_>,
    present_children: usize,
    encoded_edges: usize,
) -> Result<(), QueryWireError> {
    if matches!(node, NodeRef::ValuesRow(_)) {
        if present_children > encoded_edges {
            return Err(QueryWireError::InvalidRecord);
        }
    } else if present_children != encoded_edges {
        return Err(QueryWireError::InvalidRecord);
    }
    Ok(())
}
