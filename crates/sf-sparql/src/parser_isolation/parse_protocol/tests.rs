use sha2::{Digest, Sha256};
use spargebra::{Query, SparqlParser};

use super::*;

mod golden;
mod limits;
mod mutations;

pub(super) const SAMPLE_SOURCE: &str = "ASK {}";

pub(super) fn sample_nonce() -> HandshakeNonce {
    let mut bytes = [0_u8; DIGEST_LEN];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = index as u8;
    }
    HandshakeNonce::new(bytes)
}

pub(super) fn request(source: &str) -> ParseRequestV1<'_> {
    ParseRequestV1::new(sample_nonce(), source).expect("fixture source is bounded")
}

pub(super) fn parse(source: &str) -> Query {
    SparqlParser::new()
        .parse_query(source)
        .unwrap_or_else(|error| panic!("invalid parse-protocol fixture: {error}"))
}

pub(super) fn success_bytes(source: &str) -> Vec<u8> {
    let request = request(source);
    ParseResultV1::success_for(&request, parse(source))
        .encode()
        .expect("success result encodes")
}

pub(super) fn rejection_bytes(source: &str, rejection: ParseRejectionV1) -> Vec<u8> {
    let request = request(source);
    ParseResultV1::rejected_for(&request, rejection)
        .encode()
        .expect("rejection result encodes")
}

pub(super) fn set_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

pub(super) fn set_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

pub(super) fn set_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
}

pub(super) fn resign_request(bytes: &mut [u8]) {
    let digest: [u8; DIGEST_LEN] = Sha256::digest(&bytes[REQUEST_HEADER_LEN..]).into();
    bytes[SOURCE_DIGEST_OFFSET..SOURCE_DIGEST_OFFSET + DIGEST_LEN].copy_from_slice(&digest);
}

pub(super) fn resign_result(bytes: &mut [u8]) {
    let digest: [u8; DIGEST_LEN] = Sha256::digest(&bytes[RESULT_HEADER_LEN..]).into();
    bytes[PAYLOAD_DIGEST_OFFSET..PAYLOAD_DIGEST_OFFSET + DIGEST_LEN].copy_from_slice(&digest);
}

#[test]
fn request_success_and_closed_rejections_round_trip_exactly() {
    let original_request = request(SAMPLE_SOURCE);
    let request_wire = original_request.encode().expect("request encodes");
    let decoded_request = ParseRequestV1::decode_exact_for_nonce(&request_wire, sample_nonce())
        .expect("request decodes with its retained handshake nonce");
    assert_eq!(decoded_request.nonce(), sample_nonce());
    assert_eq!(decoded_request.source(), SAMPLE_SOURCE);
    assert_eq!(decoded_request.encode().unwrap(), request_wire);

    let query = parse(SAMPLE_SOURCE);
    let success = ParseResultV1::success_for(&decoded_request, query.clone());
    let success_wire = success.encode().expect("success encodes");
    assert_eq!(success.encode().unwrap(), success_wire);
    let decoded_success =
        ParseResultV1::decode_exact_for(&success_wire, &decoded_request).expect("success decodes");
    assert_eq!(decoded_success.query(), Some(&query));
    assert_eq!(decoded_success.rejection(), None);
    assert_eq!(decoded_success.encode().unwrap(), success_wire);

    for rejection in [
        ParseRejectionV1::Syntax,
        ParseRejectionV1::QueryEnvelope,
        ParseRejectionV1::ResourceExhausted,
    ] {
        let result = ParseResultV1::rejected_for(&decoded_request, rejection);
        let wire = result.encode().expect("rejection encodes");
        let decoded =
            ParseResultV1::decode_exact_for(&wire, &decoded_request).expect("rejection decodes");
        assert!(decoded.query().is_none());
        assert_eq!(decoded.rejection(), Some(rejection));
        assert_eq!(decoded.encode().unwrap(), wire);
    }
}

#[test]
fn request_decode_requires_the_retained_handshake_nonce() {
    let request_wire = request(SAMPLE_SOURCE).encode().expect("request encodes");
    let different_nonce = HandshakeNonce::new([0xff; DIGEST_LEN]);

    assert_eq!(
        ParseRequestV1::decode_exact_for_nonce(&request_wire, different_nonce),
        Err(ParseFrameError::NonceMismatch)
    );
}

#[test]
fn owned_prepared_request_replays_exactly_without_self_reference() {
    let prepared =
        PreparedParseRequestV1::new(sample_nonce(), SAMPLE_SOURCE).expect("prepare owned request");

    assert_eq!(prepared.nonce(), sample_nonce());
    assert_eq!(
        prepared.encoded().len(),
        REQUEST_HEADER_LEN + SAMPLE_SOURCE.len()
    );
    prepared.verify_exact().expect("exact local replay");
}

#[test]
fn diagnostics_are_closed_and_redact_source_query_and_digests() {
    const SECRET: &str = "ASK { <https://secret.example/marker> ?p ?o }";
    let request = request(SECRET);
    let request_debug = format!("{request:?}");
    assert!(!request_debug.contains(SECRET));
    assert!(!request_debug.contains("secret.example"));
    assert!(request_debug.contains("<redacted>"));

    let rejected = ParseResultV1::rejected_for(&request, ParseRejectionV1::Syntax);
    let result_debug = format!("{rejected:?}");
    assert!(!result_debug.contains(SECRET));
    assert!(!result_debug.contains("secret.example"));
    assert!(result_debug.contains("Rejected(Syntax)"));

    for error in [
        ParseFrameError::InvalidSourceEncoding,
        ParseFrameError::SourceDigestMismatch,
        ParseFrameError::InvalidQueryPayload,
    ] {
        let rendered = error.to_string();
        assert!(!rendered.contains(SECRET));
        assert!(!rendered.contains("secret.example"));
    }
}

#[test]
fn rejection_frames_are_header_only_and_carry_no_source_or_parser_text() {
    const SOURCE_MARKER: &str = "SELECT * WHERE { <source-marker> ?p ?o }";
    let wire = rejection_bytes(SOURCE_MARKER, ParseRejectionV1::Syntax);
    assert_eq!(wire.len(), RESULT_HEADER_LEN);
    assert!(!wire
        .windows(b"source-marker".len())
        .any(|window| window == b"source-marker"));
    assert_eq!(read_u64(&wire, BODY_LEN_OFFSET), 0);
    assert_eq!(read_u16(&wire, RESULT_QUERY_VERSION_OFFSET), 0);
    assert_eq!(
        read_u16(&wire, RESULT_REJECTION_OFFSET),
        ParseRejectionV1::Syntax as u16
    );
}
