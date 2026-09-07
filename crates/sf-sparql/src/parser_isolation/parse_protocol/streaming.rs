//! Post-EOF request validation for the bounded streamed worker path.

use super::binary::ContentDigest;
use super::{
    HandshakeNonce, ParseFrameError, ParseRequestV1, RequestHeaderV1, DIGEST_LEN, NONCE_OFFSET,
    REQUEST_HEADER_LEN, SOURCE_DIGEST_OFFSET,
};

/// Validate a complete request whose fixed header and body were read separately.
///
/// The worker calls this only after observing stdin EOF. Correlation is checked
/// first so an unrelated request cannot force digest or UTF-8 work.
pub(crate) fn decode_streamed_request_exact_for_nonce<'source>(
    encoded_header: &[u8; REQUEST_HEADER_LEN],
    source_bytes: &'source [u8],
    expected_nonce: HandshakeNonce,
) -> Result<ParseRequestV1<'source>, ParseFrameError> {
    let header = RequestHeaderV1::preflight(encoded_header)?;
    header.enforce_body_limit()?;
    if header.body_len() != source_bytes.len() {
        return Err(ParseFrameError::InvalidFrameLength);
    }
    if encoded_header[NONCE_OFFSET..SOURCE_DIGEST_OFFSET] != expected_nonce.correlation_bytes()[..]
    {
        return Err(ParseFrameError::NonceMismatch);
    }

    let source_digest = ContentDigest::from_slice(
        &encoded_header[SOURCE_DIGEST_OFFSET..SOURCE_DIGEST_OFFSET + DIGEST_LEN],
    );
    if ContentDigest::of(source_bytes) != source_digest {
        return Err(ParseFrameError::SourceDigestMismatch);
    }
    let source =
        std::str::from_utf8(source_bytes).map_err(|_| ParseFrameError::InvalidSourceEncoding)?;
    Ok(ParseRequestV1 {
        nonce: expected_nonce,
        source,
        source_digest,
    })
}
