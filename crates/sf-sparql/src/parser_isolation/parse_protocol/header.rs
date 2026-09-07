//! Allocation-free structural preflight for streamed outer frame headers.

use super::binary::{checked_frame_len, read_u16, read_u32, read_u64, require_header};
use super::{
    enforce_source_limit, query_v1, ParseFrameError, ParseRejectionV1, BODY_LEN_OFFSET,
    FLAGS_OFFSET, HEADER_LEN_OFFSET, KIND_OFFSET, REQUEST_ENCODING_OFFSET, REQUEST_HEADER_LEN,
    REQUEST_KIND, REQUEST_MAGIC, REQUEST_QUERY_VERSION_OFFSET, REQUEST_RESERVED_OFFSET,
    RESULT_HEADER_LEN, RESULT_MAGIC, RESULT_QUERY_VERSION_OFFSET, RESULT_REJECTED_KIND,
    RESULT_REJECTION_OFFSET, RESULT_RESERVED_OFFSET, RESULT_SUCCESS_KIND, UTF8_ENCODING,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RequestHeaderV1 {
    body_len: usize,
    frame_len: usize,
}

impl RequestHeaderV1 {
    /// Validates only the fixed header and checked declared length. Correlation,
    /// source digest, source bytes, and UTF-8 validity require the complete frame.
    pub(crate) fn preflight(input: &[u8]) -> Result<Self, ParseFrameError> {
        if input.len() != REQUEST_HEADER_LEN {
            return Err(ParseFrameError::InvalidFrameLength);
        }
        require_header(input, REQUEST_HEADER_LEN, &REQUEST_MAGIC)?;
        if input[KIND_OFFSET] != REQUEST_KIND {
            return Err(ParseFrameError::UnsupportedMessageKind);
        }
        if input[FLAGS_OFFSET] != 0
            || read_u32(input, HEADER_LEN_OFFSET) != REQUEST_HEADER_LEN as u32
            || read_u32(input, REQUEST_RESERVED_OFFSET) != 0
        {
            return Err(ParseFrameError::NonCanonicalHeader);
        }
        if read_u16(input, REQUEST_ENCODING_OFFSET) != UTF8_ENCODING {
            return Err(ParseFrameError::UnsupportedSourceEncoding);
        }
        if read_u16(input, REQUEST_QUERY_VERSION_OFFSET) != query_v1::WIRE_VERSION {
            return Err(ParseFrameError::UnsupportedQueryVersion);
        }

        let frame_len = checked_frame_len(REQUEST_HEADER_LEN, read_u64(input, BODY_LEN_OFFSET))?;
        Ok(Self {
            body_len: frame_len - REQUEST_HEADER_LEN,
            frame_len,
        })
    }

    pub(crate) const fn body_len(self) -> usize {
        self.body_len
    }

    pub(crate) const fn frame_len(self) -> usize {
        self.frame_len
    }

    pub(crate) fn enforce_body_limit(self) -> Result<(), ParseFrameError> {
        enforce_source_limit(self.body_len)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResultHeaderV1 {
    kind: u8,
    query_version: u16,
    rejection_code: u16,
    body_len: usize,
    frame_len: usize,
}

impl ResultHeaderV1 {
    /// Validates only fixed fields and checked declared length. It deliberately
    /// does not inspect correlation, any digest, or the QueryV1 payload.
    pub(crate) fn preflight(input: &[u8]) -> Result<Self, ParseFrameError> {
        if input.len() != RESULT_HEADER_LEN {
            return Err(ParseFrameError::InvalidFrameLength);
        }
        require_header(input, RESULT_HEADER_LEN, &RESULT_MAGIC)?;
        let kind = input[KIND_OFFSET];
        if !matches!(kind, RESULT_SUCCESS_KIND | RESULT_REJECTED_KIND) {
            return Err(ParseFrameError::UnsupportedMessageKind);
        }
        if input[FLAGS_OFFSET] != 0
            || read_u32(input, HEADER_LEN_OFFSET) != RESULT_HEADER_LEN as u32
            || read_u32(input, RESULT_RESERVED_OFFSET) != 0
        {
            return Err(ParseFrameError::NonCanonicalHeader);
        }

        let frame_len = checked_frame_len(RESULT_HEADER_LEN, read_u64(input, BODY_LEN_OFFSET))?;
        Ok(Self {
            kind,
            query_version: read_u16(input, RESULT_QUERY_VERSION_OFFSET),
            rejection_code: read_u16(input, RESULT_REJECTION_OFFSET),
            body_len: frame_len - RESULT_HEADER_LEN,
            frame_len,
        })
    }

    pub(crate) const fn body_len(self) -> usize {
        self.body_len
    }

    pub(crate) const fn frame_len(self) -> usize {
        self.frame_len
    }

    pub(crate) fn enforce_body_limit(self) -> Result<(), ParseFrameError> {
        if self.body_len > query_v1::MAX_QUERY_WIRE_BYTES {
            Err(ParseFrameError::PayloadLimitExceeded)
        } else {
            Ok(())
        }
    }

    pub(crate) fn validate_shape(self) -> Result<(), ParseFrameError> {
        match self.kind {
            RESULT_SUCCESS_KIND => {
                if self.rejection_code != 0 || self.body_len == 0 {
                    return Err(ParseFrameError::InvalidResultShape);
                }
                if self.query_version != query_v1::WIRE_VERSION {
                    return Err(ParseFrameError::UnsupportedQueryVersion);
                }
            }
            RESULT_REJECTED_KIND => {
                if self.query_version != 0 || self.body_len != 0 {
                    return Err(ParseFrameError::InvalidResultShape);
                }
                ParseRejectionV1::decode(self.rejection_code)?;
            }
            _ => unreachable!("message kind checked during preflight"),
        }
        Ok(())
    }
}
