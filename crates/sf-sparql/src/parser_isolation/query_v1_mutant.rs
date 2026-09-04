//! Closed control vocabulary for same-executable QueryV1 transport mutants.

use super::parse_protocol::MAX_SOURCE_BYTES_V1;

pub(crate) const MUTANT_DIRECTIVE_LEN: usize = 2;
pub(crate) const MAX_MUTANT_SOURCE_BYTES_V1: usize = MAX_SOURCE_BYTES_V1 - MUTANT_DIRECTIVE_LEN;

/// The complete, non-extensible set of synthetic transport failures.
///
/// This type exists only with `query-v1-transport-mutant-evidence`; the normal
/// transport API has no selector and cannot name or encode these behaviors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum QueryV1TransportMutant {
    WrongNonceExitZero = 1,
    WrongSourceDigestExitZero = 2,
    WrongPayloadDigestExitZero = 3,
    SelfConsistentDigestOverInvalidQueryV1ExitZero = 4,
    WrongCorrelationThenExit78 = 5,
    WrongCorrelationThenDeadlineStall = 6,
    TrailingOutput = 7,
    ExactOutputCap = 8,
    OutputCapPlusOne = 9,
    RequestFrameAllocationRefusal = 10,
}

impl QueryV1TransportMutant {
    pub(crate) const ALL: [Self; 10] = [
        Self::WrongNonceExitZero,
        Self::WrongSourceDigestExitZero,
        Self::WrongPayloadDigestExitZero,
        Self::SelfConsistentDigestOverInvalidQueryV1ExitZero,
        Self::WrongCorrelationThenExit78,
        Self::WrongCorrelationThenDeadlineStall,
        Self::TrailingOutput,
        Self::ExactOutputCap,
        Self::OutputCapPlusOne,
        Self::RequestFrameAllocationRefusal,
    ];

    pub(crate) const fn encode(self) -> [u8; MUTANT_DIRECTIVE_LEN] {
        (self as u16).to_be_bytes()
    }

    pub(crate) fn decode(encoded: [u8; MUTANT_DIRECTIVE_LEN]) -> Option<Self> {
        match u16::from_be_bytes(encoded) {
            1 => Some(Self::WrongNonceExitZero),
            2 => Some(Self::WrongSourceDigestExitZero),
            3 => Some(Self::WrongPayloadDigestExitZero),
            4 => Some(Self::SelfConsistentDigestOverInvalidQueryV1ExitZero),
            5 => Some(Self::WrongCorrelationThenExit78),
            6 => Some(Self::WrongCorrelationThenDeadlineStall),
            7 => Some(Self::TrailingOutput),
            8 => Some(Self::ExactOutputCap),
            9 => Some(Self::OutputCapPlusOne),
            10 => Some(Self::RequestFrameAllocationRefusal),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directive_is_exact_closed_u16_discriminant() {
        for mutant in QueryV1TransportMutant::ALL {
            let encoded = mutant.encode();
            assert_eq!(encoded.len(), MUTANT_DIRECTIVE_LEN);
            assert_eq!(QueryV1TransportMutant::decode(encoded), Some(mutant));
        }
        for unknown in [0_u16, 11, u16::MAX] {
            assert_eq!(QueryV1TransportMutant::decode(unknown.to_be_bytes()), None);
        }
    }

    #[test]
    fn directive_reduces_only_the_mutant_source_ceiling() {
        assert_eq!(MAX_SOURCE_BYTES_V1, 1_048_576);
        assert_eq!(MUTANT_DIRECTIVE_LEN, 2);
        assert_eq!(MAX_MUTANT_SOURCE_BYTES_V1, 1_048_574);
    }
}
