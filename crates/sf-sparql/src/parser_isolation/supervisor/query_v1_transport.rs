//! Parent sequencing for synthetic and sealed-corpus parser QueryV1 transports.

use std::process::ExitStatus;

#[cfg(feature = "parser-worker-evidence")]
use super::executable::PreparedParserExecutable;
use super::handshake::ControlReadyWorker;
use super::lifecycle::ParserWorkerProcess;
use super::SupervisorError;
#[cfg(feature = "query-v1-transport-evidence")]
use crate::parser_isolation::parse_protocol::ParseFrameError;
#[cfg(feature = "query-v1-transport-evidence")]
use crate::parser_isolation::parse_protocol::SYNTHETIC_EMPTY_ASK_QUERY_V1;
use crate::parser_isolation::parse_protocol::{
    allocate_frame_exact, FrameAllocation, PreparedParseRequestV1, ResultHeaderV1,
    RESULT_HEADER_LEN,
};
#[cfg(any(
    feature = "parser-worker-evidence",
    feature = "query-v1-transport-evidence"
))]
use crate::parser_isolation::parse_protocol::{ParseRequestV1, ParseResultV1};
#[cfg(any(
    feature = "parser-worker-evidence",
    feature = "query-v1-transport-evidence"
))]
use crate::parser_isolation::protocol::FRAME_LEN;
#[cfg(any(
    feature = "parser-worker-evidence",
    feature = "query-v1-transport-evidence"
))]
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

#[cfg(feature = "parser-worker-evidence")]
pub(super) fn exercise_parser_corpus(
    executable: &PreparedParserExecutable,
) -> Result<crate::parser_isolation::ParserObservationSummaryV1, SupervisorError> {
    use crate::parser_isolation::parser_observation::{
        ParserObservationOutcomeV1, PARSER_OBSERVATION_CORPUS_V1,
    };

    let mut parsed = 0_u16;
    let mut syntax_rejected = 0_u16;
    for case in PARSER_OBSERVATION_CORPUS_V1 {
        // This remains diagnostic evidence, not parser admission authority.
        crate::compile_envelope::CompileEnvelopeV1::scan(case.source).map_err(|_| {
            SupervisorError::InvalidState("sealed parser QueryV1 lexical envelope drifted")
        })?;
        let worker = executable.launch_parser_query_v1(case.source)?;
        finish_parser_case(worker, case.source, case.expected)?;
        match case.expected {
            ParserObservationOutcomeV1::Parsed => {
                parsed = parsed.checked_add(1).ok_or(SupervisorError::InvalidState(
                    "parser QueryV1 parsed count overflowed",
                ))?;
            }
            ParserObservationOutcomeV1::SyntaxRejected => {
                syntax_rejected =
                    syntax_rejected
                        .checked_add(1)
                        .ok_or(SupervisorError::InvalidState(
                            "parser QueryV1 syntax-rejection count overflowed",
                        ))?;
            }
        }
    }
    crate::parser_isolation::ParserObservationSummaryV1::from_verified_counts(
        parsed,
        syntax_rejected,
    )
    .map_err(SupervisorError::InvalidState)
}

#[cfg(feature = "parser-worker-evidence")]
fn finish_parser_case(
    worker: ControlReadyWorker,
    source: &str,
    expected: crate::parser_isolation::parser_observation::ParserObservationOutcomeV1,
) -> Result<(), SupervisorError> {
    let reaped = capture_reaped(worker).map_err(TransportFailure::into_error)?;
    if !reaped.status.success() {
        return Err(SupervisorError::InvalidState(
            "parser QueryV1 peer did not exit successfully",
        ));
    }
    verify_dynamic_accounting(&reaped)?;
    verify_parser_result(
        &reaped.prepared_request,
        &reaped.encoded_result,
        source,
        expected,
    )
}

#[cfg(feature = "parser-worker-evidence")]
fn verify_dynamic_accounting(reaped: &ReapedTransport) -> Result<(), SupervisorError> {
    let expected_input = FRAME_LEN
        .checked_add(reaped.prepared_request.encoded().len())
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(SupervisorError::InvalidState(
            "parser QueryV1 input accounting overflowed",
        ))?;
    let expected_output = FRAME_LEN
        .checked_add(reaped.encoded_result.len())
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(SupervisorError::InvalidState(
            "parser QueryV1 output accounting overflowed",
        ))?;
    if reaped.sent_bytes != expected_input || reaped.received_bytes != expected_output {
        return Err(SupervisorError::InvalidState(
            "parser QueryV1 lifetime byte accounting drifted",
        ));
    }
    Ok(())
}

#[cfg(feature = "parser-worker-evidence")]
fn verify_parser_result(
    prepared_request: &PreparedParseRequestV1,
    encoded_result: &[u8],
    source: &str,
    expected: crate::parser_isolation::parser_observation::ParserObservationOutcomeV1,
) -> Result<(), SupervisorError> {
    use crate::parser_isolation::alpha_equivalence::{
        compare, AlphaVerdictV1, ParseObservationV1, ParseOutcomeV1, SourceDigestV1,
    };
    use crate::parser_isolation::parser_observation::ParserObservationOutcomeV1;
    use crate::parser_isolation::profile::parser_worker_evidence_profile_v1;

    prepared_request.verify_exact()?;
    let request = ParseRequestV1::decode_exact_for_nonce(
        prepared_request.encoded(),
        prepared_request.nonce(),
    )?;
    if request.source() != source {
        return Err(SupervisorError::InvalidState(
            "parser QueryV1 request source replay drifted",
        ));
    }
    let result = ParseResultV1::decode_exact_for(encoded_result, &request)?;
    let direct = spargebra::SparqlParser::new().parse_query(source);

    match (expected, result.query(), result.rejection(), direct) {
        (ParserObservationOutcomeV1::Parsed, Some(worker_query), None, Ok(direct_query)) => {
            let payload = &encoded_result[RESULT_HEADER_LEN..];
            let worker_replay = query_v1::encode(worker_query)
                .map_err(|_| SupervisorError::InvalidState("worker QueryV1 replay failed"))?;
            if worker_replay.as_slice() != payload {
                return Err(SupervisorError::InvalidState(
                    "worker QueryV1 bytes did not replay exactly",
                ));
            }

            let direct_wire = query_v1::encode(&direct_query)
                .map_err(|_| SupervisorError::InvalidState("direct QueryV1 encoding failed"))?;
            let direct_decoded = query_v1::decode_exact(&direct_wire)
                .map_err(|_| SupervisorError::InvalidState("direct QueryV1 decoding failed"))?;
            let direct_replay = query_v1::encode(&direct_decoded)
                .map_err(|_| SupervisorError::InvalidState("direct QueryV1 replay failed"))?;
            if direct_replay != direct_wire {
                return Err(SupervisorError::InvalidState(
                    "direct QueryV1 bytes did not replay exactly",
                ));
            }

            let digest = SourceDigestV1::of(source);
            let profile = parser_worker_evidence_profile_v1();
            let verdict = compare(
                ParseObservationV1::new(digest, profile, ParseOutcomeV1::Parsed(worker_query)),
                ParseObservationV1::new(digest, profile, ParseOutcomeV1::Parsed(&direct_query)),
            );
            if verdict != AlphaVerdictV1::Equivalent {
                return Err(SupervisorError::InvalidState(
                    "worker and direct parser QueryV1 semantics diverged",
                ));
            }
            Ok(())
        }
        (
            ParserObservationOutcomeV1::SyntaxRejected,
            None,
            Some(crate::parser_isolation::parse_protocol::ParseRejectionV1::Syntax),
            Err(_),
        ) => Ok(()),
        _ => Err(SupervisorError::InvalidState(
            "worker and direct parser QueryV1 outcomes diverged",
        )),
    }
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
