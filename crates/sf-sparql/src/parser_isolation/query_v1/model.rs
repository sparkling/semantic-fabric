use super::error::{QueryWireError, QueryWireLimit};

pub(super) const MAGIC: [u8; 8] = *b"SFPQW001";
pub(super) const VERSION: u16 = 1;
pub(super) const HEADER_LEN: usize = 32;
pub(super) const RECORD_LEN: usize = 32;
pub(super) const NO_INDEX: u32 = u32::MAX;

pub(super) const MAX_RECORDS_V1: usize = 65_536;
pub(super) const MAX_EDGES_V1: usize = 131_072;
pub(super) const MAX_SCALAR_BYTES_V1: usize = 2 * 1024 * 1024;
pub(crate) const MAX_WIRE_BYTES_V1: usize = 8 * 1024 * 1024;
/// Provisional hard cap protecting reconstruction and recursive `Query` drop.
/// The tighter algebra depth envelope is checked again after reconstruction.
pub(super) const MAX_TREE_DEPTH_V1: usize = 256;

pub(super) mod tag {
    pub const QUERY_SELECT: u16 = 1;
    pub const QUERY_CONSTRUCT: u16 = 2;
    pub const QUERY_DESCRIBE: u16 = 3;
    pub const QUERY_ASK: u16 = 4;
    pub const DATASET: u16 = 5;

    pub const GRAPH_BGP: u16 = 10;
    pub const GRAPH_PATH: u16 = 11;
    pub const GRAPH_JOIN: u16 = 12;
    pub const GRAPH_LEFT_JOIN: u16 = 13;
    pub const GRAPH_LATERAL: u16 = 14;
    pub const GRAPH_FILTER: u16 = 15;
    pub const GRAPH_UNION: u16 = 16;
    pub const GRAPH_NAMED: u16 = 17;
    pub const GRAPH_EXTEND: u16 = 18;
    pub const GRAPH_MINUS: u16 = 19;
    pub const GRAPH_VALUES: u16 = 20;
    pub const GRAPH_ORDER_BY: u16 = 21;
    pub const GRAPH_PROJECT: u16 = 22;
    pub const GRAPH_DISTINCT: u16 = 23;
    pub const GRAPH_REDUCED: u16 = 24;
    pub const GRAPH_SLICE: u16 = 25;
    pub const GRAPH_GROUP: u16 = 26;
    pub const GRAPH_SERVICE: u16 = 27;

    pub const EXPR_NAMED_NODE: u16 = 40;
    pub const EXPR_LITERAL: u16 = 41;
    pub const EXPR_VARIABLE: u16 = 42;
    pub const EXPR_OR: u16 = 43;
    pub const EXPR_AND: u16 = 44;
    pub const EXPR_EQUAL: u16 = 45;
    pub const EXPR_SAME_TERM: u16 = 46;
    pub const EXPR_GREATER: u16 = 47;
    pub const EXPR_GREATER_EQUAL: u16 = 48;
    pub const EXPR_LESS: u16 = 49;
    pub const EXPR_LESS_EQUAL: u16 = 50;
    pub const EXPR_IN: u16 = 51;
    pub const EXPR_ADD: u16 = 52;
    pub const EXPR_SUBTRACT: u16 = 53;
    pub const EXPR_MULTIPLY: u16 = 54;
    pub const EXPR_DIVIDE: u16 = 55;
    pub const EXPR_UNARY_PLUS: u16 = 56;
    pub const EXPR_UNARY_MINUS: u16 = 57;
    pub const EXPR_NOT: u16 = 58;
    pub const EXPR_EXISTS: u16 = 59;
    pub const EXPR_BOUND: u16 = 60;
    pub const EXPR_IF: u16 = 61;
    pub const EXPR_COALESCE: u16 = 62;
    pub const EXPR_FUNCTION_CALL: u16 = 63;
    pub const FUNCTION: u16 = 64;

    pub const PATH_NAMED_NODE: u16 = 70;
    pub const PATH_REVERSE: u16 = 71;
    pub const PATH_SEQUENCE: u16 = 72;
    pub const PATH_ALTERNATIVE: u16 = 73;
    pub const PATH_ZERO_OR_MORE: u16 = 74;
    pub const PATH_ONE_OR_MORE: u16 = 75;
    pub const PATH_ZERO_OR_ONE: u16 = 76;
    pub const PATH_NEGATED_SET: u16 = 77;

    pub const AGGREGATE_COUNT_SOLUTIONS: u16 = 80;
    pub const AGGREGATE_FUNCTION_CALL: u16 = 81;
    pub const AGGREGATE_FUNCTION: u16 = 82;
    pub const ORDER_ASC: u16 = 83;
    pub const ORDER_DESC: u16 = 84;

    pub const TRIPLE_PATTERN: u16 = 90;
    pub const TERM_NAMED_NODE: u16 = 91;
    pub const TERM_BLANK_NODE: u16 = 92;
    pub const TERM_LITERAL: u16 = 93;
    pub const TERM_TRIPLE: u16 = 94;
    pub const TERM_VARIABLE: u16 = 95;
    pub const NAMED_PATTERN_NODE: u16 = 96;
    pub const NAMED_PATTERN_VARIABLE: u16 = 97;
    pub const GROUND_TERM_NAMED_NODE: u16 = 98;
    pub const GROUND_TERM_LITERAL: u16 = 99;
    pub const GROUND_TERM_TRIPLE: u16 = 100;
    pub const GROUND_TRIPLE: u16 = 101;

    pub const NAMED_NODE: u16 = 110;
    pub const VARIABLE: u16 = 111;
    pub const BLANK_NODE: u16 = 112;
    pub const LITERAL: u16 = 113;
    pub const BASE_IRI: u16 = 114;
    pub const VALUES_ROW: u16 = 120;
    pub const AGGREGATE_BINDING: u16 = 121;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Record {
    pub tag: u16,
    pub flags: u16,
    pub fields: [u32; 5],
    pub edge_start: u32,
    pub edge_len: u32,
}

impl Record {
    pub const fn new(tag: u16, flags: u16, fields: [u32; 5]) -> Self {
        Self {
            tag,
            flags,
            fields,
            edge_start: 0,
            edge_len: 0,
        }
    }
}

#[derive(Debug)]
pub(super) struct Arena {
    pub records: Vec<Record>,
    pub edges: Vec<u32>,
    pub scalar_bytes: Vec<u8>,
}

pub(super) fn checked_wire_len(
    record_count: usize,
    edge_count: usize,
    scalar_bytes: usize,
) -> Result<usize, QueryWireError> {
    enforce_limit(record_count, MAX_RECORDS_V1, QueryWireLimit::Records)?;
    enforce_limit(edge_count, MAX_EDGES_V1, QueryWireLimit::Edges)?;
    enforce_limit(
        scalar_bytes,
        MAX_SCALAR_BYTES_V1,
        QueryWireLimit::ScalarBytes,
    )?;
    let records = record_count
        .checked_mul(RECORD_LEN)
        .ok_or(QueryWireError::AccountingOverflow)?;
    let edges = edge_count
        .checked_mul(std::mem::size_of::<u32>())
        .ok_or(QueryWireError::AccountingOverflow)?;
    let total = HEADER_LEN
        .checked_add(records)
        .and_then(|value| value.checked_add(edges))
        .and_then(|value| value.checked_add(scalar_bytes))
        .ok_or(QueryWireError::AccountingOverflow)?;
    enforce_limit(total, MAX_WIRE_BYTES_V1, QueryWireLimit::WireBytes)?;
    Ok(total)
}

pub(super) fn enforce_limit(
    observed: usize,
    maximum: usize,
    limit: QueryWireLimit,
) -> Result<(), QueryWireError> {
    if observed > maximum {
        Err(QueryWireError::LimitExceeded(limit))
    } else {
        Ok(())
    }
}
