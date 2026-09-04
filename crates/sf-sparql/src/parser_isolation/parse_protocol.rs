//! Canonical one-shot parse request/result frames for the dormant V1 worker.
//!
//! The codecs are deliberately private. Only the parser-free synthetic
//! transport evidence peer is connected to worker I/O; the parser peer remains
//! control-only. Decoding validates fixed headers, bounded lengths, correlation
//! fields and payload digests before any input-sized QueryV1 allocation.

use std::fmt;

use spargebra::Query;

use super::protocol::{HandshakeNonce, DIGEST_LEN};
use super::query_v1;

mod binary;
mod header;
mod prepared;
#[cfg(any(
    test,
    feature = "query-v1-transport-evidence",
    feature = "query-v1-transport-mutant-evidence"
))]
mod streaming;
#[cfg(any(
    test,
    feature = "query-v1-transport-evidence",
    feature = "query-v1-transport-mutant-evidence"
))]
mod synthetic;

pub(crate) use prepared::PreparedParseRequestV1;
#[cfg(feature = "query-v1-transport-mutant-evidence")]
pub(crate) use prepared::RequestEofCorruption;
#[cfg(any(
    test,
    feature = "query-v1-transport-evidence",
    feature = "query-v1-transport-mutant-evidence"
))]
pub(crate) use streaming::decode_streamed_request_exact_for_nonce;
#[cfg(feature = "query-v1-transport-mutant-evidence")]
pub(crate) use synthetic::{
    mutate_synthetic_result_header, synthetic_mutant_result_header_for_body,
    synthetic_mutant_result_header_for_payload, SyntheticResultHeaderMutation,
};
#[cfg(any(
    test,
    feature = "query-v1-transport-evidence",
    feature = "query-v1-transport-mutant-evidence"
))]
pub(crate) use synthetic::{synthetic_empty_ask_result_header_for, SYNTHETIC_EMPTY_ASK_QUERY_V1};

pub(crate) use binary::{allocate_frame_exact, FrameAllocation};
use binary::{encode_common_header, read_u16, write_u16, ContentDigest};
pub(crate) use header::{RequestHeaderV1, ResultHeaderV1};

#[cfg(test)]
use binary::read_u64;

pub(crate) const REQUEST_HEADER_LEN: usize = 96;
pub(crate) const RESULT_HEADER_LEN: usize = 128;
pub(crate) const MAX_SOURCE_BYTES_V1: usize = 1024 * 1024;
const MAX_REQUEST_FRAME_BYTES_V1: usize = REQUEST_HEADER_LEN + MAX_SOURCE_BYTES_V1;
const MAX_RESULT_FRAME_BYTES_V1: usize = RESULT_HEADER_LEN + query_v1::MAX_QUERY_WIRE_BYTES;

const REQUEST_MAGIC: [u8; 8] = *b"SFPREQ01";
const RESULT_MAGIC: [u8; 8] = *b"SFPRES01";
const PROTOCOL_VERSION: u16 = 1;
const REQUEST_KIND: u8 = 1;
const RESULT_SUCCESS_KIND: u8 = 1;
const RESULT_REJECTED_KIND: u8 = 2;
const UTF8_ENCODING: u16 = 1;

const VERSION_OFFSET: usize = 8;
const KIND_OFFSET: usize = 10;
const FLAGS_OFFSET: usize = 11;
const HEADER_LEN_OFFSET: usize = 12;
const BODY_LEN_OFFSET: usize = 16;
const NONCE_OFFSET: usize = 24;
const SOURCE_DIGEST_OFFSET: usize = NONCE_OFFSET + DIGEST_LEN;
const REQUEST_ENCODING_OFFSET: usize = SOURCE_DIGEST_OFFSET + DIGEST_LEN;
const REQUEST_QUERY_VERSION_OFFSET: usize = REQUEST_ENCODING_OFFSET + 2;
const REQUEST_RESERVED_OFFSET: usize = REQUEST_QUERY_VERSION_OFFSET + 2;
const PAYLOAD_DIGEST_OFFSET: usize = SOURCE_DIGEST_OFFSET + DIGEST_LEN;
const RESULT_QUERY_VERSION_OFFSET: usize = PAYLOAD_DIGEST_OFFSET + DIGEST_LEN;
const RESULT_REJECTION_OFFSET: usize = RESULT_QUERY_VERSION_OFFSET + 2;
const RESULT_RESERVED_OFFSET: usize = RESULT_REJECTION_OFFSET + 2;

const _: () = {
    assert!(REQUEST_RESERVED_OFFSET + 4 == REQUEST_HEADER_LEN);
    assert!(RESULT_RESERVED_OFFSET + 4 == RESULT_HEADER_LEN);
};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) struct ParseRequestV1<'source> {
    nonce: HandshakeNonce,
    source: &'source str,
    source_digest: ContentDigest,
}

impl<'source> ParseRequestV1<'source> {
    pub(crate) fn new(
        nonce: HandshakeNonce,
        source: &'source str,
    ) -> Result<Self, ParseFrameError> {
        enforce_source_limit(source.len())?;
        Ok(Self {
            nonce,
            source,
            source_digest: ContentDigest::of(source.as_bytes()),
        })
    }

    pub(crate) fn decode_exact(input: &'source [u8]) -> Result<Self, ParseFrameError> {
        if input.len() > MAX_REQUEST_FRAME_BYTES_V1 {
            return Err(ParseFrameError::SourceLimitExceeded);
        }
        if input.len() < REQUEST_HEADER_LEN {
            return Err(ParseFrameError::InvalidFrameLength);
        }
        let header = RequestHeaderV1::preflight(&input[..REQUEST_HEADER_LEN])?;
        header.enforce_body_limit()?;
        if input.len() != header.frame_len() {
            return Err(ParseFrameError::InvalidFrameLength);
        }

        let source_bytes = &input[REQUEST_HEADER_LEN..];
        let declared_digest = ContentDigest::from_slice(
            &input[SOURCE_DIGEST_OFFSET..SOURCE_DIGEST_OFFSET + DIGEST_LEN],
        );
        if ContentDigest::of(source_bytes) != declared_digest {
            return Err(ParseFrameError::SourceDigestMismatch);
        }
        let source = std::str::from_utf8(source_bytes)
            .map_err(|_| ParseFrameError::InvalidSourceEncoding)?;
        let mut nonce = [0_u8; DIGEST_LEN];
        nonce.copy_from_slice(&input[NONCE_OFFSET..SOURCE_DIGEST_OFFSET]);
        Ok(Self {
            nonce: HandshakeNonce::new(nonce),
            source,
            source_digest: declared_digest,
        })
    }

    pub(crate) fn decode_exact_for_nonce(
        input: &'source [u8],
        expected_nonce: HandshakeNonce,
    ) -> Result<Self, ParseFrameError> {
        let request = Self::decode_exact(input)?;
        if request.nonce != expected_nonce {
            return Err(ParseFrameError::NonceMismatch);
        }
        Ok(request)
    }

    pub(crate) fn encode(self) -> Result<Vec<u8>, ParseFrameError> {
        let total = REQUEST_HEADER_LEN
            .checked_add(self.source.len())
            .ok_or(ParseFrameError::LengthOverflow)?;
        let mut output = allocate_frame_exact(total, FrameAllocation::Attempt)?;
        encode_common_header(
            &mut output,
            &REQUEST_MAGIC,
            REQUEST_KIND,
            REQUEST_HEADER_LEN,
            self.source.len(),
            self.nonce,
            self.source_digest,
        )?;
        write_u16(&mut output, REQUEST_ENCODING_OFFSET, UTF8_ENCODING);
        write_u16(
            &mut output,
            REQUEST_QUERY_VERSION_OFFSET,
            query_v1::WIRE_VERSION,
        );
        output[REQUEST_HEADER_LEN..].copy_from_slice(self.source.as_bytes());
        Ok(output)
    }

    pub(crate) const fn nonce(&self) -> HandshakeNonce {
        self.nonce
    }

    pub(crate) const fn source(&self) -> &str {
        self.source
    }
}

impl fmt::Debug for ParseRequestV1<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParseRequestV1")
            .field("nonce", &self.nonce)
            .field("source", &"<redacted>")
            .field("source_digest", &self.source_digest)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub(crate) enum ParseRejectionV1 {
    Syntax = 1,
    QueryEnvelope = 2,
    ResourceExhausted = 3,
}

impl ParseRejectionV1 {
    fn decode(value: u16) -> Result<Self, ParseFrameError> {
        match value {
            1 => Ok(Self::Syntax),
            2 => Ok(Self::QueryEnvelope),
            3 => Ok(Self::ResourceExhausted),
            _ => Err(ParseFrameError::UnsupportedRejectionKind),
        }
    }
}

pub(crate) struct ParseResultV1 {
    nonce: HandshakeNonce,
    source_digest: ContentDigest,
    query: Option<Query>,
    rejection: Option<ParseRejectionV1>,
}

impl ParseResultV1 {
    pub(crate) fn success_for(request: &ParseRequestV1<'_>, query: Query) -> Self {
        Self {
            nonce: request.nonce,
            source_digest: request.source_digest,
            query: Some(query),
            rejection: None,
        }
    }

    pub(crate) fn rejected_for(request: &ParseRequestV1<'_>, rejection: ParseRejectionV1) -> Self {
        Self {
            nonce: request.nonce,
            source_digest: request.source_digest,
            query: None,
            rejection: Some(rejection),
        }
    }

    /// Encodes a rejection entirely on the stack, including the empty-payload
    /// digest. A worker whose fallible QueryV1 allocation was refused can use
    /// this path without needing another heap allocation for its response.
    pub(crate) fn encode_fixed_rejection_for(
        request: &ParseRequestV1<'_>,
        rejection: ParseRejectionV1,
    ) -> [u8; RESULT_HEADER_LEN] {
        let mut output = [0_u8; RESULT_HEADER_LEN];
        output[..RESULT_MAGIC.len()].copy_from_slice(&RESULT_MAGIC);
        output[VERSION_OFFSET..KIND_OFFSET].copy_from_slice(&PROTOCOL_VERSION.to_be_bytes());
        output[KIND_OFFSET] = RESULT_REJECTED_KIND;
        output[HEADER_LEN_OFFSET..BODY_LEN_OFFSET]
            .copy_from_slice(&(RESULT_HEADER_LEN as u32).to_be_bytes());
        output[BODY_LEN_OFFSET..NONCE_OFFSET].copy_from_slice(&0_u64.to_be_bytes());
        output[NONCE_OFFSET..SOURCE_DIGEST_OFFSET]
            .copy_from_slice(request.nonce.correlation_bytes());
        output[SOURCE_DIGEST_OFFSET..PAYLOAD_DIGEST_OFFSET]
            .copy_from_slice(request.source_digest.as_bytes());
        output[PAYLOAD_DIGEST_OFFSET..RESULT_QUERY_VERSION_OFFSET]
            .copy_from_slice(ContentDigest::of(&[]).as_bytes());
        output[RESULT_REJECTION_OFFSET..RESULT_RESERVED_OFFSET]
            .copy_from_slice(&(rejection as u16).to_be_bytes());
        output
    }

    pub(crate) fn decode_exact_for(
        input: &[u8],
        request: &ParseRequestV1<'_>,
    ) -> Result<Self, ParseFrameError> {
        if input.len() > MAX_RESULT_FRAME_BYTES_V1 {
            return Err(ParseFrameError::PayloadLimitExceeded);
        }
        if input.len() < RESULT_HEADER_LEN {
            return Err(ParseFrameError::InvalidFrameLength);
        }
        let header = ResultHeaderV1::preflight(&input[..RESULT_HEADER_LEN])?;
        header.enforce_body_limit()?;
        if input.len() != header.frame_len() {
            return Err(ParseFrameError::InvalidFrameLength);
        }
        header.validate_shape()?;

        if input[NONCE_OFFSET..SOURCE_DIGEST_OFFSET] != request.nonce.correlation_bytes()[..] {
            return Err(ParseFrameError::NonceMismatch);
        }
        if input[SOURCE_DIGEST_OFFSET..PAYLOAD_DIGEST_OFFSET]
            != request.source_digest.as_bytes()[..]
        {
            return Err(ParseFrameError::SourceDigestMismatch);
        }
        let payload = &input[RESULT_HEADER_LEN..];
        if input[PAYLOAD_DIGEST_OFFSET..RESULT_QUERY_VERSION_OFFSET]
            != ContentDigest::of(payload).as_bytes()[..]
        {
            return Err(ParseFrameError::PayloadDigestMismatch);
        }

        let (query, rejection) = if input[KIND_OFFSET] == RESULT_SUCCESS_KIND {
            (
                Some(
                    query_v1::decode_exact(payload)
                        .map_err(|_| ParseFrameError::InvalidQueryPayload)?,
                ),
                None,
            )
        } else {
            (
                None,
                Some(ParseRejectionV1::decode(read_u16(
                    input,
                    RESULT_REJECTION_OFFSET,
                ))?),
            )
        };
        Ok(Self {
            nonce: request.nonce,
            source_digest: request.source_digest,
            query,
            rejection,
        })
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, ParseFrameError> {
        let (kind, rejection_code, payload) = match (&self.query, self.rejection) {
            (Some(query), None) => (
                RESULT_SUCCESS_KIND,
                0,
                Some(query_v1::encode(query).map_err(|_| ParseFrameError::InvalidQueryPayload)?),
            ),
            (None, Some(rejection)) => (RESULT_REJECTED_KIND, rejection as u16, None),
            _ => return Err(ParseFrameError::InvalidResultShape),
        };
        let payload = payload.as_deref().unwrap_or_default();
        if payload.len() > query_v1::MAX_QUERY_WIRE_BYTES {
            return Err(ParseFrameError::PayloadLimitExceeded);
        }
        let total = RESULT_HEADER_LEN
            .checked_add(payload.len())
            .ok_or(ParseFrameError::LengthOverflow)?;
        let mut output = allocate_frame_exact(total, FrameAllocation::Attempt)?;
        encode_common_header(
            &mut output,
            &RESULT_MAGIC,
            kind,
            RESULT_HEADER_LEN,
            payload.len(),
            self.nonce,
            self.source_digest,
        )?;
        output[PAYLOAD_DIGEST_OFFSET..RESULT_QUERY_VERSION_OFFSET]
            .copy_from_slice(ContentDigest::of(payload).as_bytes());
        match kind {
            RESULT_SUCCESS_KIND => write_u16(
                &mut output,
                RESULT_QUERY_VERSION_OFFSET,
                query_v1::WIRE_VERSION,
            ),
            RESULT_REJECTED_KIND => {
                write_u16(&mut output, RESULT_REJECTION_OFFSET, rejection_code);
            }
            _ => unreachable!(),
        }
        output[RESULT_HEADER_LEN..].copy_from_slice(payload);
        Ok(output)
    }

    pub(crate) fn query(&self) -> Option<&Query> {
        self.query.as_ref()
    }

    pub(crate) const fn rejection(&self) -> Option<ParseRejectionV1> {
        self.rejection
    }
}

impl fmt::Debug for ParseResultV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let outcome = match (&self.query, self.rejection) {
            (Some(_), None) => "Success(<redacted>)",
            (None, Some(ParseRejectionV1::Syntax)) => "Rejected(Syntax)",
            (None, Some(ParseRejectionV1::QueryEnvelope)) => "Rejected(QueryEnvelope)",
            (None, Some(ParseRejectionV1::ResourceExhausted)) => "Rejected(ResourceExhausted)",
            _ => "Invalid(<redacted>)",
        };
        formatter
            .debug_struct("ParseResultV1")
            .field("nonce", &self.nonce)
            .field("source_digest", &self.source_digest)
            .field("outcome", &outcome)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParseFrameError {
    AllocationFailed,
    LengthOverflow,
    InvalidFrameLength,
    InvalidMagic,
    UnsupportedVersion,
    UnsupportedMessageKind,
    NonCanonicalHeader,
    SourceLimitExceeded,
    PayloadLimitExceeded,
    UnsupportedSourceEncoding,
    UnsupportedQueryVersion,
    InvalidSourceEncoding,
    NonceMismatch,
    SourceDigestMismatch,
    PayloadDigestMismatch,
    InvalidResultShape,
    UnsupportedRejectionKind,
    InvalidQueryPayload,
}

impl fmt::Display for ParseFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AllocationFailed => "parser protocol allocation failed",
            Self::LengthOverflow => "parser protocol length overflowed",
            Self::InvalidFrameLength => "invalid parser protocol frame length",
            Self::InvalidMagic => "invalid parser protocol magic",
            Self::UnsupportedVersion => "unsupported parser protocol version",
            Self::UnsupportedMessageKind => "unsupported parser protocol message kind",
            Self::NonCanonicalHeader => "non-canonical parser protocol header",
            Self::SourceLimitExceeded => "parser protocol source limit exceeded",
            Self::PayloadLimitExceeded => "parser protocol payload limit exceeded",
            Self::UnsupportedSourceEncoding => "unsupported parser protocol source encoding",
            Self::UnsupportedQueryVersion => "unsupported parser protocol query version",
            Self::InvalidSourceEncoding => "invalid parser protocol source encoding",
            Self::NonceMismatch => "parser protocol nonce mismatch",
            Self::SourceDigestMismatch => "parser protocol source digest mismatch",
            Self::PayloadDigestMismatch => "parser protocol payload digest mismatch",
            Self::InvalidResultShape => "invalid parser protocol result shape",
            Self::UnsupportedRejectionKind => "unsupported parser protocol rejection kind",
            Self::InvalidQueryPayload => "invalid parser protocol query payload",
        })
    }
}

impl std::error::Error for ParseFrameError {}

fn enforce_source_limit(source_len: usize) -> Result<(), ParseFrameError> {
    if source_len > MAX_SOURCE_BYTES_V1 {
        Err(ParseFrameError::SourceLimitExceeded)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
