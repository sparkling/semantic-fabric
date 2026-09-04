use super::*;

#[test]
fn request_rejects_truncation_trailing_bytes_and_declared_length_drift() {
    let wire = request(SAMPLE_SOURCE).encode().unwrap();
    assert_eq!(
        ParseRequestV1::decode_exact(&[]),
        Err(ParseFrameError::InvalidFrameLength)
    );
    assert_eq!(
        ParseRequestV1::decode_exact(&wire[..REQUEST_HEADER_LEN - 1]),
        Err(ParseFrameError::InvalidFrameLength)
    );
    assert_eq!(
        ParseRequestV1::decode_exact(&wire[..wire.len() - 1]),
        Err(ParseFrameError::InvalidFrameLength)
    );

    let mut trailing = wire.clone();
    trailing.push(0);
    assert_eq!(
        ParseRequestV1::decode_exact(&trailing),
        Err(ParseFrameError::InvalidFrameLength)
    );

    for declared in [SAMPLE_SOURCE.len() - 1, SAMPLE_SOURCE.len() + 1] {
        let mut drifted = wire.clone();
        set_u64(&mut drifted, BODY_LEN_OFFSET, declared as u64);
        assert_eq!(
            ParseRequestV1::decode_exact(&drifted),
            Err(ParseFrameError::InvalidFrameLength)
        );
    }
}

#[test]
fn request_rejects_unknown_header_fields_and_every_nonzero_reserve() {
    let canonical = request(SAMPLE_SOURCE).encode().unwrap();

    let mut magic = canonical.clone();
    magic[0] ^= 1;
    assert_eq!(
        ParseRequestV1::decode_exact(&magic),
        Err(ParseFrameError::InvalidMagic)
    );

    let mut version = canonical.clone();
    set_u16(&mut version, VERSION_OFFSET, PROTOCOL_VERSION + 1);
    assert_eq!(
        ParseRequestV1::decode_exact(&version),
        Err(ParseFrameError::UnsupportedVersion)
    );

    let mut kind = canonical.clone();
    kind[KIND_OFFSET] = 0xff;
    assert_eq!(
        ParseRequestV1::decode_exact(&kind),
        Err(ParseFrameError::UnsupportedMessageKind)
    );

    let mut flags = canonical.clone();
    flags[FLAGS_OFFSET] = 1;
    assert_eq!(
        ParseRequestV1::decode_exact(&flags),
        Err(ParseFrameError::NonCanonicalHeader)
    );

    let mut header_len = canonical.clone();
    set_u32(
        &mut header_len,
        HEADER_LEN_OFFSET,
        REQUEST_HEADER_LEN as u32 + 1,
    );
    assert_eq!(
        ParseRequestV1::decode_exact(&header_len),
        Err(ParseFrameError::NonCanonicalHeader)
    );

    for offset in REQUEST_RESERVED_OFFSET..REQUEST_HEADER_LEN {
        let mut reserved = canonical.clone();
        reserved[offset] = 1;
        assert_eq!(
            ParseRequestV1::decode_exact(&reserved),
            Err(ParseFrameError::NonCanonicalHeader),
            "reserved byte {offset}"
        );
    }

    let mut encoding = canonical.clone();
    set_u16(&mut encoding, REQUEST_ENCODING_OFFSET, UTF8_ENCODING + 1);
    assert_eq!(
        ParseRequestV1::decode_exact(&encoding),
        Err(ParseFrameError::UnsupportedSourceEncoding)
    );

    let mut query_version = canonical;
    set_u16(
        &mut query_version,
        REQUEST_QUERY_VERSION_OFFSET,
        query_v1::WIRE_VERSION + 1,
    );
    assert_eq!(
        ParseRequestV1::decode_exact(&query_version),
        Err(ParseFrameError::UnsupportedQueryVersion)
    );
}

#[test]
fn request_binds_exact_utf8_source_bytes() {
    let mut digest_mismatch = request(SAMPLE_SOURCE).encode().unwrap();
    digest_mismatch[REQUEST_HEADER_LEN] ^= 1;
    assert_eq!(
        ParseRequestV1::decode_exact(&digest_mismatch),
        Err(ParseFrameError::SourceDigestMismatch)
    );

    let mut invalid_utf8 = request("x").encode().unwrap();
    invalid_utf8[REQUEST_HEADER_LEN] = 0xff;
    resign_request(&mut invalid_utf8);
    assert_eq!(
        ParseRequestV1::decode_exact(&invalid_utf8),
        Err(ParseFrameError::InvalidSourceEncoding)
    );
}

#[test]
fn result_rejects_truncation_trailing_bytes_and_declared_length_drift() {
    let request = request(SAMPLE_SOURCE);
    let wire = success_bytes(SAMPLE_SOURCE);
    assert!(matches!(
        ParseResultV1::decode_exact_for(&[], &request),
        Err(ParseFrameError::InvalidFrameLength)
    ));
    assert!(matches!(
        ParseResultV1::decode_exact_for(&wire[..RESULT_HEADER_LEN - 1], &request),
        Err(ParseFrameError::InvalidFrameLength)
    ));
    assert!(matches!(
        ParseResultV1::decode_exact_for(&wire[..wire.len() - 1], &request),
        Err(ParseFrameError::InvalidFrameLength)
    ));

    let mut trailing = wire.clone();
    trailing.push(0);
    assert!(matches!(
        ParseResultV1::decode_exact_for(&trailing, &request),
        Err(ParseFrameError::InvalidFrameLength)
    ));

    for delta in [-1_i64, 1] {
        let mut drifted = wire.clone();
        let declared = i64::try_from(wire.len() - RESULT_HEADER_LEN).unwrap() + delta;
        set_u64(&mut drifted, BODY_LEN_OFFSET, declared as u64);
        assert!(matches!(
            ParseResultV1::decode_exact_for(&drifted, &request),
            Err(ParseFrameError::InvalidFrameLength)
        ));
    }
}

#[test]
fn result_rejects_unknown_header_fields_and_every_nonzero_reserve() {
    let request = request(SAMPLE_SOURCE);
    let canonical = success_bytes(SAMPLE_SOURCE);

    let mut magic = canonical.clone();
    magic[0] ^= 1;
    assert_result_error(&magic, &request, ParseFrameError::InvalidMagic);

    let mut version = canonical.clone();
    set_u16(&mut version, VERSION_OFFSET, PROTOCOL_VERSION + 1);
    assert_result_error(&version, &request, ParseFrameError::UnsupportedVersion);

    let mut kind = canonical.clone();
    kind[KIND_OFFSET] = 0xff;
    assert_result_error(&kind, &request, ParseFrameError::UnsupportedMessageKind);

    let mut flags = canonical.clone();
    flags[FLAGS_OFFSET] = 1;
    assert_result_error(&flags, &request, ParseFrameError::NonCanonicalHeader);

    let mut header_len = canonical.clone();
    set_u32(
        &mut header_len,
        HEADER_LEN_OFFSET,
        RESULT_HEADER_LEN as u32 + 1,
    );
    assert_result_error(&header_len, &request, ParseFrameError::NonCanonicalHeader);

    for offset in RESULT_RESERVED_OFFSET..RESULT_HEADER_LEN {
        let mut reserved = canonical.clone();
        reserved[offset] = 1;
        assert_result_error(&reserved, &request, ParseFrameError::NonCanonicalHeader);
    }
}

#[test]
fn result_rejects_all_invalid_kind_field_combinations() {
    let request = request(SAMPLE_SOURCE);
    let success = success_bytes(SAMPLE_SOURCE);
    let rejected = rejection_bytes(SAMPLE_SOURCE, ParseRejectionV1::Syntax);

    let mut success_with_rejection = success.clone();
    set_u16(&mut success_with_rejection, RESULT_REJECTION_OFFSET, 1);
    assert_result_error(
        &success_with_rejection,
        &request,
        ParseFrameError::InvalidResultShape,
    );

    let mut success_without_payload = success;
    success_without_payload.truncate(RESULT_HEADER_LEN);
    set_u64(&mut success_without_payload, BODY_LEN_OFFSET, 0);
    resign_result(&mut success_without_payload);
    assert_result_error(
        &success_without_payload,
        &request,
        ParseFrameError::InvalidResultShape,
    );

    let mut rejected_with_query_version = rejected.clone();
    set_u16(
        &mut rejected_with_query_version,
        RESULT_QUERY_VERSION_OFFSET,
        query_v1::WIRE_VERSION,
    );
    assert_result_error(
        &rejected_with_query_version,
        &request,
        ParseFrameError::InvalidResultShape,
    );

    for rejection in [
        ParseRejectionV1::Syntax,
        ParseRejectionV1::QueryEnvelope,
        ParseRejectionV1::ResourceExhausted,
    ] {
        let mut rejected_with_payload = rejection_bytes(SAMPLE_SOURCE, rejection);
        rejected_with_payload.push(0);
        set_u64(&mut rejected_with_payload, BODY_LEN_OFFSET, 1);
        resign_result(&mut rejected_with_payload);
        assert_result_error(
            &rejected_with_payload,
            &request,
            ParseFrameError::InvalidResultShape,
        );
    }

    for code in [0_u16, u16::MAX] {
        let mut unknown_rejection = rejected.clone();
        set_u16(&mut unknown_rejection, RESULT_REJECTION_OFFSET, code);
        assert_result_error(
            &unknown_rejection,
            &request,
            ParseFrameError::UnsupportedRejectionKind,
        );
    }

    let mut unknown_query_version = success_bytes(SAMPLE_SOURCE);
    set_u16(
        &mut unknown_query_version,
        RESULT_QUERY_VERSION_OFFSET,
        query_v1::WIRE_VERSION + 1,
    );
    assert_result_error(
        &unknown_query_version,
        &request,
        ParseFrameError::UnsupportedQueryVersion,
    );
}

#[test]
fn result_binds_request_and_payload_before_query_decode() {
    let request = request(SAMPLE_SOURCE);
    let canonical = success_bytes(SAMPLE_SOURCE);

    let mut nonce = canonical.clone();
    nonce[NONCE_OFFSET] ^= 1;
    assert_result_error(&nonce, &request, ParseFrameError::NonceMismatch);

    let mut source_digest = canonical.clone();
    source_digest[SOURCE_DIGEST_OFFSET] ^= 1;
    assert_result_error(
        &source_digest,
        &request,
        ParseFrameError::SourceDigestMismatch,
    );

    let mut payload_digest = canonical.clone();
    payload_digest[PAYLOAD_DIGEST_OFFSET] ^= 1;
    assert_result_error(
        &payload_digest,
        &request,
        ParseFrameError::PayloadDigestMismatch,
    );

    let mut malformed_query = canonical.clone();
    malformed_query[RESULT_HEADER_LEN] ^= 1;
    resign_result(&mut malformed_query);
    assert_result_error(
        &malformed_query,
        &request,
        ParseFrameError::InvalidQueryPayload,
    );

    let mut query_with_trailing_byte = canonical;
    query_with_trailing_byte.push(0);
    let payload_len = query_with_trailing_byte.len() - RESULT_HEADER_LEN;
    set_u64(
        &mut query_with_trailing_byte,
        BODY_LEN_OFFSET,
        payload_len as u64,
    );
    resign_result(&mut query_with_trailing_byte);
    assert_result_error(
        &query_with_trailing_byte,
        &request,
        ParseFrameError::InvalidQueryPayload,
    );
}

fn assert_result_error(wire: &[u8], request: &ParseRequestV1<'_>, expected: ParseFrameError) {
    assert!(matches!(
        ParseResultV1::decode_exact_for(wire, request),
        Err(error) if error == expected
    ));
}
