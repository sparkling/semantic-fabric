//! Parent sequencing for the parser-free synthetic QueryV1 transport.

use super::handshake::ControlReadyWorker;
use super::lifecycle::ParserWorkerProcess;
use super::SupervisorError;
use crate::parser_isolation::parse_protocol::{
    allocate_frame_exact, FrameAllocation, ParseFrameError, ParseRequestV1, ParseResultV1,
    PreparedParseRequestV1, ResultHeaderV1, RESULT_HEADER_LEN, SYNTHETIC_EMPTY_ASK_QUERY_V1,
};
use crate::parser_isolation::protocol::FRAME_LEN;
use crate::parser_isolation::query_v1;

pub(super) fn finish(worker: ControlReadyWorker) -> Result<(), SupervisorError> {
    let ControlReadyWorker {
        mut process,
        prepared_request,
    } = worker;

    process.write_all_until_deadline(prepared_request.encoded())?;
    process.close_stdin();

    let mut encoded_header = [0_u8; RESULT_HEADER_LEN];
    process.read_exact_until_deadline(&mut encoded_header)?;
    let header = contain_frame_result(&mut process, ResultHeaderV1::preflight(&encoded_header))?;

    // Charge the peer's declared body against Ready plus the fixed header
    // before either trusting that declaration or allocating storage for it.
    process.ensure_can_receive(header.body_len())?;
    contain_frame_result(&mut process, header.enforce_body_limit())?;

    // The only result-frame allocation owns the complete header-plus-body
    // bytes. No detached body or later capacity growth is permitted.
    let mut encoded_result = contain_frame_result(
        &mut process,
        allocate_frame_exact(header.frame_len(), FrameAllocation::Attempt),
    )?;
    encoded_result[..RESULT_HEADER_LEN].copy_from_slice(&encoded_header);
    process.read_exact_until_deadline(&mut encoded_result[RESULT_HEADER_LEN..])?;
    process.expect_stdout_eof_until_deadline()?;

    // wait_until_deadline performs pidfd waitability inspection, sweeps the
    // process group while its leader is unreaped, and then reaps the exact
    // Child. Peer-controlled semantics remain untouched until this succeeds.
    let status = process.wait_until_deadline()?;
    if !status.success() {
        return Err(SupervisorError::InvalidState(
            "QueryV1 transport peer did not exit successfully",
        ));
    }

    verify_reaped_accounting(&process, &prepared_request)?;
    verify_reaped_result(&prepared_request, &encoded_result)
}

fn contain_frame_result<T>(
    process: &mut ParserWorkerProcess,
    result: Result<T, ParseFrameError>,
) -> Result<T, SupervisorError> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => Err(process.contain_live_failure(error.into())),
    }
}

fn verify_reaped_accounting(
    process: &ParserWorkerProcess,
    prepared_request: &PreparedParseRequestV1,
) -> Result<(), SupervisorError> {
    let expected_input = FRAME_LEN
        .checked_add(prepared_request.encoded().len())
        .and_then(|total| u64::try_from(total).ok())
        .ok_or(SupervisorError::InvalidState(
            "QueryV1 transport input accounting overflowed",
        ))?;
    let expected_output = FRAME_LEN
        .checked_add(RESULT_HEADER_LEN)
        .and_then(|total| total.checked_add(SYNTHETIC_EMPTY_ASK_QUERY_V1.len()))
        .and_then(|total| u64::try_from(total).ok())
        .ok_or(SupervisorError::InvalidState(
            "QueryV1 transport output accounting overflowed",
        ))?;
    if process.sent_bytes() != expected_input || process.received_bytes() != expected_output {
        return Err(SupervisorError::InvalidState(
            "QueryV1 transport lifetime byte accounting drifted",
        ));
    }
    Ok(())
}

/// Replays request and result semantics only after the caller has completed a
/// successful exact reap. The decoded `Query` remains local and is dropped
/// before this evidence-only function returns unit.
fn verify_reaped_result(
    prepared_request: &PreparedParseRequestV1,
    encoded_result: &[u8],
) -> Result<(), SupervisorError> {
    prepared_request.verify_exact()?;
    let request = ParseRequestV1::decode_exact_for_nonce(
        prepared_request.encoded(),
        prepared_request.nonce(),
    )?;
    let result = ParseResultV1::decode_exact_for(encoded_result, &request)?;
    let query = result.query().ok_or(SupervisorError::InvalidState(
        "QueryV1 transport peer returned a rejection",
    ))?;
    let replayed_payload =
        query_v1::encode(query).map_err(|_| ParseFrameError::InvalidQueryPayload)?;
    let received_payload = &encoded_result[RESULT_HEADER_LEN..];
    if replayed_payload.as_slice() != received_payload {
        return Err(SupervisorError::InvalidState(
            "QueryV1 transport payload is not an exact direct replay",
        ));
    }
    if replayed_payload.as_slice() != SYNTHETIC_EMPTY_ASK_QUERY_V1 {
        return Err(SupervisorError::InvalidState(
            "QueryV1 transport payload differs from the fixed fixture",
        ));
    }

    // `result` owns the Query and falls out of scope here; no AST escapes the
    // private evidence boundary.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser_isolation::parse_protocol::synthetic_empty_ask_result_header_for;
    use crate::parser_isolation::protocol::HandshakeNonce;
    use spargebra::algebra::GraphPattern;
    use spargebra::term::{TriplePattern, Variable};
    use spargebra::Query;

    #[test]
    fn post_reap_replay_accepts_only_the_static_query_fixture() {
        let nonce = HandshakeNonce::new([7; 32]);
        let prepared = PreparedParseRequestV1::new(nonce, "unparsed evidence source").unwrap();
        let request =
            ParseRequestV1::decode_exact_for_nonce(prepared.encoded(), prepared.nonce()).unwrap();
        let header = synthetic_empty_ask_result_header_for(&request).unwrap();
        let mut encoded = Vec::from(header);
        encoded.extend_from_slice(&SYNTHETIC_EMPTY_ASK_QUERY_V1);

        verify_reaped_result(&prepared, &encoded).expect("fixed result must replay exactly");

        encoded[RESULT_HEADER_LEN] ^= 1;
        assert!(matches!(
            verify_reaped_result(&prepared, &encoded),
            Err(SupervisorError::ParseFrame(
                ParseFrameError::PayloadDigestMismatch
            ))
        ));

        let alternate_query = Query::Ask {
            dataset: None,
            pattern: GraphPattern::Bgp {
                patterns: vec![TriplePattern {
                    subject: Variable::new_unchecked("subject").into(),
                    predicate: Variable::new_unchecked("predicate").into(),
                    object: Variable::new_unchecked("object").into(),
                }],
            },
            base_iri: None,
        };
        let alternate = ParseResultV1::success_for(&request, alternate_query)
            .encode()
            .expect("alternate valid result encodes");
        assert!(matches!(
            verify_reaped_result(&prepared, &alternate),
            Err(SupervisorError::InvalidState(
                "QueryV1 transport payload differs from the fixed fixture"
            ))
        ));
    }
}
