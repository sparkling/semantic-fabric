//! Owned pre-spawn request material for the control-only evidence session.

use super::{ContentDigest, ParseFrameError, ParseRequestV1};
use crate::parser_isolation::protocol::HandshakeNonce;

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
}
