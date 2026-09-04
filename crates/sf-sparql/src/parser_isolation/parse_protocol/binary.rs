use std::fmt;

use sha2::{Digest, Sha256};

use super::{
    HandshakeNonce, ParseFrameError, BODY_LEN_OFFSET, DIGEST_LEN, HEADER_LEN_OFFSET, KIND_OFFSET,
    NONCE_OFFSET, PROTOCOL_VERSION, SOURCE_DIGEST_OFFSET, VERSION_OFFSET,
};

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct ContentDigest([u8; DIGEST_LEN]);

impl ContentDigest {
    pub(super) fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    pub(super) const fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
        &self.0
    }

    pub(super) fn from_slice(bytes: &[u8]) -> Self {
        let mut digest = [0_u8; DIGEST_LEN];
        digest.copy_from_slice(bytes);
        Self(digest)
    }
}

impl fmt::Debug for ContentDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ContentDigest(<redacted>)")
    }
}

pub(super) fn require_header(
    input: &[u8],
    header_len: usize,
    magic: &[u8; 8],
) -> Result<(), ParseFrameError> {
    if input.len() < header_len {
        return Err(ParseFrameError::InvalidFrameLength);
    }
    if input[..magic.len()] != *magic {
        return Err(ParseFrameError::InvalidMagic);
    }
    if read_u16(input, VERSION_OFFSET) != PROTOCOL_VERSION {
        return Err(ParseFrameError::UnsupportedVersion);
    }
    Ok(())
}

pub(super) fn encode_common_header(
    output: &mut [u8],
    magic: &[u8; 8],
    kind: u8,
    header_len: usize,
    body_len: usize,
    nonce: HandshakeNonce,
    source_digest: ContentDigest,
) -> Result<(), ParseFrameError> {
    output[..magic.len()].copy_from_slice(magic);
    write_u16(output, VERSION_OFFSET, PROTOCOL_VERSION);
    output[KIND_OFFSET] = kind;
    write_u32(
        output,
        HEADER_LEN_OFFSET,
        u32::try_from(header_len).map_err(|_| ParseFrameError::LengthOverflow)?,
    );
    write_u64(
        output,
        BODY_LEN_OFFSET,
        u64::try_from(body_len).map_err(|_| ParseFrameError::LengthOverflow)?,
    );
    output[NONCE_OFFSET..SOURCE_DIGEST_OFFSET].copy_from_slice(nonce.correlation_bytes());
    output[SOURCE_DIGEST_OFFSET..SOURCE_DIGEST_OFFSET + DIGEST_LEN]
        .copy_from_slice(source_digest.as_bytes());
    Ok(())
}

pub(super) fn allocate_frame(total: usize) -> Result<Vec<u8>, ParseFrameError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(total)
        .map_err(|_| ParseFrameError::AllocationFailed)?;
    Ok(output)
}

pub(super) fn checked_frame_len(
    header_len: usize,
    declared: u64,
) -> Result<usize, ParseFrameError> {
    let body_len = usize::try_from(declared).map_err(|_| ParseFrameError::LengthOverflow)?;
    header_len
        .checked_add(body_len)
        .ok_or(ParseFrameError::LengthOverflow)
}

pub(super) fn read_u16(input: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes(
        input[offset..offset + 2]
            .try_into()
            .expect("checked header"),
    )
}

pub(super) fn read_u32(input: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(
        input[offset..offset + 4]
            .try_into()
            .expect("checked header"),
    )
}

pub(super) fn read_u64(input: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(
        input[offset..offset + 8]
            .try_into()
            .expect("checked header"),
    )
}

pub(super) fn write_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn write_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn write_u64(output: &mut [u8], offset: usize, value: u64) {
    output[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
}
