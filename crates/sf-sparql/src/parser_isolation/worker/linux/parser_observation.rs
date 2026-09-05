//! One-shot real-parser observation for the sealed qualification corpus.

use spargebra::SparqlParser;

use crate::parser_isolation::parse_protocol::{FrameAllocation, ParseRequestV1};
use crate::parser_isolation::parser_observation::{
    encode_observation, sealed_expected_outcome, ParserObservationOutcomeV1,
};
use crate::parser_isolation::protocol::{HandshakeNonce, FRAME_LEN};

use super::query_v1_transport::{run_with_validated_request, RequestConstraints};
use super::{write_all, WorkerFailure};
use crate::parser_isolation::parse_protocol::MAX_SOURCE_BYTES_V1;

pub(super) fn run(expected_nonce: HandshakeNonce) -> Result<(), WorkerFailure> {
    run_with_validated_request(
        expected_nonce,
        RequestConstraints::new(MAX_SOURCE_BYTES_V1, FRAME_LEN, FrameAllocation::Attempt),
        observe_parse,
    )
}

fn observe_parse(request: &ParseRequestV1<'_>) -> Result<(), WorkerFailure> {
    // Membership is checked before the parser sees bytes. This peer cannot be
    // repurposed into an arbitrary private parser service.
    sealed_expected_outcome(request.source()).ok_or(WorkerFailure)?;
    let outcome = if SparqlParser::new().parse_query(request.source()).is_ok() {
        ParserObservationOutcomeV1::Parsed
    } else {
        ParserObservationOutcomeV1::SyntaxRejected
    };
    let frame = encode_observation(request.nonce(), request.source(), outcome);
    write_all(libc::STDOUT_FILENO, &frame)
}
