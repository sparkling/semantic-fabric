use super::protocol::{
    verify_ready, BuildIdentityDigest, HandshakeError, HandshakeNonce, HelloFrame, LimitField,
    ParserProfileDigest, ParserWorkerLimitValues, ParserWorkerLimits, ReadyFrame, DIGEST_LEN,
    FRAME_LEN,
};

const VERSION_OFFSET: usize = 8;
const KIND_OFFSET: usize = 10;
const RESERVED_OFFSET: usize = 11;
const LENGTH_OFFSET: usize = 12;
const LIMITS_OFFSET: usize = 112;
const LIMIT_WIDTH: usize = 8;

fn sample_values() -> ParserWorkerLimitValues {
    ParserWorkerLimitValues {
        stack_bytes: 8 * 1024 * 1024,
        address_space_bytes: 256 * 1024 * 1024,
        cpu_time_millis: 2_000,
        wall_time_millis: 3_000,
        max_input_bytes: 1024 * 1024,
        max_output_bytes: 8 * 1024 * 1024,
        max_open_fds: 8,
        max_processes: 1,
        max_concurrency: 4,
    }
}

fn sample_limits() -> ParserWorkerLimits {
    ParserWorkerLimits::new(sample_values()).expect("sample limits are valid")
}

fn sample_hello() -> HelloFrame {
    HelloFrame::new(
        HandshakeNonce::new([0x11; DIGEST_LEN]),
        BuildIdentityDigest::new([0x22; DIGEST_LEN]),
        ParserProfileDigest::new([0x33; DIGEST_LEN]),
        sample_limits(),
    )
}

fn sample_ready() -> ReadyFrame {
    ReadyFrame::new(
        HandshakeNonce::new([0x11; DIGEST_LEN]),
        BuildIdentityDigest::new([0x22; DIGEST_LEN]),
        ParserProfileDigest::new([0x33; DIGEST_LEN]),
        sample_limits(),
    )
}

#[test]
fn hello_and_ready_have_canonical_deterministic_round_trips() {
    let hello = sample_hello();
    let hello_bytes = hello.encode();
    assert_eq!(hello_bytes, hello.encode());
    assert_eq!(&hello_bytes[..8], b"SFPWHS01");
    assert_eq!(&hello_bytes[VERSION_OFFSET..KIND_OFFSET], &[0, 1]);
    assert_eq!(hello_bytes[KIND_OFFSET], 1);
    assert_eq!(hello_bytes[RESERVED_OFFSET], 0);
    assert_eq!(&hello_bytes[LENGTH_OFFSET..16], &184_u32.to_be_bytes());
    assert_eq!(
        &hello_bytes[LIMITS_OFFSET..LIMITS_OFFSET + LIMIT_WIDTH],
        &sample_values().stack_bytes.to_be_bytes()
    );
    assert_eq!(HelloFrame::decode(&hello_bytes), Ok(hello));

    let ready = sample_ready();
    let ready_bytes = ready.encode();
    assert_eq!(ready_bytes, ready.encode());
    assert_eq!(ready_bytes[KIND_OFFSET], 2);
    assert_eq!(ReadyFrame::decode(&ready_bytes), Ok(ready));
    assert_eq!(verify_ready(&hello, &ready), Ok(()));
}

#[test]
fn worker_acknowledgement_uses_independently_supplied_contract_values() {
    let hello = sample_hello();
    let ready = hello
        .acknowledge_verified(
            BuildIdentityDigest::new([0x22; DIGEST_LEN]),
            ParserProfileDigest::new([0x33; DIGEST_LEN]),
            sample_limits(),
        )
        .expect("independent values match the request");
    assert_eq!(verify_ready(&hello, &ready), Ok(()));

    assert_eq!(
        hello.acknowledge_verified(
            BuildIdentityDigest::new([0x23; DIGEST_LEN]),
            ParserProfileDigest::new([0x33; DIGEST_LEN]),
            sample_limits(),
        ),
        Err(HandshakeError::BuildIdentityMismatch)
    );
}

#[test]
fn decoder_requires_exact_zero_n_and_n_plus_one_frame_lengths() {
    let encoded = sample_hello().encode();
    assert_eq!(
        HelloFrame::decode(&[]),
        Err(HandshakeError::InvalidFrameLength)
    );
    assert_eq!(HelloFrame::decode(&encoded), Ok(sample_hello()));
    assert_eq!(
        HelloFrame::decode(&encoded[..FRAME_LEN - 1]),
        Err(HandshakeError::InvalidFrameLength)
    );
    let mut trailing = encoded.to_vec();
    trailing.push(0);
    assert_eq!(
        HelloFrame::decode(&trailing),
        Err(HandshakeError::InvalidFrameLength)
    );
}

#[test]
fn decoder_rejects_every_header_mutation_family() {
    let canonical = sample_hello().encode();

    let mut bad_magic = canonical;
    bad_magic[0] ^= 1;
    assert_eq!(
        HelloFrame::decode(&bad_magic),
        Err(HandshakeError::InvalidMagic)
    );

    let mut bad_version = canonical;
    bad_version[VERSION_OFFSET..KIND_OFFSET].copy_from_slice(&2_u16.to_be_bytes());
    assert_eq!(
        HelloFrame::decode(&bad_version),
        Err(HandshakeError::UnsupportedVersion)
    );

    let mut unknown_kind = canonical;
    unknown_kind[KIND_OFFSET] = 0xff;
    assert_eq!(
        HelloFrame::decode(&unknown_kind),
        Err(HandshakeError::UnsupportedMessageKind)
    );

    assert_eq!(
        HelloFrame::decode(&sample_ready().encode()),
        Err(HandshakeError::UnexpectedMessageKind)
    );
    assert_eq!(
        ReadyFrame::decode(&canonical),
        Err(HandshakeError::UnexpectedMessageKind)
    );

    let mut nonzero_reserved = canonical;
    nonzero_reserved[RESERVED_OFFSET] = 1;
    assert_eq!(
        HelloFrame::decode(&nonzero_reserved),
        Err(HandshakeError::NonCanonicalHeader)
    );

    let mut wrong_embedded_length = canonical;
    wrong_embedded_length[LENGTH_OFFSET..16].copy_from_slice(&185_u32.to_be_bytes());
    assert_eq!(
        HelloFrame::decode(&wrong_embedded_length),
        Err(HandshakeError::NonCanonicalHeader)
    );
}

#[test]
fn decoder_rejects_zero_for_each_limit_field() {
    let expected = [
        LimitField::StackBytes,
        LimitField::AddressSpaceBytes,
        LimitField::CpuTimeMillis,
        LimitField::WallTimeMillis,
        LimitField::MaxInputBytes,
        LimitField::MaxOutputBytes,
        LimitField::MaxOpenFds,
        LimitField::MaxProcesses,
        LimitField::MaxConcurrency,
    ];

    for (index, field) in expected.into_iter().enumerate() {
        let mut encoded = sample_hello().encode();
        let offset = LIMITS_OFFSET + index * LIMIT_WIDTH;
        encoded[offset..offset + LIMIT_WIDTH].fill(0);
        assert_eq!(
            HelloFrame::decode(&encoded),
            Err(HandshakeError::InvalidLimit(field)),
            "limit field {field:?}"
        );
    }
}

#[test]
fn limits_reject_nonportable_counts_resources_and_stack_relationship() {
    let resource_fields = [
        LimitField::StackBytes,
        LimitField::AddressSpaceBytes,
        LimitField::CpuTimeMillis,
        LimitField::WallTimeMillis,
        LimitField::MaxInputBytes,
        LimitField::MaxOutputBytes,
    ];
    for (index, field) in resource_fields.into_iter().enumerate() {
        let mut encoded = sample_hello().encode();
        write_limit(&mut encoded, index, i64::MAX as u64 + 1);
        assert_eq!(
            HelloFrame::decode(&encoded),
            Err(HandshakeError::InvalidLimit(field))
        );
    }

    let count_fields = [
        LimitField::MaxOpenFds,
        LimitField::MaxProcesses,
        LimitField::MaxConcurrency,
    ];
    for (relative_index, field) in count_fields.into_iter().enumerate() {
        let mut encoded = sample_hello().encode();
        write_limit(&mut encoded, 6 + relative_index, u64::from(u32::MAX) + 1);
        assert_eq!(
            HelloFrame::decode(&encoded),
            Err(HandshakeError::InvalidLimit(field))
        );
    }

    let mut stack_exceeds_address_space = sample_values();
    stack_exceeds_address_space.stack_bytes = stack_exceeds_address_space.address_space_bytes + 1;
    assert_eq!(
        ParserWorkerLimits::new(stack_exceeds_address_space),
        Err(HandshakeError::InvalidLimit(LimitField::AddressSpaceBytes))
    );
}

#[test]
fn limits_accept_every_exact_maximum_and_equal_stack_address_boundary() {
    let resource_max = i64::MAX as u64;
    let count_max = u64::from(u32::MAX);
    let values = ParserWorkerLimitValues {
        stack_bytes: resource_max,
        address_space_bytes: resource_max,
        cpu_time_millis: resource_max,
        wall_time_millis: resource_max,
        max_input_bytes: resource_max,
        max_output_bytes: resource_max,
        max_open_fds: count_max,
        max_processes: count_max,
        max_concurrency: count_max,
    };
    let limits = ParserWorkerLimits::new(values).expect("inclusive maxima must be valid");
    assert_eq!(limits.values(), values);

    let hello = HelloFrame::new(
        HandshakeNonce::new([0x11; DIGEST_LEN]),
        BuildIdentityDigest::new([0x22; DIGEST_LEN]),
        ParserProfileDigest::new([0x33; DIGEST_LEN]),
        limits,
    );
    assert_eq!(HelloFrame::decode(&hello.encode()), Ok(hello));
}

#[test]
fn ready_verifier_rejects_each_identity_mismatch_axis() {
    let hello = sample_hello();
    let nonce_mismatch = ReadyFrame::new(
        HandshakeNonce::new([0x12; DIGEST_LEN]),
        BuildIdentityDigest::new([0x22; DIGEST_LEN]),
        ParserProfileDigest::new([0x33; DIGEST_LEN]),
        sample_limits(),
    );
    assert_eq!(
        verify_ready(&hello, &nonce_mismatch),
        Err(HandshakeError::NonceMismatch)
    );
    assert_eq!(hello.nonce(), HandshakeNonce::new([0x11; DIGEST_LEN]));

    let build_mismatch = ReadyFrame::new(
        HandshakeNonce::new([0x11; DIGEST_LEN]),
        BuildIdentityDigest::new([0x23; DIGEST_LEN]),
        ParserProfileDigest::new([0x33; DIGEST_LEN]),
        sample_limits(),
    );
    assert_eq!(
        verify_ready(&hello, &build_mismatch),
        Err(HandshakeError::BuildIdentityMismatch)
    );

    let profile_mismatch = ReadyFrame::new(
        HandshakeNonce::new([0x11; DIGEST_LEN]),
        BuildIdentityDigest::new([0x22; DIGEST_LEN]),
        ParserProfileDigest::new([0x34; DIGEST_LEN]),
        sample_limits(),
    );
    assert_eq!(
        verify_ready(&hello, &profile_mismatch),
        Err(HandshakeError::ParserProfileMismatch)
    );
}

#[test]
fn ready_verifier_rejects_each_acknowledged_limit_mismatch_axis() {
    let hello = sample_hello();
    for index in 0..9 {
        let acknowledged = changed_limit(index);
        let ready = ReadyFrame::new(
            HandshakeNonce::new([0x11; DIGEST_LEN]),
            BuildIdentityDigest::new([0x22; DIGEST_LEN]),
            ParserProfileDigest::new([0x33; DIGEST_LEN]),
            acknowledged,
        );
        assert_eq!(
            verify_ready(&hello, &ready),
            Err(HandshakeError::AcknowledgedLimitsMismatch),
            "limit axis {index}"
        );
    }
}

#[test]
fn fixed_identity_debug_names_the_type_without_reflecting_bytes() {
    assert_eq!(
        format!("{:?}", HandshakeNonce::new([0x11; DIGEST_LEN])),
        "HandshakeNonce(<redacted>)"
    );
    assert_eq!(
        format!("{:?}", BuildIdentityDigest::new([0x22; DIGEST_LEN])),
        "BuildIdentityDigest(<redacted>)"
    );
    assert_eq!(
        format!("{:?}", ParserProfileDigest::new([0x33; DIGEST_LEN])),
        "ParserProfileDigest(<redacted>)"
    );

    let frame_debug = format!("{:?}", sample_hello());
    assert!(frame_debug.contains("HandshakeNonce(<redacted>)"));
    assert!(frame_debug.contains("BuildIdentityDigest(<redacted>)"));
    assert!(frame_debug.contains("ParserProfileDigest(<redacted>)"));
    assert!(!frame_debug.contains("[17, 17"));
    assert!(!frame_debug.contains("[34, 34"));
    assert!(!frame_debug.contains("[51, 51"));
}

#[test]
fn every_handshake_error_has_fixed_non_reflective_display_and_debug() {
    let cases = [
        (
            HandshakeError::InvalidFrameLength,
            "invalid parser-worker handshake frame length",
            "InvalidFrameLength",
        ),
        (
            HandshakeError::InvalidMagic,
            "invalid parser-worker handshake magic",
            "InvalidMagic",
        ),
        (
            HandshakeError::UnsupportedVersion,
            "unsupported parser-worker handshake version",
            "UnsupportedVersion",
        ),
        (
            HandshakeError::UnsupportedMessageKind,
            "unsupported parser-worker handshake message kind",
            "UnsupportedMessageKind",
        ),
        (
            HandshakeError::UnexpectedMessageKind,
            "unexpected parser-worker handshake message kind",
            "UnexpectedMessageKind",
        ),
        (
            HandshakeError::NonCanonicalHeader,
            "non-canonical parser-worker handshake header",
            "NonCanonicalHeader",
        ),
        (
            HandshakeError::InvalidLimit(LimitField::StackBytes),
            "invalid parser-worker stack-byte limit",
            "InvalidLimit(StackBytes)",
        ),
        (
            HandshakeError::NonceMismatch,
            "parser-worker nonce mismatch",
            "NonceMismatch",
        ),
        (
            HandshakeError::BuildIdentityMismatch,
            "parser-worker build identity mismatch",
            "BuildIdentityMismatch",
        ),
        (
            HandshakeError::ParserProfileMismatch,
            "parser-worker parser profile mismatch",
            "ParserProfileMismatch",
        ),
        (
            HandshakeError::AcknowledgedLimitsMismatch,
            "parser-worker acknowledged limits mismatch",
            "AcknowledgedLimitsMismatch",
        ),
    ];

    for (error, expected_display, expected_debug) in cases {
        assert_eq!(error.to_string(), expected_display);
        assert_eq!(format!("{error:?}"), expected_debug);
        assert!(!error.to_string().contains("DO_NOT_REFLECT"));
        assert!(!format!("{error:?}").contains("DO_NOT_REFLECT"));
    }
}

#[test]
fn errors_never_reflect_peer_controlled_bytes_or_values() {
    let marker = b"DO_NOT_REFLECT_THIS_VALUE";
    let mut encoded = sample_hello().encode();
    encoded[16..16 + marker.len()].copy_from_slice(marker);
    encoded[0] ^= 1;
    let error = HelloFrame::decode(&encoded).expect_err("mutated magic must fail");
    assert!(!error.to_string().contains("DO_NOT_REFLECT"));
    assert!(!format!("{error:?}").contains("DO_NOT_REFLECT"));

    let mut encoded = sample_hello().encode();
    write_limit(&mut encoded, 6, 0x5345_4352_4554);
    let error = HelloFrame::decode(&encoded).expect_err("invalid FD limit must fail");
    assert_eq!(error, HandshakeError::InvalidLimit(LimitField::MaxOpenFds));
    assert!(!error.to_string().contains("5345"));
}

fn write_limit(frame: &mut [u8; FRAME_LEN], index: usize, value: u64) {
    let offset = LIMITS_OFFSET + index * LIMIT_WIDTH;
    frame[offset..offset + LIMIT_WIDTH].copy_from_slice(&value.to_be_bytes());
}

fn changed_limit(index: usize) -> ParserWorkerLimits {
    let mut values = sample_values();
    match index {
        0 => values.stack_bytes += 1,
        1 => values.address_space_bytes += 1,
        2 => values.cpu_time_millis += 1,
        3 => values.wall_time_millis += 1,
        4 => values.max_input_bytes += 1,
        5 => values.max_output_bytes += 1,
        6 => values.max_open_fds += 1,
        7 => values.max_processes += 1,
        8 => values.max_concurrency += 1,
        _ => unreachable!(),
    }
    ParserWorkerLimits::new(values).expect("single-axis change remains valid")
}
