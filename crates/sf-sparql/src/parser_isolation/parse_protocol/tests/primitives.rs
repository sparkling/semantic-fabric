use spargebra::algebra::GraphPattern;

use super::*;

#[test]
fn request_header_preflight_is_allocation_free_and_body_agnostic() {
    let mut wire = request(SAMPLE_SOURCE).encode().unwrap();
    let expected_len = wire.len();

    wire[NONCE_OFFSET] ^= 1;
    wire[SOURCE_DIGEST_OFFSET] ^= 1;
    wire[REQUEST_HEADER_LEN] = 0xff;

    let header = RequestHeaderV1::preflight(&wire[..REQUEST_HEADER_LEN])
        .expect("structural request header remains valid");
    assert_eq!(header.body_len(), SAMPLE_SOURCE.len());
    assert_eq!(header.frame_len(), expected_len);
    header
        .enforce_body_limit()
        .expect("fixture body is bounded");
}

#[test]
fn result_header_preflight_is_allocation_free_and_payload_agnostic() {
    let request = request(SAMPLE_SOURCE);
    let mut wire = fixed_success(&request);
    let expected_len = wire.len();

    wire[NONCE_OFFSET] ^= 1;
    wire[SOURCE_DIGEST_OFFSET] ^= 1;
    wire[PAYLOAD_DIGEST_OFFSET] ^= 1;
    wire[RESULT_HEADER_LEN] ^= 1;

    let header = ResultHeaderV1::preflight(&wire[..RESULT_HEADER_LEN])
        .expect("structural result header remains valid");
    assert_eq!(header.body_len(), SYNTHETIC_EMPTY_ASK_QUERY_V1.len());
    assert_eq!(header.frame_len(), expected_len);
    header
        .enforce_body_limit()
        .expect("fixture payload is bounded");
    header.validate_shape().expect("success shape is canonical");
}

#[test]
fn header_preflights_check_declared_lengths_before_allocation() {
    let mut request_header = request("").encode().unwrap();
    set_u64(
        &mut request_header,
        BODY_LEN_OFFSET,
        (MAX_SOURCE_BYTES_V1 + 1) as u64,
    );
    let request_header = RequestHeaderV1::preflight(&request_header).unwrap();
    assert_eq!(
        request_header.enforce_body_limit(),
        Err(ParseFrameError::SourceLimitExceeded)
    );

    let correlated_request = request(SAMPLE_SOURCE);
    let mut result_header =
        ParseResultV1::encode_fixed_rejection_for(&correlated_request, ParseRejectionV1::Syntax);
    set_u64(
        &mut result_header,
        BODY_LEN_OFFSET,
        (query_v1::MAX_QUERY_WIRE_BYTES + 1) as u64,
    );
    result_header[KIND_OFFSET] = RESULT_SUCCESS_KIND;
    set_u16(
        &mut result_header,
        RESULT_QUERY_VERSION_OFFSET,
        query_v1::WIRE_VERSION,
    );
    set_u16(&mut result_header, RESULT_REJECTION_OFFSET, 0);
    let result_header = ResultHeaderV1::preflight(&result_header).unwrap();
    assert_eq!(
        result_header.enforce_body_limit(),
        Err(ParseFrameError::PayloadLimitExceeded)
    );

    let mut overflow = request("").encode().unwrap();
    set_u64(&mut overflow, BODY_LEN_OFFSET, u64::MAX);
    assert_eq!(
        RequestHeaderV1::preflight(&overflow),
        Err(ParseFrameError::LengthOverflow)
    );

    let mut result_overflow =
        ParseResultV1::encode_fixed_rejection_for(&correlated_request, ParseRejectionV1::Syntax);
    set_u64(&mut result_overflow, BODY_LEN_OFFSET, u64::MAX);
    assert_eq!(
        ResultHeaderV1::preflight(&result_overflow),
        Err(ParseFrameError::LengthOverflow)
    );
}

#[test]
fn header_preflights_require_exact_header_slices() {
    let request_wire = request(SAMPLE_SOURCE).encode().unwrap();
    assert_eq!(
        RequestHeaderV1::preflight(&request_wire[..REQUEST_HEADER_LEN - 1]),
        Err(ParseFrameError::InvalidFrameLength)
    );
    assert_eq!(
        RequestHeaderV1::preflight(&request_wire[..=REQUEST_HEADER_LEN]),
        Err(ParseFrameError::InvalidFrameLength)
    );

    let correlated_request = request(SAMPLE_SOURCE);
    let result_wire = fixed_success(&correlated_request);
    assert_eq!(
        ResultHeaderV1::preflight(&result_wire[..RESULT_HEADER_LEN - 1]),
        Err(ParseFrameError::InvalidFrameLength)
    );
    assert_eq!(
        ResultHeaderV1::preflight(&result_wire[..=RESULT_HEADER_LEN]),
        Err(ParseFrameError::InvalidFrameLength)
    );
}

#[test]
fn exact_frame_allocation_is_initialized_and_injectably_refused() {
    for total in [0, REQUEST_HEADER_LEN, REQUEST_HEADER_LEN + 17] {
        let mut frame = allocate_frame_exact(total, FrameAllocation::Attempt)
            .expect("system allocation succeeds");
        let capacity = frame.capacity();
        assert_eq!(frame.len(), total);
        assert!(frame.iter().all(|byte| *byte == 0));
        frame.fill(0xa5);
        assert_eq!(frame.capacity(), capacity, "initialization must not grow");
    }

    assert_eq!(
        allocate_frame_exact(REQUEST_HEADER_LEN, FrameAllocation::Refuse),
        Err(ParseFrameError::AllocationFailed)
    );
}

#[test]
fn static_empty_ask_fixture_has_independent_query_v1_replay() {
    let expected = Query::Ask {
        dataset: None,
        pattern: GraphPattern::Bgp {
            patterns: Vec::new(),
        },
        base_iri: None,
    };
    let decoded = query_v1::decode_exact(&SYNTHETIC_EMPTY_ASK_QUERY_V1)
        .expect("static empty-ASK QueryV1 decodes");
    assert_eq!(decoded, expected);
    assert_eq!(
        query_v1::encode(&decoded).expect("static empty-ASK QueryV1 re-encodes"),
        SYNTHETIC_EMPTY_ASK_QUERY_V1
    );
}

fn fixed_success(request: &ParseRequestV1<'_>) -> Vec<u8> {
    ParseResultV1::success_for(
        request,
        Query::Ask {
            dataset: None,
            pattern: GraphPattern::Bgp {
                patterns: Vec::new(),
            },
            base_iri: None,
        },
    )
    .encode()
    .expect("fixed success encodes")
}
