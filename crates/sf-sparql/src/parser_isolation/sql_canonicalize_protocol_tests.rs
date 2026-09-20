//! Frame round-trip and rejection tests for the SQL-canonicalization
//! wire, split out to keep that module within the 500-line source limit.

mod tests {
    use super::super::*;

    fn nonce(byte: u8) -> HandshakeNonce {
        HandshakeNonce::new([byte; DIGEST_LEN])
    }

    #[test]
    fn request_round_trips_through_header_preflight_and_body_decode() {
        let request =
            SqlCanonicalizeRequestV1::new(nonce(7), SqlDialectCodeV1::Postgres, "SELECT 1")
                .unwrap();
        let encoded = request.encode().unwrap();
        let (header, body) = encoded.split_at(REQUEST_HEADER_LEN);
        let preflight = RequestHeaderV1::preflight(header).unwrap();
        assert_eq!(preflight.body_len(), body.len());
        let decoded = preflight.decode_body(header, body).unwrap();
        assert_eq!(decoded.skeleton, "SELECT 1");
        assert_eq!(decoded.dialect, SqlDialectCodeV1::Postgres);
        assert_eq!(decoded.nonce, nonce(7));
    }

    #[test]
    fn tampered_source_digest_is_rejected() {
        let request =
            SqlCanonicalizeRequestV1::new(nonce(1), SqlDialectCodeV1::Sqlite, "SELECT 1").unwrap();
        let mut encoded = request.encode().unwrap();
        let last = encoded.len() - 1;
        encoded[last] ^= 1;
        let (header, body) = encoded.split_at(REQUEST_HEADER_LEN);
        let preflight = RequestHeaderV1::preflight(header).unwrap();
        assert!(matches!(
            preflight.decode_body(header, body),
            Err(SqlFrameError::SourceDigestMismatch)
        ));
    }

    #[test]
    fn oversized_declared_body_is_rejected_before_allocation() {
        let mut header = [0_u8; REQUEST_HEADER_LEN];
        header[..REQUEST_MAGIC.len()].copy_from_slice(&REQUEST_MAGIC);
        write_u16(&mut header, VERSION_OFFSET, WIRE_VERSION);
        header[DIALECT_OFFSET] = 0;
        write_u32(&mut header, HEADER_LEN_OFFSET, REQUEST_HEADER_LEN as u32);
        write_u64(
            &mut header,
            BODY_LEN_OFFSET,
            (MAX_SQL_SKELETON_BYTES_V1 + 1) as u64,
        );
        assert!(matches!(
            RequestHeaderV1::preflight(&header),
            Err(SqlFrameError::SkeletonLimitExceeded)
        ));
    }

    #[test]
    fn success_and_rejection_results_round_trip() {
        let request =
            SqlCanonicalizeRequestV1::new(nonce(3), SqlDialectCodeV1::MySql, "SELECT 1").unwrap();
        let encoded = request.encode().unwrap();
        let (header, body) = encoded.split_at(REQUEST_HEADER_LEN);
        let decoded = RequestHeaderV1::preflight(header)
            .unwrap()
            .decode_body(header, body)
            .unwrap();

        let success = encode_success(&decoded, "SELECT 1").unwrap();
        let (result_header, result_body) = success.split_at(RESULT_HEADER_LEN);
        let outcome = ResultHeaderV1::preflight(result_header)
            .unwrap()
            .decode_result_body(
                result_header,
                result_body,
                decoded.nonce,
                decoded.source_digest,
            )
            .unwrap();
        assert!(matches!(outcome, SqlCanonicalizeOutcomeV1::Success(sql) if sql == "SELECT 1"));

        let rejected = encode_rejection(&decoded, SqlRejectionV1::Syntax);
        let outcome = ResultHeaderV1::preflight(&rejected)
            .unwrap()
            .decode_result_body(&rejected, &[], decoded.nonce, decoded.source_digest)
            .unwrap();
        assert!(matches!(
            outcome,
            SqlCanonicalizeOutcomeV1::Rejected(SqlRejectionV1::Syntax)
        ));
    }

    #[test]
    fn mismatched_nonce_or_digest_on_result_is_rejected() {
        let request =
            SqlCanonicalizeRequestV1::new(nonce(5), SqlDialectCodeV1::Postgres, "SELECT 1")
                .unwrap();
        let encoded = request.encode().unwrap();
        let (header, body) = encoded.split_at(REQUEST_HEADER_LEN);
        let decoded = RequestHeaderV1::preflight(header)
            .unwrap()
            .decode_body(header, body)
            .unwrap();
        let success = encode_success(&decoded, "SELECT 1").unwrap();
        let (result_header, result_body) = success.split_at(RESULT_HEADER_LEN);
        assert!(matches!(
            ResultHeaderV1::preflight(result_header)
                .unwrap()
                .decode_result_body(result_header, result_body, nonce(9), decoded.source_digest),
            Err(SqlFrameError::NonceMismatch)
        ));
        assert!(matches!(
            ResultHeaderV1::preflight(result_header)
                .unwrap()
                .decode_result_body(
                    result_header,
                    result_body,
                    decoded.nonce,
                    ContentDigestV1::of(b"different")
                ),
            Err(SqlFrameError::SourceDigestMismatch)
        ));
    }
}
