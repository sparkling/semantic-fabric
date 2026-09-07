//! Closed same-executable failure behaviors for QueryV1 transport evidence.

use crate::parser_isolation::parse_protocol::{
    mutate_synthetic_result_header, synthetic_empty_ask_result_header_for,
    synthetic_mutant_result_header_for_body, synthetic_mutant_result_header_for_payload,
    FrameAllocation, ParseRequestV1, SyntheticResultHeaderMutation, SYNTHETIC_EMPTY_ASK_QUERY_V1,
};
use crate::parser_isolation::protocol::{HandshakeNonce, DIGEST_LEN, FRAME_LEN};
use crate::parser_isolation::query_v1::MAX_QUERY_WIRE_BYTES;
use crate::parser_isolation::query_v1_mutant::{
    QueryV1TransportMutant, MAX_MUTANT_SOURCE_BYTES_V1, MUTANT_DIRECTIVE_LEN,
};
use crate::parser_isolation::worker::PRIVATE_WORKER_REJECTED_EXIT_CODE;

use super::query_v1_transport::{run_with_validated_request, RequestConstraints};
use super::{
    exit_with_policy_owner_live, prepare_for_hello, raw_exit, read_exact,
    read_hello_and_emit_ready, write_all, WorkerFailure,
};

const ZERO_BODY_DIGEST: [u8; DIGEST_LEN] = [
    0x2d, 0xae, 0xb1, 0xf3, 0x60, 0x95, 0xb4, 0x4b, 0x31, 0x84, 0x10, 0xb3, 0xf4, 0xe8, 0xb5, 0xd9,
    0x89, 0xdc, 0xc7, 0xbb, 0x02, 0x3d, 0x14, 0x26, 0xc4, 0x92, 0xda, 0xb0, 0xa3, 0x05, 0x3e, 0x74,
];
const ZERO_CHUNK: [u8; 4096] = [0; 4096];
const TRAILING_BYTE: [u8; 1] = [0xa5];

pub(super) fn run_peer() -> ! {
    match prepare_for_hello() {
        Ok(prepared) => {
            let mut encoded_directive = [0_u8; MUTANT_DIRECTIVE_LEN];
            let result = read_exact(libc::STDIN_FILENO, &mut encoded_directive)
                .and_then(|_| {
                    QueryV1TransportMutant::decode(encoded_directive).ok_or(WorkerFailure)
                })
                .and_then(|mutant| {
                    read_hello_and_emit_ready(&prepared).and_then(|nonce| run(mutant, nonce))
                });
            exit_with_policy_owner_live(&prepared, result)
        }
        Err(_) => raw_exit(PRIVATE_WORKER_REJECTED_EXIT_CODE),
    }
}

pub(super) fn run(
    mutant: QueryV1TransportMutant,
    expected_nonce: HandshakeNonce,
) -> Result<(), WorkerFailure> {
    let allocation = if mutant == QueryV1TransportMutant::RequestFrameAllocationRefusal {
        FrameAllocation::Refuse
    } else {
        FrameAllocation::Attempt
    };
    let constraints = RequestConstraints::new(
        MAX_MUTANT_SOURCE_BYTES_V1,
        FRAME_LEN + MUTANT_DIRECTIVE_LEN,
        allocation,
    );
    run_with_validated_request(expected_nonce, constraints, |request| emit(mutant, request))
}

fn emit(mutant: QueryV1TransportMutant, request: &ParseRequestV1<'_>) -> Result<(), WorkerFailure> {
    match mutant {
        QueryV1TransportMutant::WrongNonceExitZero => {
            emit_mutated_static(request, SyntheticResultHeaderMutation::Nonce)
        }
        QueryV1TransportMutant::WrongSourceDigestExitZero => {
            emit_mutated_static(request, SyntheticResultHeaderMutation::SourceDigest)
        }
        QueryV1TransportMutant::WrongPayloadDigestExitZero => {
            emit_mutated_static(request, SyntheticResultHeaderMutation::PayloadDigest)
        }
        QueryV1TransportMutant::SelfConsistentDigestOverInvalidQueryV1ExitZero => {
            let mut invalid = SYNTHETIC_EMPTY_ASK_QUERY_V1;
            invalid[0] ^= 1;
            let header = synthetic_mutant_result_header_for_payload(request, &invalid)
                .map_err(|_| WorkerFailure)?;
            write_all(libc::STDOUT_FILENO, &header)?;
            write_all(libc::STDOUT_FILENO, &invalid)
        }
        QueryV1TransportMutant::WrongCorrelationThenExit78 => {
            emit_mutated_static(request, SyntheticResultHeaderMutation::Nonce)?;
            Err(WorkerFailure)
        }
        QueryV1TransportMutant::WrongCorrelationThenDeadlineStall => {
            emit_mutated_static(request, SyntheticResultHeaderMutation::Nonce)?;
            stall_on_private_futex()
        }
        QueryV1TransportMutant::TrailingOutput => {
            emit_valid_static(request)?;
            write_all(libc::STDOUT_FILENO, &TRAILING_BYTE)
        }
        QueryV1TransportMutant::ExactOutputCap => {
            let header = synthetic_mutant_result_header_for_body(
                request,
                MAX_QUERY_WIRE_BYTES,
                ZERO_BODY_DIGEST,
            )
            .map_err(|_| WorkerFailure)?;
            write_all(libc::STDOUT_FILENO, &header)?;
            stream_zero_body(MAX_QUERY_WIRE_BYTES)
        }
        QueryV1TransportMutant::OutputCapPlusOne => {
            let header = synthetic_mutant_result_header_for_body(
                request,
                MAX_QUERY_WIRE_BYTES + 1,
                ZERO_BODY_DIGEST,
            )
            .map_err(|_| WorkerFailure)?;
            write_all(libc::STDOUT_FILENO, &header)?;
            stall_on_private_futex()
        }
        // `run` selected the refusing allocator, so the shared request path
        // must return before invoking this callback or emitting result bytes.
        QueryV1TransportMutant::RequestFrameAllocationRefusal => Err(WorkerFailure),
    }
}

fn emit_valid_static(request: &ParseRequestV1<'_>) -> Result<(), WorkerFailure> {
    let header = synthetic_empty_ask_result_header_for(request).map_err(|_| WorkerFailure)?;
    write_all(libc::STDOUT_FILENO, &header)?;
    write_all(libc::STDOUT_FILENO, &SYNTHETIC_EMPTY_ASK_QUERY_V1)
}

fn emit_mutated_static(
    request: &ParseRequestV1<'_>,
    mutation: SyntheticResultHeaderMutation,
) -> Result<(), WorkerFailure> {
    let mut header = synthetic_empty_ask_result_header_for(request).map_err(|_| WorkerFailure)?;
    mutate_synthetic_result_header(&mut header, mutation);
    write_all(libc::STDOUT_FILENO, &header)?;
    write_all(libc::STDOUT_FILENO, &SYNTHETIC_EMPTY_ASK_QUERY_V1)
}

fn stream_zero_body(mut remaining: usize) -> Result<(), WorkerFailure> {
    while remaining >= ZERO_CHUNK.len() {
        write_all(libc::STDOUT_FILENO, &ZERO_CHUNK)?;
        remaining -= ZERO_CHUNK.len();
    }
    write_all(libc::STDOUT_FILENO, &ZERO_CHUNK[..remaining])
}

fn stall_on_private_futex() -> ! {
    let futex_word = 0_u32;
    loop {
        // A private wait with no timeout blocks without consuming the worker's
        // CPU allowance. Signals or spurious wakeups simply re-enter the wait;
        // the parent remains the sole owner of wall-deadline containment.
        unsafe {
            libc::syscall(
                libc::SYS_futex,
                std::ptr::addr_of!(futex_word),
                libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
                0_u32,
                std::ptr::null::<libc::timespec>(),
                std::ptr::null::<u32>(),
                0_u32,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    #[test]
    fn fixed_zero_digest_matches_exact_eight_mib_stream() {
        let mut digest = Sha256::new();
        for _ in 0..MAX_QUERY_WIRE_BYTES / ZERO_CHUNK.len() {
            digest.update(ZERO_CHUNK);
        }
        assert_eq!(digest.finalize().as_slice(), ZERO_BODY_DIGEST);
        assert_eq!(MAX_QUERY_WIRE_BYTES % ZERO_CHUNK.len(), 0);
    }
}
