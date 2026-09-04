//! Parser-free, one-shot QueryV1 transport evidence child.

use crate::parser_isolation::parse_protocol::{
    allocate_frame_exact, decode_streamed_request_exact_for_nonce,
    synthetic_empty_ask_result_header_for, FrameAllocation, RequestHeaderV1, REQUEST_HEADER_LEN,
    SYNTHETIC_EMPTY_ASK_QUERY_V1,
};
use crate::parser_isolation::protocol::HandshakeNonce;

use super::{read_exact, require_parent_eof, write_all, WorkerFailure};

pub(super) fn run(expected_nonce: HandshakeNonce) -> Result<(), WorkerFailure> {
    let mut encoded_header = [0_u8; REQUEST_HEADER_LEN];
    read_exact(libc::STDIN_FILENO, &mut encoded_header)?;

    let header = RequestHeaderV1::preflight(&encoded_header).map_err(|_| WorkerFailure)?;
    header.enforce_body_limit().map_err(|_| WorkerFailure)?;

    // The sole request-sized allocation owns the complete header-plus-body
    // frame. Its initialized header prefix is overwritten before any decoder
    // observes it; no detached body or concatenation allocation exists.
    let mut encoded_request = allocate_frame_exact(header.frame_len(), FrameAllocation::Attempt)
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

    let result_header =
        synthetic_empty_ask_result_header_for(&request).map_err(|_| WorkerFailure)?;
    write_all(libc::STDOUT_FILENO, &result_header)?;
    write_all(libc::STDOUT_FILENO, &SYNTHETIC_EMPTY_ASK_QUERY_V1)
}
