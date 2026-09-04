//! Fixed-size parser-worker handshake codec.
//!
//! Every field has one canonical big-endian representation. Decoding performs
//! no input-sized allocation and errors carry only closed categories, never
//! peer-controlled bytes or values.

use std::fmt;

pub(crate) const DIGEST_LEN: usize = 32;
pub(crate) const FRAME_LEN: usize = 184;

const MAGIC: [u8; 8] = *b"SFPWHS01";
const VERSION: u16 = 1;
const FRAME_LEN_U32: u32 = 184;
const HELLO_KIND: u8 = 1;
const READY_KIND: u8 = 2;
const HEADER_LEN: usize = 16;
const NONCE_OFFSET: usize = HEADER_LEN;
const BUILD_ID_OFFSET: usize = NONCE_OFFSET + DIGEST_LEN;
const PROFILE_ID_OFFSET: usize = BUILD_ID_OFFSET + DIGEST_LEN;
const LIMITS_OFFSET: usize = PROFILE_ID_OFFSET + DIGEST_LEN;
const LIMIT_FIELD_COUNT: usize = 9;
const LIMIT_WIDTH: usize = 8;
// Keep resources finite in a signed platform quantity and count-like limits
// losslessly convertible to the narrow counters used by later supervisors.
const MAX_RESOURCE_LIMIT: u64 = i64::MAX as u64;
const MAX_COUNT_LIMIT: u64 = u32::MAX as u64;

const _: () = {
    assert!(LIMITS_OFFSET + LIMIT_FIELD_COUNT * LIMIT_WIDTH == FRAME_LEN);
    assert!(FRAME_LEN_U32 as usize == FRAME_LEN);
};

macro_rules! fixed_bytes_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Eq, PartialEq)]
        pub(crate) struct $name([u8; DIGEST_LEN]);

        impl $name {
            pub(crate) const fn new(bytes: [u8; DIGEST_LEN]) -> Self {
                Self(bytes)
            }

            const fn as_bytes(&self) -> &[u8; DIGEST_LEN] {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(<redacted>)"))
            }
        }
    };
}

fixed_bytes_type!(HandshakeNonce);
fixed_bytes_type!(BuildIdentityDigest);
fixed_bytes_type!(ParserProfileDigest);

/// Unvalidated scalar values used only at the construction/decoding boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ParserWorkerLimitValues {
    pub(crate) stack_bytes: u64,
    pub(crate) address_space_bytes: u64,
    pub(crate) cpu_time_millis: u64,
    pub(crate) wall_time_millis: u64,
    pub(crate) max_input_bytes: u64,
    pub(crate) max_output_bytes: u64,
    pub(crate) max_open_fds: u64,
    pub(crate) max_processes: u64,
    pub(crate) max_concurrency: u64,
}

impl ParserWorkerLimitValues {
    fn as_array(self) -> [u64; LIMIT_FIELD_COUNT] {
        [
            self.stack_bytes,
            self.address_space_bytes,
            self.cpu_time_millis,
            self.wall_time_millis,
            self.max_input_bytes,
            self.max_output_bytes,
            self.max_open_fds,
            self.max_processes,
            self.max_concurrency,
        ]
    }

    fn from_array(values: [u64; LIMIT_FIELD_COUNT]) -> Self {
        Self {
            stack_bytes: values[0],
            address_space_bytes: values[1],
            cpu_time_millis: values[2],
            wall_time_millis: values[3],
            max_input_bytes: values[4],
            max_output_bytes: values[5],
            max_open_fds: values[6],
            max_processes: values[7],
            max_concurrency: values[8],
        }
    }
}

/// Limits whose finite, positive wire representation has been validated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ParserWorkerLimits(ParserWorkerLimitValues);

impl ParserWorkerLimits {
    pub(crate) fn new(values: ParserWorkerLimitValues) -> Result<Self, HandshakeError> {
        let fields = values.as_array();
        for (index, value) in fields.into_iter().enumerate() {
            let field = LimitField::from_index(index);
            let maximum = if index < 6 {
                MAX_RESOURCE_LIMIT
            } else {
                MAX_COUNT_LIMIT
            };
            if value == 0 || value > maximum {
                return Err(HandshakeError::InvalidLimit(field));
            }
        }
        if values.stack_bytes > values.address_space_bytes {
            return Err(HandshakeError::InvalidLimit(LimitField::AddressSpaceBytes));
        }
        Ok(Self(values))
    }

    pub(crate) const fn values(self) -> ParserWorkerLimitValues {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HelloFrame {
    payload: HandshakePayload,
}

impl HelloFrame {
    pub(crate) const fn new(
        nonce: HandshakeNonce,
        build_identity: BuildIdentityDigest,
        parser_profile: ParserProfileDigest,
        requested_limits: ParserWorkerLimits,
    ) -> Self {
        Self {
            payload: HandshakePayload {
                nonce,
                build_identity,
                parser_profile,
                limits: requested_limits,
            },
        }
    }

    pub(crate) fn encode(self) -> [u8; FRAME_LEN] {
        encode_frame(HELLO_KIND, self.payload)
    }

    pub(crate) fn decode(input: &[u8]) -> Result<Self, HandshakeError> {
        decode_frame(input, HELLO_KIND).map(|payload| Self { payload })
    }

    pub(crate) const fn nonce(self) -> HandshakeNonce {
        self.payload.nonce
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReadyFrame {
    payload: HandshakePayload,
}

impl ReadyFrame {
    pub(crate) const fn new(
        nonce: HandshakeNonce,
        build_identity: BuildIdentityDigest,
        parser_profile: ParserProfileDigest,
        effective_limits: ParserWorkerLimits,
    ) -> Self {
        Self {
            payload: HandshakePayload {
                nonce,
                build_identity,
                parser_profile,
                limits: effective_limits,
            },
        }
    }

    pub(crate) fn encode(self) -> [u8; FRAME_LEN] {
        encode_frame(READY_KIND, self.payload)
    }

    pub(crate) fn decode(input: &[u8]) -> Result<Self, HandshakeError> {
        decode_frame(input, READY_KIND).map(|payload| Self { payload })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HandshakePayload {
    nonce: HandshakeNonce,
    build_identity: BuildIdentityDigest,
    parser_profile: ParserProfileDigest,
    limits: ParserWorkerLimits,
}

/// Verifies that `Ready` acknowledges the exact request identity and limits.
pub(crate) fn verify_ready(hello: &HelloFrame, ready: &ReadyFrame) -> Result<(), HandshakeError> {
    if hello.payload.nonce != ready.payload.nonce {
        return Err(HandshakeError::NonceMismatch);
    }
    if hello.payload.build_identity != ready.payload.build_identity {
        return Err(HandshakeError::BuildIdentityMismatch);
    }
    if hello.payload.parser_profile != ready.payload.parser_profile {
        return Err(HandshakeError::ParserProfileMismatch);
    }
    if hello.payload.limits != ready.payload.limits {
        return Err(HandshakeError::EffectiveLimitsMismatch);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LimitField {
    StackBytes,
    AddressSpaceBytes,
    CpuTimeMillis,
    WallTimeMillis,
    MaxInputBytes,
    MaxOutputBytes,
    MaxOpenFds,
    MaxProcesses,
    MaxConcurrency,
}

impl LimitField {
    const fn from_index(index: usize) -> Self {
        match index {
            0 => Self::StackBytes,
            1 => Self::AddressSpaceBytes,
            2 => Self::CpuTimeMillis,
            3 => Self::WallTimeMillis,
            4 => Self::MaxInputBytes,
            5 => Self::MaxOutputBytes,
            6 => Self::MaxOpenFds,
            7 => Self::MaxProcesses,
            8 => Self::MaxConcurrency,
            _ => unreachable!(),
        }
    }
}

impl fmt::Display for LimitField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::StackBytes => "stack-byte",
            Self::AddressSpaceBytes => "address-space-byte",
            Self::CpuTimeMillis => "CPU-time",
            Self::WallTimeMillis => "wall-time",
            Self::MaxInputBytes => "input-byte",
            Self::MaxOutputBytes => "output-byte",
            Self::MaxOpenFds => "open-FD",
            Self::MaxProcesses => "process-count",
            Self::MaxConcurrency => "concurrency",
        })
    }
}

/// Closed, non-reflective failures for handshake decoding and verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HandshakeError {
    InvalidFrameLength,
    InvalidMagic,
    UnsupportedVersion,
    UnsupportedMessageKind,
    UnexpectedMessageKind,
    NonCanonicalHeader,
    InvalidLimit(LimitField),
    NonceMismatch,
    BuildIdentityMismatch,
    ParserProfileMismatch,
    EffectiveLimitsMismatch,
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFrameLength => {
                formatter.write_str("invalid parser-worker handshake frame length")
            }
            Self::InvalidMagic => formatter.write_str("invalid parser-worker handshake magic"),
            Self::UnsupportedVersion => {
                formatter.write_str("unsupported parser-worker handshake version")
            }
            Self::UnsupportedMessageKind => {
                formatter.write_str("unsupported parser-worker handshake message kind")
            }
            Self::UnexpectedMessageKind => {
                formatter.write_str("unexpected parser-worker handshake message kind")
            }
            Self::NonCanonicalHeader => {
                formatter.write_str("non-canonical parser-worker handshake header")
            }
            Self::InvalidLimit(field) => {
                write!(formatter, "invalid parser-worker {field} limit")
            }
            Self::NonceMismatch => formatter.write_str("parser-worker nonce mismatch"),
            Self::BuildIdentityMismatch => {
                formatter.write_str("parser-worker build identity mismatch")
            }
            Self::ParserProfileMismatch => {
                formatter.write_str("parser-worker parser profile mismatch")
            }
            Self::EffectiveLimitsMismatch => {
                formatter.write_str("parser-worker effective limits mismatch")
            }
        }
    }
}

impl std::error::Error for HandshakeError {}

fn encode_frame(kind: u8, payload: HandshakePayload) -> [u8; FRAME_LEN] {
    let mut output = [0_u8; FRAME_LEN];
    output[..MAGIC.len()].copy_from_slice(&MAGIC);
    output[8..10].copy_from_slice(&VERSION.to_be_bytes());
    output[10] = kind;
    output[11] = 0;
    output[12..16].copy_from_slice(&FRAME_LEN_U32.to_be_bytes());
    output[NONCE_OFFSET..BUILD_ID_OFFSET].copy_from_slice(payload.nonce.as_bytes());
    output[BUILD_ID_OFFSET..PROFILE_ID_OFFSET].copy_from_slice(payload.build_identity.as_bytes());
    output[PROFILE_ID_OFFSET..LIMITS_OFFSET].copy_from_slice(payload.parser_profile.as_bytes());
    for (index, value) in payload.limits.values().as_array().into_iter().enumerate() {
        let offset = LIMITS_OFFSET + index * LIMIT_WIDTH;
        output[offset..offset + LIMIT_WIDTH].copy_from_slice(&value.to_be_bytes());
    }
    output
}

fn decode_frame(input: &[u8], expected_kind: u8) -> Result<HandshakePayload, HandshakeError> {
    if input.len() != FRAME_LEN {
        return Err(HandshakeError::InvalidFrameLength);
    }
    if input[..MAGIC.len()] != MAGIC {
        return Err(HandshakeError::InvalidMagic);
    }
    if read_u16(input, 8) != VERSION {
        return Err(HandshakeError::UnsupportedVersion);
    }
    match input[10] {
        HELLO_KIND | READY_KIND if input[10] != expected_kind => {
            return Err(HandshakeError::UnexpectedMessageKind);
        }
        HELLO_KIND | READY_KIND => {}
        _ => return Err(HandshakeError::UnsupportedMessageKind),
    }
    if input[11] != 0 || read_u32(input, 12) != FRAME_LEN_U32 {
        return Err(HandshakeError::NonCanonicalHeader);
    }

    let mut nonce = [0_u8; DIGEST_LEN];
    nonce.copy_from_slice(&input[NONCE_OFFSET..BUILD_ID_OFFSET]);
    let mut build_identity = [0_u8; DIGEST_LEN];
    build_identity.copy_from_slice(&input[BUILD_ID_OFFSET..PROFILE_ID_OFFSET]);
    let mut parser_profile = [0_u8; DIGEST_LEN];
    parser_profile.copy_from_slice(&input[PROFILE_ID_OFFSET..LIMITS_OFFSET]);

    let mut limit_values = [0_u64; LIMIT_FIELD_COUNT];
    for (index, value) in limit_values.iter_mut().enumerate() {
        *value = read_u64(input, LIMITS_OFFSET + index * LIMIT_WIDTH);
    }
    let limits = ParserWorkerLimits::new(ParserWorkerLimitValues::from_array(limit_values))?;

    Ok(HandshakePayload {
        nonce: HandshakeNonce::new(nonce),
        build_identity: BuildIdentityDigest::new(build_identity),
        parser_profile: ParserProfileDigest::new(parser_profile),
        limits,
    })
}

fn read_u16(input: &[u8], offset: usize) -> u16 {
    let mut bytes = [0_u8; 2];
    bytes.copy_from_slice(&input[offset..offset + 2]);
    u16::from_be_bytes(bytes)
}

fn read_u32(input: &[u8], offset: usize) -> u32 {
    let mut bytes = [0_u8; 4];
    bytes.copy_from_slice(&input[offset..offset + 4]);
    u32::from_be_bytes(bytes)
}

fn read_u64(input: &[u8], offset: usize) -> u64 {
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&input[offset..offset + 8]);
    u64::from_be_bytes(bytes)
}
