//! Immutable control-ready candidate inputs shared by parent and child.

use sha2::{Digest, Sha256};

use super::parse_protocol::{MAX_SOURCE_BYTES_V1, REQUEST_HEADER_LEN, RESULT_HEADER_LEN};
use super::protocol::FRAME_LEN;
use super::protocol::{ParserProfileDigest, ParserWorkerLimitValues, ParserWorkerLimits};
use super::query_v1::MAX_QUERY_WIRE_BYTES;

pub(super) const V1_CANDIDATE_MAX_INPUT_BYTES: u64 =
    (FRAME_LEN + REQUEST_HEADER_LEN + MAX_SOURCE_BYTES_V1) as u64;
pub(super) const V1_CANDIDATE_MAX_OUTPUT_BYTES: u64 =
    (FRAME_LEN + RESULT_HEADER_LEN + MAX_QUERY_WIRE_BYTES) as u64;

/// Separate regular-file ceiling; this does not govern worker pipe output.
pub(super) const V1_CANDIDATE_RLIMIT_FSIZE_BYTES: u64 = 64 * 1024 * 1024;

const _: () = {
    assert!(V1_CANDIDATE_MAX_INPUT_BYTES == 1_048_856);
    assert!(V1_CANDIDATE_MAX_OUTPUT_BYTES == 8_388_920);
    assert!(V1_CANDIDATE_RLIMIT_FSIZE_BYTES == 67_108_864);
};

/// Partial parser-worker identity used only to correlate the evidence seam.
///
/// This wrapper is deliberately not a governed parser profile and has no
/// conversion into a future governed-profile identity. Equality only says that
/// two evidence observations used the same provisional material. It cannot
/// qualify a receipt for ADR-0053 gate 6.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ParserWorkerEvidenceProfileV1(ParserProfileDigest);

impl ParserWorkerEvidenceProfileV1 {
    pub(super) const fn new(digest: ParserProfileDigest) -> Self {
        Self(digest)
    }
}

pub(super) const V1_LIMIT_VALUES: ParserWorkerLimitValues = ParserWorkerLimitValues {
    stack_bytes: 16 * 1024 * 1024,
    address_space_bytes: 1024 * 1024 * 1024,
    cpu_time_millis: 10_000,
    wall_time_millis: 15_000,
    max_input_bytes: V1_CANDIDATE_MAX_INPUT_BYTES,
    max_output_bytes: V1_CANDIDATE_MAX_OUTPUT_BYTES,
    max_open_fds: 64,
    max_processes: 1,
    max_concurrency: 64,
};

/// Partial grammar-source marker for the control-ready evidence exchange.
///
/// This is not the governed parser/compile profile, a complete resolved runtime
/// dependency-and-feature closure, a signature, or release attestation. QueryV1
/// activation must replace it with the complete profile required by ADR-0053.
/// The source-lock test only prevents these selected grammar sources and direct
/// workspace features from drifting silently during the control-only slice.
const CONTROL_READY_PROFILE_CANDIDATE_V1_MATERIAL: &[u8] =
    b"semantic-fabric/control-ready-profile-candidate/v1\n\
target=x86_64-unknown-linux-gnu\n\
spargebra=0.4.6;checksum=46715eb957d1fe960cbbc0b713da8f78e2cb19df315b48fb09c9467a1c84f656\n\
features=sep-0002,sep-0006,sparql-12,standard-unicode-escaping\n\
peg=0.8.6;checksum=0aad070be5b63aa72103f2fcdd70a83adbd5e90112ce5b574171ff1c65501773\n\
peg-macros=0.8.6;checksum=ddd8ef6825cae95355031ae26a99b616a2a21f22ba2de0197c43dfb05acbe7ee\n\
peg-runtime=0.8.6;checksum=7011d97b484a5ebdc4b1fdb3b12d5e4bbbea56e9d22b688f2e79e04b65a7d8a6\n";

pub(super) fn v1_limits() -> ParserWorkerLimits {
    ParserWorkerLimits::new(V1_LIMIT_VALUES).expect("the fixed V1 limits are valid")
}

pub(super) fn parser_worker_evidence_profile_v1() -> ParserWorkerEvidenceProfileV1 {
    ParserWorkerEvidenceProfileV1::new(control_ready_profile_candidate_digest())
}

/// The digest retained for the existing control-ready handshake only.
///
/// It remains a partial grammar-source marker, not a complete governed parser
/// profile. `ParserWorkerEvidenceProfileV1` wraps this material only for the
/// provisional, feature-gated correlation seam.
pub(super) fn control_ready_profile_candidate_digest() -> ParserProfileDigest {
    ParserProfileDigest::new(Sha256::digest(CONTROL_READY_PROFILE_CANDIDATE_V1_MATERIAL).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKSPACE_MANIFEST: &str = include_str!("../../../../Cargo.toml");
    const WORKSPACE_LOCK: &str = include_str!("../../../../Cargo.lock");

    #[test]
    fn control_ready_marker_is_bound_to_selected_sources_and_features() {
        assert!(WORKSPACE_MANIFEST.contains(
            "spargebra = { version = \"=0.4.6\", features = [\"sparql-12\", \"sep-0002\", \"sep-0006\", \"standard-unicode-escaping\"] }"
        ));
        assert!(
            std::str::from_utf8(CONTROL_READY_PROFILE_CANDIDATE_V1_MATERIAL)
                .unwrap()
                .contains("target=x86_64-unknown-linux-gnu\n")
        );
        for locked_identity in [
            "name = \"spargebra\"\nversion = \"0.4.6\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"46715eb957d1fe960cbbc0b713da8f78e2cb19df315b48fb09c9467a1c84f656\"",
            "name = \"peg\"\nversion = \"0.8.6\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"0aad070be5b63aa72103f2fcdd70a83adbd5e90112ce5b574171ff1c65501773\"",
            "name = \"peg-macros\"\nversion = \"0.8.6\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"ddd8ef6825cae95355031ae26a99b616a2a21f22ba2de0197c43dfb05acbe7ee\"",
            "name = \"peg-runtime\"\nversion = \"0.8.6\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"7011d97b484a5ebdc4b1fdb3b12d5e4bbbea56e9d22b688f2e79e04b65a7d8a6\"",
        ] {
            assert!(WORKSPACE_LOCK.contains(locked_identity));
        }
        assert_ne!(
            control_ready_profile_candidate_digest(),
            ParserProfileDigest::new([0; 32])
        );
    }

    #[test]
    fn transport_caps_include_both_handshake_and_frame_headers() {
        assert_eq!(
            V1_CANDIDATE_MAX_INPUT_BYTES,
            (FRAME_LEN + REQUEST_HEADER_LEN + MAX_SOURCE_BYTES_V1) as u64
        );
        assert_eq!(
            V1_CANDIDATE_MAX_OUTPUT_BYTES,
            (FRAME_LEN + RESULT_HEADER_LEN + MAX_QUERY_WIRE_BYTES) as u64
        );
        assert_ne!(
            V1_LIMIT_VALUES.max_output_bytes,
            V1_CANDIDATE_RLIMIT_FSIZE_BYTES
        );
    }
}
