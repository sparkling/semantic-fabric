use super::*;

#[test]
fn hex_codec_is_lowercase_and_exact() {
    let bytes = [0, 1, 15, 16, 127, 128, 254, 255];
    let encoded = encode_hex(&bytes);
    assert_eq!(encoded, "00010f107f80feff");
    assert_eq!(decode_hex(&encoded, 1).unwrap(), bytes);
    assert!(decode_hex("AA", 1).is_err());
    assert!(decode_hex("0", 1).is_err());
}

#[test]
fn record_and_receipt_domains_are_distinct() {
    assert_ne!(
        domain_sha256(super::super::OBSERVATION_DOMAIN, b"same"),
        domain_sha256(RECEIPT_DOMAIN, b"same")
    );
}

#[test]
fn stdout_assembly_holds_every_chunk_boundary() {
    for length in [1, 1023, 1024, 1025, MAX_LOADER_OUTPUT_BYTES] {
        let bytes = vec![b'x'; length];
        let chunks = bytes
            .chunks(STDOUT_CHUNK_BYTES)
            .map(<[u8]>::to_vec)
            .collect::<Vec<_>>();
        let assembled = assemble_stdout(chunks, length, length.div_ceil(1024), &sha256(&bytes));
        assert_eq!(assembled.unwrap(), bytes, "{length}");
    }
}

#[test]
fn stdout_assembly_rejects_short_nonfinal_and_oversize_claims() {
    let bytes = vec![b'x'; 1025];
    assert!(assemble_stdout(
        vec![bytes[..1023].to_vec(), bytes[1023..].to_vec()],
        bytes.len(),
        2,
        &sha256(&bytes),
    )
    .is_err());
    assert!(assemble_stdout(
        vec![vec![b'x']],
        MAX_LOADER_OUTPUT_BYTES + 1,
        1,
        &sha256(b"x"),
    )
    .is_err());
}

#[test]
fn text_and_metadata_budgets_are_exact() {
    assert_eq!(fixed_metadata().len() + 1, MAX_METADATA);
    assert!(validate_text_shape(&"x".repeat(MAX_PREPARED_RECEIPT_BYTES + 1)).is_err());
    assert!(validate_text_shape(&format!("{}\n", "x".repeat(MAX_LINE_BYTES + 1))).is_err());
    assert!(validate_text_shape("header\0\n").is_err());
}
