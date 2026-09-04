//! Owned pre-spawn request material for the control-only evidence session.

#[cfg(feature = "query-v1-transport-mutant-evidence")]
use super::{
    allocate_frame_exact, FrameAllocation, DIGEST_LEN, NONCE_OFFSET, REQUEST_HEADER_LEN,
    SOURCE_DIGEST_OFFSET,
};
use super::{ContentDigest, ParseFrameError, ParseRequestV1};
use crate::parser_isolation::protocol::HandshakeNonce;

#[cfg(feature = "query-v1-transport-mutant-evidence")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestEofCorruption {
    Nonce,
    SourceDigest,
    InvalidUtf8,
}

#[cfg(feature = "query-v1-transport-mutant-evidence")]
impl RequestEofCorruption {
    pub(crate) const ALL: [Self; 3] = [Self::Nonce, Self::SourceDigest, Self::InvalidUtf8];
}

pub(crate) struct PreparedParseRequestV1 {
    encoded: Vec<u8>,
    nonce: HandshakeNonce,
    source_digest: ContentDigest,
    source_len: usize,
}

impl PreparedParseRequestV1 {
    pub(crate) fn new(nonce: HandshakeNonce, source: &str) -> Result<Self, ParseFrameError> {
        let request = ParseRequestV1::new(nonce, source)?;
        let source_digest = request.source_digest;
        let source_len = request.source.len();
        let encoded = request.encode()?;
        Ok(Self {
            encoded,
            nonce,
            source_digest,
            source_len,
        })
    }

    pub(crate) const fn nonce(&self) -> HandshakeNonce {
        self.nonce
    }

    /// Recheck all request correlation only after the control child is reaped.
    pub(crate) fn verify_exact(&self) -> Result<(), ParseFrameError> {
        let decoded = ParseRequestV1::decode_exact_for_nonce(&self.encoded, self.nonce)?;
        if decoded.nonce != self.nonce {
            return Err(ParseFrameError::NonceMismatch);
        }
        if decoded.source.len() != self.source_len || decoded.source_digest != self.source_digest {
            return Err(ParseFrameError::SourceDigestMismatch);
        }
        if decoded.encode()? != self.encoded {
            return Err(ParseFrameError::NonCanonicalHeader);
        }
        Ok(())
    }

    pub(crate) fn encoded(&self) -> &[u8] {
        &self.encoded
    }

    /// Build one fixed invalid request for live post-EOF ordering evidence.
    #[cfg(feature = "query-v1-transport-mutant-evidence")]
    pub(crate) fn corrupted_for_eof_evidence(
        &self,
        corruption: RequestEofCorruption,
    ) -> Result<Vec<u8>, ParseFrameError> {
        let mut encoded = allocate_frame_exact(self.encoded.len(), FrameAllocation::Attempt)?;
        encoded.copy_from_slice(&self.encoded);
        match corruption {
            RequestEofCorruption::Nonce => encoded[NONCE_OFFSET] ^= 1,
            RequestEofCorruption::SourceDigest => encoded[SOURCE_DIGEST_OFFSET] ^= 1,
            RequestEofCorruption::InvalidUtf8 => {
                let first_source = encoded
                    .get_mut(REQUEST_HEADER_LEN)
                    .ok_or(ParseFrameError::InvalidFrameLength)?;
                *first_source = 0xff;
                let digest = ContentDigest::of(&encoded[REQUEST_HEADER_LEN..]);
                encoded[SOURCE_DIGEST_OFFSET..SOURCE_DIGEST_OFFSET + DIGEST_LEN]
                    .copy_from_slice(digest.as_bytes());
            }
        }
        Ok(encoded)
    }
}

#[cfg(all(test, feature = "query-v1-transport-mutant-evidence"))]
mod tests {
    use super::*;
    use crate::parser_isolation::parse_protocol::{
        decode_streamed_request_exact_for_nonce, REQUEST_HEADER_LEN,
    };

    #[test]
    fn fixed_eof_corruptions_reach_three_distinct_streamed_validation_failures() {
        let nonce = HandshakeNonce::new([9; DIGEST_LEN]);
        let prepared = PreparedParseRequestV1::new(nonce, "x").unwrap();
        for (corruption, expected) in [
            (RequestEofCorruption::Nonce, ParseFrameError::NonceMismatch),
            (
                RequestEofCorruption::SourceDigest,
                ParseFrameError::SourceDigestMismatch,
            ),
            (
                RequestEofCorruption::InvalidUtf8,
                ParseFrameError::InvalidSourceEncoding,
            ),
        ] {
            let encoded = prepared.corrupted_for_eof_evidence(corruption).unwrap();
            let (header, source) = encoded.split_at(REQUEST_HEADER_LEN);
            assert_eq!(
                decode_streamed_request_exact_for_nonce(
                    header.try_into().unwrap(),
                    source,
                    prepared.nonce()
                ),
                Err(expected)
            );
        }
    }
}
