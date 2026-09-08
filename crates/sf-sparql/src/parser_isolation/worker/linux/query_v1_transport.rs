//! One-shot real parser transport and separately gated parser-free evidence.

use crate::parser_isolation::parse_protocol::{
    allocate_frame_exact, decode_streamed_request_exact_for_nonce, FrameAllocation, ParseRequestV1,
    RequestHeaderV1, MAX_SOURCE_BYTES_V1, REQUEST_HEADER_LEN,
};
#[cfg(feature = "query-v1-transport-evidence")]
use crate::parser_isolation::parse_protocol::{
    synthetic_empty_ask_result_header_for, SYNTHETIC_EMPTY_ASK_QUERY_V1,
};
use crate::parser_isolation::parse_protocol::{ParseRejectionV1, ParseResultV1};
use crate::parser_isolation::profile::V1_CANDIDATE_MAX_INPUT_BYTES;
use crate::parser_isolation::protocol::{HandshakeNonce, FRAME_LEN};
use crate::parser_isolation::query_v1::{self, QueryWireError};

use super::write_all;
use super::{read_exact, require_parent_eof, WorkerFailure};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RequestConstraints {
    source_limit: usize,
    input_prefix_bytes: usize,
    allocation: FrameAllocation,
}

impl RequestConstraints {
    pub(super) const fn new(
        source_limit: usize,
        input_prefix_bytes: usize,
        allocation: FrameAllocation,
    ) -> Self {
        Self {
            source_limit,
            input_prefix_bytes,
            allocation,
        }
    }

    const fn normal() -> Self {
        Self::new(MAX_SOURCE_BYTES_V1, FRAME_LEN, FrameAllocation::Attempt)
    }
}

#[cfg(feature = "query-v1-transport-evidence")]
pub(super) fn run(expected_nonce: HandshakeNonce) -> Result<(), WorkerFailure> {
    run_with_validated_request(
        expected_nonce,
        RequestConstraints::normal(),
        emit_normal_result,
    )
}

pub(super) fn run_parser_query_v1(expected_nonce: HandshakeNonce) -> Result<(), WorkerFailure> {
    run_with_validated_request(
        expected_nonce,
        RequestConstraints::normal(),
        emit_parser_result,
    )
}

/// Own the one shared request receive/validate path for normal and mutant peers.
pub(super) fn run_with_validated_request(
    expected_nonce: HandshakeNonce,
    constraints: RequestConstraints,
    emit: impl FnOnce(&ParseRequestV1<'_>) -> Result<(), WorkerFailure>,
) -> Result<(), WorkerFailure> {
    let mut encoded_header = [0_u8; REQUEST_HEADER_LEN];
    read_exact(libc::STDIN_FILENO, &mut encoded_header)?;

    let header = RequestHeaderV1::preflight(&encoded_header).map_err(|_| WorkerFailure)?;
    header.enforce_body_limit().map_err(|_| WorkerFailure)?;
    if header.body_len() > constraints.source_limit
        || !request_fits_input_budget(header.frame_len(), constraints.input_prefix_bytes)
    {
        return Err(WorkerFailure);
    }

    // The sole request-sized allocation owns the complete header-plus-body
    // frame. Its initialized header prefix is overwritten before any decoder
    // observes it; no detached body or concatenation allocation exists.
    let mut encoded_request = allocate_frame_exact(header.frame_len(), constraints.allocation)
        .map_err(|_| WorkerFailure)?;
    encoded_request[..REQUEST_HEADER_LEN].copy_from_slice(&encoded_header);
    read_exact(
        libc::STDIN_FILENO,
        &mut encoded_request[REQUEST_HEADER_LEN..],
    )?;

    // Peer-controlled correlation, digest, and UTF-8 work is deferred until
    // the parent proves the one-shot request ended exactly at this frame.
    require_parent_eof()?;
    let request = decode_streamed_request_exact_for_nonce(
        &encoded_header,
        &encoded_request[REQUEST_HEADER_LEN..],
        expected_nonce,
    )
    .map_err(|_| WorkerFailure)?;

    emit(&request)
}

#[cfg(feature = "query-v1-transport-evidence")]
fn emit_normal_result(request: &ParseRequestV1<'_>) -> Result<(), WorkerFailure> {
    let result_header =
        synthetic_empty_ask_result_header_for(request).map_err(|_| WorkerFailure)?;
    write_all(libc::STDOUT_FILENO, &result_header)?;
    write_all(libc::STDOUT_FILENO, &SYNTHETIC_EMPTY_ASK_QUERY_V1)
}

fn emit_parser_result(request: &ParseRequestV1<'_>) -> Result<(), WorkerFailure> {
    let query = match spargebra::SparqlParser::new().parse_query(request.source()) {
        Ok(query) => query,
        Err(_) => {
            let rejection =
                ParseResultV1::encode_fixed_rejection_for(request, ParseRejectionV1::Syntax);
            return write_all(libc::STDOUT_FILENO, &rejection);
        }
    };

    let payload = match query_v1::encode(&query) {
        Ok(payload) => payload,
        Err(QueryWireError::AllocationFailed) => {
            let rejection = ParseResultV1::encode_fixed_rejection_for(
                request,
                ParseRejectionV1::ResourceExhausted,
            );
            return write_all(libc::STDOUT_FILENO, &rejection);
        }
        Err(QueryWireError::AccountingOverflow | QueryWireError::LimitExceeded(_)) => {
            let rejection =
                ParseResultV1::encode_fixed_rejection_for(request, ParseRejectionV1::QueryEnvelope);
            return write_all(libc::STDOUT_FILENO, &rejection);
        }
        Err(_) => return Err(WorkerFailure),
    };
    let result = ParseResultV1::encode_canonical_success_payload_for(request, &payload)
        .map_err(|_| WorkerFailure)?;
    write_all(libc::STDOUT_FILENO, &result)
}

fn request_fits_input_budget(frame_len: usize, input_prefix_bytes: usize) -> bool {
    input_prefix_bytes
        .checked_add(frame_len)
        .and_then(|total| u64::try_from(total).ok())
        .is_some_and(|total| total <= V1_CANDIDATE_MAX_INPUT_BYTES)
}

#[cfg(all(test, feature = "query-v1-transport-mutant-evidence"))]
mod tests {
    use super::*;
    use crate::parser_isolation::query_v1_mutant::{
        MAX_MUTANT_SOURCE_BYTES_V1, MUTANT_DIRECTIVE_LEN,
    };

    #[test]
    fn whole_life_input_budget_preserves_normal_and_reduces_mutant_source() {
        assert!(request_fits_input_budget(
            REQUEST_HEADER_LEN + MAX_SOURCE_BYTES_V1,
            FRAME_LEN
        ));
        assert!(request_fits_input_budget(
            REQUEST_HEADER_LEN + MAX_MUTANT_SOURCE_BYTES_V1,
            FRAME_LEN + MUTANT_DIRECTIVE_LEN
        ));
        assert!(!request_fits_input_budget(
            REQUEST_HEADER_LEN + MAX_MUTANT_SOURCE_BYTES_V1 + 1,
            FRAME_LEN + MUTANT_DIRECTIVE_LEN
        ));
    }
}
