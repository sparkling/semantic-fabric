//! Flat request/result frames for the governed SQL-canonicalization peer.
//!
//! Deliberately independent of [`super::parse_protocol`]'s `QueryV1` wire: the
//! canonical output here is an opaque bounded UTF-8 SQL string, never a
//! recursive AST, so no flat-index encoder is needed. The frame ceilings are
//! DERIVED from the worker profile's immutable whole-life transport envelope
//! (see the static assertions below) rather than chosen independently, so a
//! frame this protocol admits can never be refused mid-transfer by the
//! shared cumulative byte cap. The envelope itself remains UNCALIBRATED
//! (ADR-0053's own labeling convention): change it only from checked-in
//! corpus and adversarial evidence.

use std::fmt;

use sha2::{Digest, Sha256};

use super::profile::{V1_CANDIDATE_MAX_INPUT_BYTES, V1_CANDIDATE_MAX_OUTPUT_BYTES};
use super::protocol::{HandshakeNonce, DIGEST_LEN, FRAME_LEN};

pub(crate) const REQUEST_HEADER_LEN: usize = 88;
pub(crate) const RESULT_HEADER_LEN: usize = 96;
/// Bounds the skeleton this peer will accept, independent of the request's
/// own SourceWork budget: a hard whole-frame ceiling before any allocation.
///
/// This peer shares the worker profile's immutable whole-life transport
/// envelope (`V1_CANDIDATE_MAX_INPUT_BYTES`), which also has to cover the
/// 184-byte `Hello` and this request header. The ceiling below is therefore
/// derived from that envelope rather than chosen independently, and the
/// static assertion under it keeps the two from silently drifting apart --
/// an independently larger value would be admitted by this protocol and then
/// rejected by the transport only after a partial transfer.
pub(crate) const MAX_SQL_SKELETON_BYTES_V1: usize =
    V1_CANDIDATE_MAX_INPUT_BYTES as usize - FRAME_LEN - REQUEST_HEADER_LEN;
/// Canonical `Display` re-serialization is byte-linear in the parsed AST
/// (measured, not assumed). Derived from the output envelope for the same
/// reason as the skeleton ceiling above; a hard ceiling, not an expected size.
pub(crate) const MAX_SQL_TEXT_BYTES_V1: usize =
    V1_CANDIDATE_MAX_OUTPUT_BYTES as usize - FRAME_LEN - RESULT_HEADER_LEN;

const REQUEST_MAGIC: [u8; 8] = *b"SFSQREQ1";
const RESULT_MAGIC: [u8; 8] = *b"SFSQRES1";
const WIRE_VERSION: u16 = 1;
const RESULT_SUCCESS_KIND: u8 = 1;
const RESULT_REJECTED_KIND: u8 = 2;

const VERSION_OFFSET: usize = 8;
const DIALECT_OFFSET: usize = 10;
const FLAGS_OFFSET: usize = 11;
const HEADER_LEN_OFFSET: usize = 12;
const BODY_LEN_OFFSET: usize = 16;
const NONCE_OFFSET: usize = 24;
const REQUEST_SOURCE_DIGEST_OFFSET: usize = NONCE_OFFSET + DIGEST_LEN;

const KIND_OFFSET: usize = 10;
const REJECTION_OFFSET: usize = 11;
const RESULT_SOURCE_DIGEST_OFFSET: usize = NONCE_OFFSET + DIGEST_LEN;
const RESERVED_OFFSET: usize = RESULT_SOURCE_DIGEST_OFFSET + DIGEST_LEN;

const _: () = {
    assert!(REQUEST_SOURCE_DIGEST_OFFSET + DIGEST_LEN == REQUEST_HEADER_LEN);
    assert!(RESERVED_OFFSET + 8 == RESULT_HEADER_LEN);
    // A maximal frame must fit the immutable whole-life transport envelope
    // exactly, so the protocol ceiling can never admit a frame the shared
    // cumulative cap would later refuse mid-transfer.
    assert!(
        (FRAME_LEN + REQUEST_HEADER_LEN + MAX_SQL_SKELETON_BYTES_V1) as u64
            == V1_CANDIDATE_MAX_INPUT_BYTES
    );
    assert!(
        (FRAME_LEN + RESULT_HEADER_LEN + MAX_SQL_TEXT_BYTES_V1) as u64
            == V1_CANDIDATE_MAX_OUTPUT_BYTES
    );
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SqlDialectCodeV1 {
    Postgres = 0,
    Sqlite = 1,
    MySql = 2,
}

impl SqlDialectCodeV1 {
    pub(crate) const fn from_dialect(dialect: sf_sql::Dialect) -> Option<Self> {
        match dialect {
            sf_sql::Dialect::Postgres => Some(Self::Postgres),
            sf_sql::Dialect::Sqlite => Some(Self::Sqlite),
            sf_sql::Dialect::MySql => Some(Self::MySql),
            _ => None,
        }
    }

    pub(crate) const fn to_dialect(self) -> sf_sql::Dialect {
        match self {
            Self::Postgres => sf_sql::Dialect::Postgres,
            Self::Sqlite => sf_sql::Dialect::Sqlite,
            Self::MySql => sf_sql::Dialect::MySql,
        }
    }

    fn decode(byte: u8) -> Result<Self, SqlFrameError> {
        match byte {
            0 => Ok(Self::Postgres),
            1 => Ok(Self::Sqlite),
            2 => Ok(Self::MySql),
            _ => Err(SqlFrameError::UnsupportedDialect),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ContentDigestV1([u8; DIGEST_LEN]);

impl ContentDigestV1 {
    pub(crate) fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    fn from_slice(slice: &[u8]) -> Self {
        let mut out = [0_u8; DIGEST_LEN];
        out.copy_from_slice(slice);
        Self(out)
    }

    fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SqlFrameError {
    InvalidFrameLength,
    InvalidMagic,
    UnsupportedVersion,
    NonCanonicalHeader,
    UnsupportedDialect,
    SkeletonLimitExceeded,
    TextLimitExceeded,
    InvalidSourceEncoding,
    NonceMismatch,
    SourceDigestMismatch,
    InvalidResultShape,
    UnsupportedRejectionKind,
    LengthOverflow,
    AllocationFailed,
}

impl fmt::Display for SqlFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidFrameLength => "invalid SQL canonicalize frame length",
            Self::InvalidMagic => "invalid SQL canonicalize frame magic",
            Self::UnsupportedVersion => "unsupported SQL canonicalize wire version",
            Self::NonCanonicalHeader => "non-canonical SQL canonicalize header",
            Self::UnsupportedDialect => "unsupported SQL canonicalize dialect code",
            Self::SkeletonLimitExceeded => "SQL canonicalize skeleton limit exceeded",
            Self::TextLimitExceeded => "SQL canonicalize text limit exceeded",
            Self::InvalidSourceEncoding => "invalid SQL canonicalize source encoding",
            Self::NonceMismatch => "SQL canonicalize nonce mismatch",
            Self::SourceDigestMismatch => "SQL canonicalize source digest mismatch",
            Self::InvalidResultShape => "invalid SQL canonicalize result shape",
            Self::UnsupportedRejectionKind => "unsupported SQL canonicalize rejection kind",
            Self::LengthOverflow => "SQL canonicalize frame length overflowed",
            Self::AllocationFailed => "SQL canonicalize frame allocation failed",
        })
    }
}

impl std::error::Error for SqlFrameError {}

pub(crate) fn allocate_exact(len: usize) -> Result<Vec<u8>, SqlFrameError> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(len)
        .map_err(|_| SqlFrameError::AllocationFailed)?;
    buffer.resize(len, 0);
    Ok(buffer)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(bytes[offset..offset + 2].try_into().expect("2 bytes"))
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("4 bytes"))
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(bytes[offset..offset + 8].try_into().expect("8 bytes"))
}

fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
}

/// One outbound request: a bounded skeleton plus its target dialect.
pub(crate) struct SqlCanonicalizeRequestV1<'a> {
    nonce: HandshakeNonce,
    dialect: SqlDialectCodeV1,
    skeleton: &'a str,
    source_digest: ContentDigestV1,
}

impl<'a> SqlCanonicalizeRequestV1<'a> {
    pub(crate) fn new(
        nonce: HandshakeNonce,
        dialect: SqlDialectCodeV1,
        skeleton: &'a str,
    ) -> Result<Self, SqlFrameError> {
        if skeleton.len() > MAX_SQL_SKELETON_BYTES_V1 {
            return Err(SqlFrameError::SkeletonLimitExceeded);
        }
        Ok(Self {
            nonce,
            dialect,
            skeleton,
            source_digest: ContentDigestV1::of(skeleton.as_bytes()),
        })
    }

    pub(crate) const fn nonce(&self) -> HandshakeNonce {
        self.nonce
    }

    pub(crate) const fn source_digest(&self) -> ContentDigestV1 {
        self.source_digest
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, SqlFrameError> {
        let total = REQUEST_HEADER_LEN
            .checked_add(self.skeleton.len())
            .ok_or(SqlFrameError::LengthOverflow)?;
        let mut output = allocate_exact(total)?;
        output[..REQUEST_MAGIC.len()].copy_from_slice(&REQUEST_MAGIC);
        write_u16(&mut output, VERSION_OFFSET, WIRE_VERSION);
        output[DIALECT_OFFSET] = self.dialect as u8;
        output[FLAGS_OFFSET] = 0;
        write_u32(&mut output, HEADER_LEN_OFFSET, REQUEST_HEADER_LEN as u32);
        write_u64(&mut output, BODY_LEN_OFFSET, self.skeleton.len() as u64);
        output[NONCE_OFFSET..REQUEST_SOURCE_DIGEST_OFFSET]
            .copy_from_slice(self.nonce.correlation_bytes());
        output[REQUEST_SOURCE_DIGEST_OFFSET..REQUEST_HEADER_LEN]
            .copy_from_slice(self.source_digest.as_bytes());
        output[REQUEST_HEADER_LEN..].copy_from_slice(self.skeleton.as_bytes());
        Ok(output)
    }
}

/// A decoded request, borrowing its skeleton text from the caller's buffer.
pub(crate) struct DecodedSqlRequestV1<'a> {
    pub(crate) nonce: HandshakeNonce,
    pub(crate) dialect: SqlDialectCodeV1,
    pub(crate) skeleton: &'a str,
    source_digest: ContentDigestV1,
}

/// Preflight only the fixed header; the caller reads exactly `body_len` more
/// bytes before calling [`decode_body`].
pub(crate) struct RequestHeaderV1 {
    dialect: SqlDialectCodeV1,
    body_len: usize,
}

impl RequestHeaderV1 {
    pub(crate) fn preflight(input: &[u8]) -> Result<Self, SqlFrameError> {
        if input.len() != REQUEST_HEADER_LEN {
            return Err(SqlFrameError::InvalidFrameLength);
        }
        if input[..REQUEST_MAGIC.len()] != REQUEST_MAGIC {
            return Err(SqlFrameError::InvalidMagic);
        }
        if read_u16(input, VERSION_OFFSET) != WIRE_VERSION {
            return Err(SqlFrameError::UnsupportedVersion);
        }
        if input[FLAGS_OFFSET] != 0
            || read_u32(input, HEADER_LEN_OFFSET) != REQUEST_HEADER_LEN as u32
        {
            return Err(SqlFrameError::NonCanonicalHeader);
        }
        let dialect = SqlDialectCodeV1::decode(input[DIALECT_OFFSET])?;
        let body_len = read_u64(input, BODY_LEN_OFFSET);
        let body_len =
            usize::try_from(body_len).map_err(|_| SqlFrameError::SkeletonLimitExceeded)?;
        if body_len > MAX_SQL_SKELETON_BYTES_V1 {
            return Err(SqlFrameError::SkeletonLimitExceeded);
        }
        Ok(Self { dialect, body_len })
    }

    pub(crate) const fn body_len(&self) -> usize {
        self.body_len
    }

    pub(crate) fn decode_body<'a>(
        self,
        header: &[u8],
        body: &'a [u8],
    ) -> Result<DecodedSqlRequestV1<'a>, SqlFrameError> {
        if body.len() != self.body_len {
            return Err(SqlFrameError::InvalidFrameLength);
        }
        let declared_digest =
            ContentDigestV1::from_slice(&header[REQUEST_SOURCE_DIGEST_OFFSET..REQUEST_HEADER_LEN]);
        if ContentDigestV1::of(body) != declared_digest {
            return Err(SqlFrameError::SourceDigestMismatch);
        }
        let skeleton =
            std::str::from_utf8(body).map_err(|_| SqlFrameError::InvalidSourceEncoding)?;
        let mut nonce = [0_u8; DIGEST_LEN];
        nonce.copy_from_slice(&header[NONCE_OFFSET..REQUEST_SOURCE_DIGEST_OFFSET]);
        Ok(DecodedSqlRequestV1 {
            nonce: HandshakeNonce::new(nonce),
            dialect: self.dialect,
            skeleton,
            source_digest: declared_digest,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum SqlRejectionV1 {
    Syntax = 1,
    ResourceExhausted = 2,
    Envelope = 3,
}

impl SqlRejectionV1 {
    fn decode(byte: u8) -> Result<Self, SqlFrameError> {
        match byte {
            1 => Ok(Self::Syntax),
            2 => Ok(Self::ResourceExhausted),
            3 => Ok(Self::Envelope),
            _ => Err(SqlFrameError::UnsupportedRejectionKind),
        }
    }
}

/// Build the worker's fixed rejection frame, entirely on the stack.
pub(crate) fn encode_rejection(
    request: &DecodedSqlRequestV1<'_>,
    rejection: SqlRejectionV1,
) -> [u8; RESULT_HEADER_LEN] {
    let mut output = [0_u8; RESULT_HEADER_LEN];
    output[..RESULT_MAGIC.len()].copy_from_slice(&RESULT_MAGIC);
    write_u16(&mut output, VERSION_OFFSET, WIRE_VERSION);
    output[KIND_OFFSET] = RESULT_REJECTED_KIND;
    output[REJECTION_OFFSET] = rejection as u8;
    write_u32(&mut output, HEADER_LEN_OFFSET, RESULT_HEADER_LEN as u32);
    write_u64(&mut output, BODY_LEN_OFFSET, 0);
    output[NONCE_OFFSET..RESULT_SOURCE_DIGEST_OFFSET]
        .copy_from_slice(request.nonce.correlation_bytes());
    output[RESULT_SOURCE_DIGEST_OFFSET..RESERVED_OFFSET]
        .copy_from_slice(request.source_digest.as_bytes());
    output
}

/// Build the worker's success frame: header plus the canonical UTF-8 body.
pub(crate) fn encode_success(
    request: &DecodedSqlRequestV1<'_>,
    canonical: &str,
) -> Result<Vec<u8>, SqlFrameError> {
    if canonical.is_empty() {
        return Err(SqlFrameError::InvalidResultShape);
    }
    if canonical.len() > MAX_SQL_TEXT_BYTES_V1 {
        return Err(SqlFrameError::TextLimitExceeded);
    }
    let total = RESULT_HEADER_LEN
        .checked_add(canonical.len())
        .ok_or(SqlFrameError::LengthOverflow)?;
    let mut output = allocate_exact(total)?;
    output[..RESULT_MAGIC.len()].copy_from_slice(&RESULT_MAGIC);
    write_u16(&mut output, VERSION_OFFSET, WIRE_VERSION);
    output[KIND_OFFSET] = RESULT_SUCCESS_KIND;
    output[REJECTION_OFFSET] = 0;
    write_u32(&mut output, HEADER_LEN_OFFSET, RESULT_HEADER_LEN as u32);
    write_u64(&mut output, BODY_LEN_OFFSET, canonical.len() as u64);
    output[NONCE_OFFSET..RESULT_SOURCE_DIGEST_OFFSET]
        .copy_from_slice(request.nonce.correlation_bytes());
    output[RESULT_SOURCE_DIGEST_OFFSET..RESERVED_OFFSET]
        .copy_from_slice(request.source_digest.as_bytes());
    output[RESULT_HEADER_LEN..].copy_from_slice(canonical.as_bytes());
    Ok(output)
}

#[cfg(feature = "sql-canonicalize-evidence")]
pub(crate) fn encode_oversized_result_for_evidence(
    request: &DecodedSqlRequestV1<'_>,
) -> [u8; RESULT_HEADER_LEN] {
    let mut output = encode_rejection(request, SqlRejectionV1::Syntax);
    output[KIND_OFFSET] = RESULT_SUCCESS_KIND;
    output[REJECTION_OFFSET] = 0;
    write_u64(
        &mut output,
        BODY_LEN_OFFSET,
        (MAX_SQL_TEXT_BYTES_V1 as u64) + 1,
    );
    output
}

#[derive(Debug)]
pub(crate) enum SqlCanonicalizeOutcomeV1 {
    Success(String),
    Rejected(SqlRejectionV1),
}

/// Preflight only the fixed result header; the caller reads exactly
/// `body_len` more bytes (capped by [`MAX_SQL_TEXT_BYTES_V1`]) before
/// calling [`decode_result_body`].
pub(crate) struct ResultHeaderV1 {
    kind: u8,
    rejection: u8,
    body_len: usize,
}

impl ResultHeaderV1 {
    pub(crate) fn preflight(input: &[u8]) -> Result<Self, SqlFrameError> {
        if input.len() != RESULT_HEADER_LEN {
            return Err(SqlFrameError::InvalidFrameLength);
        }
        if input[..RESULT_MAGIC.len()] != RESULT_MAGIC {
            return Err(SqlFrameError::InvalidMagic);
        }
        if read_u16(input, VERSION_OFFSET) != WIRE_VERSION {
            return Err(SqlFrameError::UnsupportedVersion);
        }
        let kind = input[KIND_OFFSET];
        if !matches!(kind, RESULT_SUCCESS_KIND | RESULT_REJECTED_KIND) {
            return Err(SqlFrameError::InvalidResultShape);
        }
        if read_u32(input, HEADER_LEN_OFFSET) != RESULT_HEADER_LEN as u32
            || input[RESERVED_OFFSET..RESULT_HEADER_LEN] != [0_u8; 8]
        {
            return Err(SqlFrameError::NonCanonicalHeader);
        }
        let body_len = read_u64(input, BODY_LEN_OFFSET);
        let body_len = usize::try_from(body_len).map_err(|_| SqlFrameError::TextLimitExceeded)?;
        if body_len > MAX_SQL_TEXT_BYTES_V1 {
            return Err(SqlFrameError::TextLimitExceeded);
        }
        match kind {
            RESULT_SUCCESS_KIND if body_len == 0 => return Err(SqlFrameError::InvalidResultShape),
            RESULT_REJECTED_KIND if body_len != 0 => return Err(SqlFrameError::InvalidResultShape),
            _ => {}
        }
        Ok(Self {
            kind,
            rejection: input[REJECTION_OFFSET],
            body_len,
        })
    }

    pub(crate) const fn body_len(&self) -> usize {
        self.body_len
    }

    pub(crate) fn decode_result_body(
        self,
        header: &[u8],
        body: &[u8],
        request_nonce: HandshakeNonce,
        request_source_digest: ContentDigestV1,
    ) -> Result<SqlCanonicalizeOutcomeV1, SqlFrameError> {
        if body.len() != self.body_len {
            return Err(SqlFrameError::InvalidFrameLength);
        }
        if header[NONCE_OFFSET..RESULT_SOURCE_DIGEST_OFFSET]
            != request_nonce.correlation_bytes()[..]
        {
            return Err(SqlFrameError::NonceMismatch);
        }
        if header[RESULT_SOURCE_DIGEST_OFFSET..RESERVED_OFFSET]
            != request_source_digest.as_bytes()[..]
        {
            return Err(SqlFrameError::SourceDigestMismatch);
        }
        if self.kind == RESULT_SUCCESS_KIND {
            let canonical =
                std::str::from_utf8(body).map_err(|_| SqlFrameError::InvalidSourceEncoding)?;
            Ok(SqlCanonicalizeOutcomeV1::Success(canonical.to_owned()))
        } else {
            Ok(SqlCanonicalizeOutcomeV1::Rejected(SqlRejectionV1::decode(
                self.rejection,
            )?))
        }
    }
}

#[cfg(test)]
#[path = "sql_canonicalize_protocol_tests.rs"]
mod protocol_tests;
