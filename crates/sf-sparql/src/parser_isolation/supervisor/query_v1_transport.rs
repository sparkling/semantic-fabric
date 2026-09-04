//! Parent sequencing for the parser-free synthetic QueryV1 transport.

use std::process::ExitStatus;

use super::handshake::ControlReadyWorker;
use super::lifecycle::ParserWorkerProcess;
use super::SupervisorError;
use crate::parser_isolation::parse_protocol::{
    allocate_frame_exact, FrameAllocation, PreparedParseRequestV1, ResultHeaderV1,
    RESULT_HEADER_LEN,
};
#[cfg(feature = "query-v1-transport-evidence")]
use crate::parser_isolation::parse_protocol::{
    ParseFrameError, ParseRequestV1, ParseResultV1, SYNTHETIC_EMPTY_ASK_QUERY_V1,
};
#[cfg(feature = "query-v1-transport-evidence")]
use crate::parser_isolation::protocol::FRAME_LEN;
#[cfg(feature = "query-v1-transport-evidence")]
use crate::parser_isolation::query_v1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TransportStage {
    RequestWrite,
    ResultHeaderRead,
    StructuralPreflight,
    ProspectiveOutput,
    BodyLimit,
    FrameAllocation,
    ResultBodyRead,
    OutputEof,
    TerminalWait,
    Containment,
}

/// A complete frame whose peer has already passed pidfd waitability, process-
/// group sweep, and exact reap. Callers must still require successful status
/// before inspecting any peer-controlled result semantics.
pub(super) struct ReapedTransport {
    pub(super) prepared_request: PreparedParseRequestV1,
    pub(super) encoded_result: Vec<u8>,
    pub(super) status: ExitStatus,
    pub(super) sent_bytes: u64,
    pub(super) received_bytes: u64,
}

/// Closed evidence for a transport failure after containment was attempted.
/// No stack header, partial frame, or other peer-controlled bytes escape.
pub(super) struct TransportFailure {
    pub(super) stage: TransportStage,
    pub(super) error: SupervisorError,
    pub(super) terminal_status: Option<ExitStatus>,
    pub(super) reaped: bool,
    pub(super) sent_bytes: u64,
    pub(super) received_bytes: u64,
}

impl TransportFailure {
    fn into_error(self) -> SupervisorError {
        self.error
    }
}

#[cfg(feature = "query-v1-transport-evidence")]
pub(super) fn finish(worker: ControlReadyWorker) -> Result<(), SupervisorError> {
    let reaped = capture_reaped(worker).map_err(TransportFailure::into_error)?;
    if !reaped.status.success() {
        return Err(SupervisorError::InvalidState(
            "QueryV1 transport peer did not exit successfully",
        ));
    }

    verify_reaped_accounting(&reaped)?;
    verify_reaped_result(&reaped.prepared_request, &reaped.encoded_result)
}

/// Capture one complete QueryV1 frame without touching its semantic fields.
/// Every pre-terminal failure is contained and reaped before returning.
pub(super) fn capture_reaped(
    worker: ControlReadyWorker,
) -> Result<ReapedTransport, TransportFailure> {
    let ControlReadyWorker {
        mut process,
        prepared_request,
    } = worker;

    let write_result = process.io.write_all(
        &process.pidfd,
        process.wall_deadline,
        prepared_request.encoded(),
    );
    complete_live_step(&mut process, TransportStage::RequestWrite, write_result)?;
    process.close_stdin();

    let mut encoded_header = [0_u8; RESULT_HEADER_LEN];
    let header_read =
        process
            .io
            .read_exact(&process.pidfd, process.wall_deadline, &mut encoded_header);
    complete_live_step(&mut process, TransportStage::ResultHeaderRead, header_read)?;
    let preflight = ResultHeaderV1::preflight(&encoded_header).map_err(SupervisorError::from);
    let header = complete_live_step(&mut process, TransportStage::StructuralPreflight, preflight)?;

    // Charge the peer's declared body against Ready plus the fixed header
    // before either trusting that declaration or allocating storage for it.
    let prospective = process
        .io
        .ensure_can_receive(process.wall_deadline, header.body_len());
    complete_live_step(&mut process, TransportStage::ProspectiveOutput, prospective)?;
    let body_limit = header.enforce_body_limit().map_err(SupervisorError::from);
    complete_live_step(&mut process, TransportStage::BodyLimit, body_limit)?;

    // The only result-frame allocation owns the complete header-plus-body
    // bytes. No detached body or later capacity growth is permitted.
    let allocation = allocate_frame_exact(header.frame_len(), FrameAllocation::Attempt)
        .map_err(SupervisorError::from);
    let mut encoded_result =
        complete_live_step(&mut process, TransportStage::FrameAllocation, allocation)?;
    encoded_result[..RESULT_HEADER_LEN].copy_from_slice(&encoded_header);
    let body_read = process.io.read_exact(
        &process.pidfd,
        process.wall_deadline,
        &mut encoded_result[RESULT_HEADER_LEN..],
    );
    complete_live_step(&mut process, TransportStage::ResultBodyRead, body_read)?;
    let eof = process.io.expect_eof(&process.pidfd, process.wall_deadline);
    complete_live_step(&mut process, TransportStage::OutputEof, eof)?;

    // wait_until_deadline performs pidfd waitability inspection, sweeps the
    // process group while its leader is unreaped, and then reaps the exact
    // Child. Peer-controlled semantics remain untouched until this succeeds.
    let status = match process.wait_until_deadline() {
        Ok(status) => status,
        Err(error) => return Err(failure_after_wait(&mut process, error)),
    };
    Ok(ReapedTransport {
        prepared_request,
        encoded_result,
        status,
        sent_bytes: process.sent_bytes(),
        received_bytes: process.received_bytes(),
    })
}

fn complete_live_step<T>(
    process: &mut ParserWorkerProcess,
    stage: TransportStage,
    result: Result<T, SupervisorError>,
) -> Result<T, TransportFailure> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => Err(contain_failure(process, stage, error)),
    }
}

fn contain_failure(
    process: &mut ParserWorkerProcess,
    stage: TransportStage,
    primary: SupervisorError,
) -> TransportFailure {
    let sent_bytes = process.sent_bytes();
    let received_bytes = process.received_bytes();
    match process.terminate_and_reap() {
        Ok(status) => TransportFailure {
            stage,
            error: primary,
            terminal_status: Some(status),
            reaped: true,
            sent_bytes,
            received_bytes,
        },
        Err(containment) => TransportFailure {
            stage: TransportStage::Containment,
            error: containment,
            terminal_status: None,
            reaped: process.child.is_none(),
            sent_bytes,
            received_bytes,
        },
    }
}

fn failure_after_wait(
    process: &mut ParserWorkerProcess,
    primary: SupervisorError,
) -> TransportFailure {
    if process.child.is_some() {
        return contain_failure(process, TransportStage::TerminalWait, primary);
    }
    TransportFailure {
        stage: TransportStage::TerminalWait,
        error: primary,
        terminal_status: None,
        reaped: true,
        sent_bytes: process.sent_bytes(),
        received_bytes: process.received_bytes(),
    }
}

#[cfg(feature = "query-v1-transport-evidence")]
fn verify_reaped_accounting(reaped: &ReapedTransport) -> Result<(), SupervisorError> {
    let expected_input = FRAME_LEN
        .checked_add(reaped.prepared_request.encoded().len())
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
    if reaped.sent_bytes != expected_input || reaped.received_bytes != expected_output {
        return Err(SupervisorError::InvalidState(
            "QueryV1 transport lifetime byte accounting drifted",
        ));
    }
    Ok(())
}

/// Replays request and result semantics only after the caller has completed a
/// successful exact reap. The decoded `Query` remains local and is dropped
/// before this evidence-only function returns unit.
#[cfg(feature = "query-v1-transport-evidence")]
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

#[cfg(all(test, feature = "query-v1-transport-evidence"))]
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
