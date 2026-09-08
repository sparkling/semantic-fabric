//! Canonical flat wire for one parsed `spargebra::Query`.
//!
//! QueryV1 is an index-addressed postorder arena. Records may reference only
//! earlier records, every non-root record is consumed exactly once, edge and
//! scalar-byte ranges are contiguous, and the root is the final record. This
//! keeps decoding iterative and prevents hidden graph sharing, garbage, or
//! padding from becoming alternate encodings of the same query.

mod allocation;
mod binary;
mod decode;
mod decode_expression;
mod decode_graph;
mod decode_record;
mod decode_term;
mod decode_value;
mod encode;
mod encode_children;
mod encode_record;
mod error;
mod function;
mod model;
mod schema;

pub(super) const WIRE_VERSION: u16 = model::VERSION;
pub(super) const MAX_QUERY_WIRE_BYTES: usize = model::MAX_WIRE_BYTES_V1;
pub(super) use error::QueryWireError;

pub(super) fn encode(query: &spargebra::Query) -> Result<Vec<u8>, error::QueryWireError> {
    encode::encode(query)
}

pub(super) fn decode_exact(input: &[u8]) -> Result<spargebra::Query, error::QueryWireError> {
    decode::decode_exact(input)
}

#[cfg(test)]
mod tests;
