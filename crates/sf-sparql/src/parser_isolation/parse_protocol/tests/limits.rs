use super::*;

#[test]
fn request_source_limit_accepts_zero_and_exact_n_then_rejects_n_plus_one() {
    let empty = request("").encode().expect("zero source is a valid frame");
    assert_eq!(empty.len(), REQUEST_HEADER_LEN);
    assert_eq!(ParseRequestV1::decode_exact(&empty).unwrap().source(), "");

    let exact_source = "x".repeat(MAX_SOURCE_BYTES_V1);
    let exact = request(&exact_source).encode().expect("N source encodes");
    assert_eq!(exact.len(), REQUEST_HEADER_LEN + MAX_SOURCE_BYTES_V1);
    assert_eq!(
        ParseRequestV1::decode_exact(&exact).unwrap().source().len(),
        MAX_SOURCE_BYTES_V1
    );

    let over_source = "x".repeat(MAX_SOURCE_BYTES_V1 + 1);
    assert_eq!(
        ParseRequestV1::new(sample_nonce(), &over_source),
        Err(ParseFrameError::SourceLimitExceeded)
    );

    let mut declared_over = exact;
    declared_over.push(b'x');
    set_u64(
        &mut declared_over,
        BODY_LEN_OFFSET,
        (MAX_SOURCE_BYTES_V1 + 1) as u64,
    );
    assert_eq!(
        ParseRequestV1::decode_exact(&declared_over),
        Err(ParseFrameError::SourceLimitExceeded)
    );
}

#[test]
fn result_payload_limit_distinguishes_zero_exact_n_and_n_plus_one() {
    let request = request(SAMPLE_SOURCE);
    let rejection = rejection_bytes(SAMPLE_SOURCE, ParseRejectionV1::Syntax);
    assert_eq!(rejection.len(), RESULT_HEADER_LEN);
    assert!(ParseResultV1::decode_exact_for(&rejection, &request).is_ok());

    let canonical_success = success_bytes(SAMPLE_SOURCE);
    assert!(ParseResultV1::decode_exact_for(&canonical_success, &request).is_ok());

    let exact = synthetic_success(query_v1::MAX_QUERY_WIRE_BYTES);
    assert!(matches!(
        ParseResultV1::decode_exact_for(&exact, &request),
        Err(ParseFrameError::InvalidQueryPayload)
    ));

    let over = synthetic_success(query_v1::MAX_QUERY_WIRE_BYTES + 1);
    assert!(matches!(
        ParseResultV1::decode_exact_for(&over, &request),
        Err(ParseFrameError::PayloadLimitExceeded)
    ));
}

#[test]
fn declared_u64_lengths_cannot_wrap_platform_accounting() {
    let mut request_wire = request("").encode().unwrap();
    set_u64(&mut request_wire, BODY_LEN_OFFSET, u64::MAX);
    assert_eq!(
        ParseRequestV1::decode_exact(&request_wire),
        Err(ParseFrameError::LengthOverflow)
    );

    let request = request(SAMPLE_SOURCE);
    let mut result_wire = rejection_bytes(SAMPLE_SOURCE, ParseRejectionV1::Syntax);
    set_u64(&mut result_wire, BODY_LEN_OFFSET, u64::MAX);
    assert!(matches!(
        ParseResultV1::decode_exact_for(&result_wire, &request),
        Err(ParseFrameError::LengthOverflow)
    ));
}

#[test]
fn raw_frame_caps_run_before_any_header_access() {
    let exact_request = vec![0; MAX_REQUEST_FRAME_BYTES_V1];
    assert_eq!(
        ParseRequestV1::decode_exact(&exact_request),
        Err(ParseFrameError::InvalidMagic)
    );
    let over_request = vec![0; MAX_REQUEST_FRAME_BYTES_V1 + 1];
    assert_eq!(
        ParseRequestV1::decode_exact(&over_request),
        Err(ParseFrameError::SourceLimitExceeded)
    );

    let request = request(SAMPLE_SOURCE);
    let exact_result = vec![0; MAX_RESULT_FRAME_BYTES_V1];
    assert!(matches!(
        ParseResultV1::decode_exact_for(&exact_result, &request),
        Err(ParseFrameError::InvalidMagic)
    ));
    let over_result = vec![0; MAX_RESULT_FRAME_BYTES_V1 + 1];
    assert!(matches!(
        ParseResultV1::decode_exact_for(&over_result, &request),
        Err(ParseFrameError::PayloadLimitExceeded)
    ));
}

fn synthetic_success(payload_len: usize) -> Vec<u8> {
    let mut wire = success_bytes(SAMPLE_SOURCE);
    wire.resize(RESULT_HEADER_LEN + payload_len, 0);
    wire[KIND_OFFSET] = RESULT_SUCCESS_KIND;
    set_u64(&mut wire, BODY_LEN_OFFSET, payload_len as u64);
    set_u16(
        &mut wire,
        RESULT_QUERY_VERSION_OFFSET,
        query_v1::WIRE_VERSION,
    );
    set_u16(&mut wire, RESULT_REJECTION_OFFSET, 0);
    resign_result(&mut wire);
    wire
}
