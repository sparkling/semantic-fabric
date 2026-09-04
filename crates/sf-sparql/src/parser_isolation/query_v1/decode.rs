use spargebra::Query;

use super::binary::{inspect_wire, u32_to_usize, BorrowedWire};
use super::decode_record::decode_record;
use super::decode_value::Value;
use super::encode::encode;
use super::error::{QueryWireError, QueryWireLimit};
use super::model::{tag, MAX_TREE_DEPTH_V1, NO_INDEX};
use super::schema::{is_query_tag, scalar_spans, validate_record_shape, validate_scalar_values};

pub(crate) fn decode_exact(input: &[u8]) -> Result<Query, QueryWireError> {
    let wire = inspect_wire(input)?;
    preflight(&wire)?;
    let query = reconstruct(&wire)?;
    let canonical = encode(&query)?;
    if canonical.as_slice() != input {
        return Err(QueryWireError::NonCanonical);
    }
    Ok(query)
}

fn preflight(wire: &BorrowedWire<'_>) -> Result<(), QueryWireError> {
    let root = wire.record_count() - 1;
    let mut edge_cursor = 0_usize;
    let mut scalar_cursor = 0_usize;
    for index in 0..wire.record_count() {
        let record = wire.record(index)?;
        validate_record_shape(record)?;
        if u32_to_usize(record.edge_start)? != edge_cursor {
            return Err(QueryWireError::InvalidIndex);
        }
        let edge_end = edge_cursor
            .checked_add(u32_to_usize(record.edge_len)?)
            .ok_or(QueryWireError::AccountingOverflow)?;
        if edge_end > wire.edge_count() {
            return Err(QueryWireError::InvalidIndex);
        }
        for edge_index in edge_cursor..edge_end {
            let child = wire.edge(edge_index)?;
            if child == NO_INDEX {
                if record.tag != tag::VALUES_ROW {
                    return Err(QueryWireError::InvalidIndex);
                }
            } else if u32_to_usize(child)? >= index {
                return Err(QueryWireError::InvalidIndex);
            }
        }
        edge_cursor = edge_end;

        let (spans, count) = scalar_spans(record);
        for (offset, len) in spans.into_iter().take(count) {
            scalar_cursor = validate_scalar_span(wire.scalar_bytes(), scalar_cursor, offset, len)?;
        }
        validate_scalar_values(record, wire.scalar_bytes())?;
    }
    if edge_cursor != wire.edge_count() || scalar_cursor != wire.scalar_bytes().len() {
        return Err(QueryWireError::InvalidLength);
    }
    if !is_query_tag(wire.record(root)?.tag) {
        return Err(QueryWireError::InvalidRecord);
    }
    validate_canonical_tree(wire)
}

#[derive(Clone, Copy)]
struct TraversalFrame {
    edge_start: usize,
    next_edge: usize,
}

impl TraversalFrame {
    const EMPTY: Self = Self {
        edge_start: 0,
        next_edge: 0,
    };

    fn for_record(record: super::model::Record) -> Result<Self, QueryWireError> {
        let edge_start = u32_to_usize(record.edge_start)?;
        let next_edge = edge_start
            .checked_add(u32_to_usize(record.edge_len)?)
            .ok_or(QueryWireError::AccountingOverflow)?;
        Ok(Self {
            edge_start,
            next_edge,
        })
    }
}

fn validate_canonical_tree(wire: &BorrowedWire<'_>) -> Result<(), QueryWireError> {
    let root = wire.record_count() - 1;
    let mut expected = root.checked_sub(1);
    let mut stack = [TraversalFrame::EMPTY; MAX_TREE_DEPTH_V1];
    stack[0] = TraversalFrame::for_record(wire.record(root)?)?;
    let mut stack_len = 1_usize;

    while stack_len != 0 {
        let frame = &mut stack[stack_len - 1];
        if frame.next_edge == frame.edge_start {
            stack_len -= 1;
            continue;
        }
        frame.next_edge -= 1;
        let child = wire.edge(frame.next_edge)?;
        if child == NO_INDEX {
            continue;
        }
        let child = u32_to_usize(child)?;
        if expected != Some(child) {
            return Err(QueryWireError::InvalidIndex);
        }
        if stack_len == MAX_TREE_DEPTH_V1 {
            return Err(QueryWireError::LimitExceeded(QueryWireLimit::TreeDepth));
        }
        stack[stack_len] = TraversalFrame::for_record(wire.record(child)?)?;
        stack_len += 1;
        expected = child.checked_sub(1);
    }

    if expected.is_some() {
        return Err(QueryWireError::InvalidIndex);
    }
    Ok(())
}

fn validate_scalar_span(
    scalar_bytes: &[u8],
    expected_offset: usize,
    offset: u32,
    len: u32,
) -> Result<usize, QueryWireError> {
    let offset = u32_to_usize(offset)?;
    let len = u32_to_usize(len)?;
    if offset != expected_offset {
        return Err(QueryWireError::InvalidScalar);
    }
    let end = offset
        .checked_add(len)
        .ok_or(QueryWireError::AccountingOverflow)?;
    let value = scalar_bytes
        .get(offset..end)
        .ok_or(QueryWireError::InvalidScalar)?;
    std::str::from_utf8(value).map_err(|_| QueryWireError::InvalidScalar)?;
    Ok(end)
}

fn reconstruct(wire: &BorrowedWire<'_>) -> Result<Query, QueryWireError> {
    let mut values = Vec::new();
    values.try_reserve_exact(wire.record_count())?;
    values.resize_with(wire.record_count(), || None);

    for index in 0..wire.record_count() {
        let record = wire.record(index)?;
        let edge_start = u32_to_usize(record.edge_start)?;
        let edge_len = u32_to_usize(record.edge_len)?;
        let edge_end = edge_start
            .checked_add(edge_len)
            .ok_or(QueryWireError::AccountingOverflow)?;
        let mut children = Vec::new();
        children.try_reserve_exact(edge_len)?;
        for edge_index in edge_start..edge_end {
            let child = wire.edge(edge_index)?;
            if child == NO_INDEX {
                children.push(None);
            } else {
                let child = u32_to_usize(child)?;
                let value = values
                    .get_mut(child)
                    .ok_or(QueryWireError::InvalidIndex)?
                    .take()
                    .ok_or(QueryWireError::InvalidIndex)?;
                children.push(Some(value));
            }
        }
        values[index] = Some(decode_record(record, children, wire.scalar_bytes())?);
    }

    let root = values
        .last_mut()
        .ok_or(QueryWireError::InvalidRecord)?
        .take()
        .ok_or(QueryWireError::InvalidRecord)?;
    if values.iter().any(Option::is_some) {
        return Err(QueryWireError::InvalidIndex);
    }
    match root {
        Value::Query(query) => Ok(query),
        _ => Err(QueryWireError::TypeMismatch),
    }
}
