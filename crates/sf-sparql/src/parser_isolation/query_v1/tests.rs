use spargebra::{Query, SparqlParser};

use super::error::{QueryWireError, QueryWireLimit};
use super::model::{
    checked_wire_len, tag, HEADER_LEN, MAX_EDGES_V1, MAX_RECORDS_V1, MAX_SCALAR_BYTES_V1,
    MAX_WIRE_BYTES_V1, NO_INDEX, RECORD_LEN,
};
use super::{decode_exact, encode};

mod corpus;
mod mutations;

fn parse(source: &str) -> Query {
    SparqlParser::new()
        .parse_query(source)
        .unwrap_or_else(|error| panic!("invalid QueryV1 fixture: {error}\n{source}"))
}

fn assert_query_round_trip(label: &str, query: &Query) -> Vec<u8> {
    let first = encode(query).unwrap_or_else(|error| panic!("{label}: encode failed: {error}"));
    let second = encode(query).unwrap_or_else(|error| panic!("{label}: repeat failed: {error}"));
    assert_eq!(first, second, "{label}: encoding must be deterministic");

    let decoded =
        decode_exact(&first).unwrap_or_else(|error| panic!("{label}: decode failed: {error}"));
    assert_eq!(&decoded, query, "{label}: exact AST identity changed");
    assert_eq!(
        encode(&decoded).expect("decoded query must encode"),
        first,
        "{label}: canonical byte replay changed",
    );
    first
}

fn assert_source_round_trip(label: &str, source: &str) -> Vec<u8> {
    assert_query_round_trip(label, &parse(source))
}

fn canonical_wire() -> Vec<u8> {
    assert_source_round_trip(
        "mutation seed",
        concat!(
            "PREFIX ex: <https://example.test/> ",
            "SELECT ?s ?n WHERE { ",
            "VALUES (?s ?n) { (ex:s 1) (UNDEF \"café\"@fr) } ",
            "?s ex:p _:local . FILTER(?n > 0) }",
        ),
    )
}

fn read_u16(wire: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(wire[offset..offset + 2].try_into().expect("u16 field"))
}

fn read_u32(wire: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(wire[offset..offset + 4].try_into().expect("u32 field"))
}

fn write_u16(wire: &mut [u8], offset: usize, value: u16) {
    wire[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn write_u32(wire: &mut [u8], offset: usize, value: u32) {
    wire[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn record_count(wire: &[u8]) -> usize {
    read_u32(wire, 20) as usize
}

fn edge_count(wire: &[u8]) -> usize {
    read_u32(wire, 24) as usize
}

fn scalar_len(wire: &[u8]) -> usize {
    read_u32(wire, 28) as usize
}

fn record_offset(index: usize) -> usize {
    HEADER_LEN + index * RECORD_LEN
}

fn edge_table_offset(wire: &[u8]) -> usize {
    HEADER_LEN + record_count(wire) * RECORD_LEN
}

fn scalar_table_offset(wire: &[u8]) -> usize {
    edge_table_offset(wire) + edge_count(wire) * std::mem::size_of::<u32>()
}

fn record_tag(wire: &[u8], index: usize) -> u16 {
    read_u16(wire, record_offset(index))
}

fn record_edge_start(wire: &[u8], index: usize) -> usize {
    read_u32(wire, record_offset(index) + 24) as usize
}

fn record_edge_len(wire: &[u8], index: usize) -> usize {
    read_u32(wire, record_offset(index) + 28) as usize
}

fn find_record(wire: &[u8], wanted_tag: u16) -> usize {
    (0..record_count(wire))
        .find(|&index| record_tag(wire, index) == wanted_tag)
        .unwrap_or_else(|| panic!("fixture has no record tag {wanted_tag}"))
}

fn scalar_range(wire: &[u8], record: usize) -> std::ops::Range<usize> {
    let record = record_offset(record);
    let start = read_u32(wire, record + 4) as usize;
    let len = read_u32(wire, record + 8) as usize;
    let absolute = scalar_table_offset(wire) + start;
    absolute..absolute + len
}

fn auxiliary_scalar_range(wire: &[u8], record: usize) -> std::ops::Range<usize> {
    let record = record_offset(record);
    let start = read_u32(wire, record + 12) as usize;
    let len = read_u32(wire, record + 16) as usize;
    let absolute = scalar_table_offset(wire) + start;
    absolute..absolute + len
}

fn edge_offset(wire: &[u8], record: usize, position: usize) -> usize {
    assert!(position < record_edge_len(wire, record));
    edge_table_offset(wire)
        + (record_edge_start(wire, record) + position) * std::mem::size_of::<u32>()
}

fn record_tags(wire: &[u8]) -> std::collections::BTreeSet<u16> {
    (0..record_count(wire))
        .map(|index| record_tag(wire, index))
        .collect()
}

fn assert_has_tags(wire: &[u8], expected: &[u16]) {
    let actual = record_tags(wire);
    for expected in expected {
        assert!(actual.contains(expected), "missing QueryV1 tag {expected}");
    }
}

fn record_field_values(wire: &[u8], wanted_tag: u16, field: usize) -> Vec<u32> {
    assert!(field < 5);
    (0..record_count(wire))
        .filter(|&index| record_tag(wire, index) == wanted_tag)
        .map(|index| read_u32(wire, record_offset(index) + 4 + field * 4))
        .collect()
}

fn assert_rejected(wire: &[u8], label: &str) -> QueryWireError {
    decode_exact(wire).unwrap_err_or_else(label)
}

trait UnwrapWireError {
    fn unwrap_err_or_else(self, label: &str) -> QueryWireError;
}

impl UnwrapWireError for Result<Query, QueryWireError> {
    fn unwrap_err_or_else(self, label: &str) -> QueryWireError {
        match self {
            Ok(query) => panic!("{label}: malformed wire decoded as {query:?}"),
            Err(error) => error,
        }
    }
}
