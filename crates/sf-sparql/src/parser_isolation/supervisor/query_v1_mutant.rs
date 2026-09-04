//! Parent-only verification of the ten closed QueryV1 transport mutants.

use std::time::{Duration, Instant};

use super::executable::PreparedParserExecutable;
use super::handshake::ControlReadyWorker;
use super::query_v1_transport::{
    capture_reaped, ReapedTransport, TransportFailure, TransportStage,
};
use super::SupervisorError;
use crate::parser_isolation::parse_protocol::{
    ParseFrameError, ParseRequestV1, ParseResultV1, PreparedParseRequestV1, RequestEofCorruption,
    RESULT_HEADER_LEN,
};
use crate::parser_isolation::profile::V1_CANDIDATE_MAX_OUTPUT_BYTES;
use crate::parser_isolation::protocol::FRAME_LEN;
use crate::parser_isolation::query_v1_mutant::{QueryV1TransportMutant, MUTANT_DIRECTIVE_LEN};

const PRIVATE_REJECTED_EXIT_CODE: i32 = 78;
const STANDARD_RESULT_BODY_LEN: usize = 100;
const STANDARD_OUTPUT_BYTES: u64 =
    (FRAME_LEN + RESULT_HEADER_LEN + STANDARD_RESULT_BODY_LEN) as u64;
const HEADER_ONLY_OUTPUT_BYTES: u64 = (FRAME_LEN + RESULT_HEADER_LEN) as u64;
const READY_ONLY_OUTPUT_BYTES: u64 = FRAME_LEN as u64;
const REQUEST_EOF_OBSERVATION: Duration = Duration::from_millis(100);
const REQUEST_EOF_SOURCE_BYTES: usize = 128 * 1024;
const REQUEST_EOF_MUTANT: QueryV1TransportMutant = QueryV1TransportMutant::WrongNonceExitZero;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpectedTerminal {
    PostReap(ParseFrameError),
    Exit78,
    Contained {
        stage: TransportStage,
        failure: ExpectedFailure,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpectedFailure {
    Deadline,
    TrailingOutput,
    ProspectiveOutputCap,
    RequestAllocationRefusal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExpectedOutcome {
    terminal: ExpectedTerminal,
    output_bytes: u64,
}

/// Prove that validation of three fixed request defects is strictly post-EOF.
pub(super) fn exercise_request_eof_order(
    executable: &PreparedParserExecutable,
) -> Result<(), SupervisorError> {
    let source = request_eof_source()?;
    for corruption in RequestEofCorruption::ALL {
        observe_one_request_eof_order(executable, &source, corruption)?;
        prove_clean_next_launch(executable)?;
    }
    Ok(())
}

fn observe_one_request_eof_order(
    executable: &PreparedParserExecutable,
    source: &str,
    corruption: RequestEofCorruption,
) -> Result<(), SupervisorError> {
    let mut worker = executable.launch_query_v1_transport_mutant(source, REQUEST_EOF_MUTANT)?;
    let corrupted = worker
        .prepared_request
        .corrupted_for_eof_evidence(corruption)?;
    if corrupted.len() != worker.prepared_request.encoded().len() {
        return Err(SupervisorError::InvalidState(
            "request EOF corruption changed the frame length",
        ));
    }
    let expected_input = expected_input_bytes(&worker.prepared_request)?;

    worker.process.write_all_until_deadline(&corrupted)?;
    let observation_deadline = Instant::now().checked_add(REQUEST_EOF_OBSERVATION).ok_or(
        SupervisorError::InvalidState("request EOF observation deadline overflowed"),
    )?;
    worker
        .process
        .observe_alive_and_silent_until(observation_deadline)?;
    require_accounting(
        worker.process.sent_bytes(),
        worker.process.received_bytes(),
        expected_input,
        READY_ONLY_OUTPUT_BYTES,
    )?;

    worker.process.close_stdin();
    worker.process.expect_stdout_eof_until_deadline()?;
    let status = worker.process.wait_until_deadline()?;
    require_exit_78(&status)?;
    require_accounting(
        worker.process.sent_bytes(),
        worker.process.received_bytes(),
        expected_input,
        READY_ONLY_OUTPUT_BYTES,
    )?;
    worker.prepared_request.verify_exact()?;
    Ok(())
}

fn prove_clean_next_launch(executable: &PreparedParserExecutable) -> Result<(), SupervisorError> {
    let worker = executable
        .launch_query_v1_transport_mutant("clean request EOF recovery", REQUEST_EOF_MUTANT)?;
    finish(worker, REQUEST_EOF_MUTANT)
}

fn request_eof_source() -> Result<String, SupervisorError> {
    let mut source = String::new();
    source
        .try_reserve_exact(REQUEST_EOF_SOURCE_BYTES)
        .map_err(|_| {
            SupervisorError::InvalidState("request EOF evidence source allocation failed")
        })?;
    source.extend(std::iter::repeat_n('x', REQUEST_EOF_SOURCE_BYTES));
    Ok(source)
}

pub(super) fn finish(
    worker: ControlReadyWorker,
    mutant: QueryV1TransportMutant,
) -> Result<(), SupervisorError> {
    let expected_input = expected_input_bytes(&worker.prepared_request)?;
    let expected = expected_outcome(mutant);
    match (expected.terminal, capture_reaped(worker)) {
        (ExpectedTerminal::PostReap(expected_error), Ok(reaped)) => {
            require_success(&reaped)?;
            require_accounting(
                reaped.sent_bytes,
                reaped.received_bytes,
                expected_input,
                expected.output_bytes,
            )?;
            let observed_error = decode_reaped_error(&reaped)?;
            if observed_error != expected_error {
                return Err(unexpected_outcome());
            }
            Ok(())
        }
        (ExpectedTerminal::Exit78, Ok(reaped)) => {
            require_exit_78(&reaped.status)?;
            require_accounting(
                reaped.sent_bytes,
                reaped.received_bytes,
                expected_input,
                expected.output_bytes,
            )
        }
        (
            ExpectedTerminal::Contained {
                stage,
                failure: expected_failure,
            },
            Err(failure),
        ) => verify_contained_failure(
            &failure,
            stage,
            expected_failure,
            expected_input,
            expected.output_bytes,
        ),
        _ => Err(unexpected_outcome()),
    }
}

fn expected_input_bytes(prepared_request: &PreparedParseRequestV1) -> Result<u64, SupervisorError> {
    MUTANT_DIRECTIVE_LEN
        .checked_add(FRAME_LEN)
        .and_then(|total| total.checked_add(prepared_request.encoded().len()))
        .and_then(|total| u64::try_from(total).ok())
        .ok_or(SupervisorError::InvalidState(
            "QueryV1 mutant input accounting overflowed",
        ))
}

fn expected_outcome(mutant: QueryV1TransportMutant) -> ExpectedOutcome {
    use QueryV1TransportMutant as Mutant;

    match mutant {
        Mutant::WrongNonceExitZero => post_reap(ParseFrameError::NonceMismatch),
        Mutant::WrongSourceDigestExitZero => post_reap(ParseFrameError::SourceDigestMismatch),
        Mutant::WrongPayloadDigestExitZero => post_reap(ParseFrameError::PayloadDigestMismatch),
        Mutant::SelfConsistentDigestOverInvalidQueryV1ExitZero => {
            post_reap(ParseFrameError::InvalidQueryPayload)
        }
        Mutant::WrongCorrelationThenExit78 => ExpectedOutcome {
            terminal: ExpectedTerminal::Exit78,
            output_bytes: STANDARD_OUTPUT_BYTES,
        },
        Mutant::WrongCorrelationThenDeadlineStall => contained(
            TransportStage::OutputEof,
            ExpectedFailure::Deadline,
            STANDARD_OUTPUT_BYTES,
        ),
        Mutant::TrailingOutput => contained(
            TransportStage::OutputEof,
            ExpectedFailure::TrailingOutput,
            STANDARD_OUTPUT_BYTES,
        ),
        Mutant::ExactOutputCap => ExpectedOutcome {
            terminal: ExpectedTerminal::PostReap(ParseFrameError::InvalidQueryPayload),
            output_bytes: V1_CANDIDATE_MAX_OUTPUT_BYTES,
        },
        Mutant::OutputCapPlusOne => contained(
            TransportStage::ProspectiveOutput,
            ExpectedFailure::ProspectiveOutputCap,
            HEADER_ONLY_OUTPUT_BYTES,
        ),
        Mutant::RequestFrameAllocationRefusal => contained(
            TransportStage::ResultHeaderRead,
            ExpectedFailure::RequestAllocationRefusal,
            READY_ONLY_OUTPUT_BYTES,
        ),
    }
}

const fn post_reap(error: ParseFrameError) -> ExpectedOutcome {
    ExpectedOutcome {
        terminal: ExpectedTerminal::PostReap(error),
        output_bytes: STANDARD_OUTPUT_BYTES,
    }
}

const fn contained(
    stage: TransportStage,
    failure: ExpectedFailure,
    output_bytes: u64,
) -> ExpectedOutcome {
    ExpectedOutcome {
        terminal: ExpectedTerminal::Contained { stage, failure },
        output_bytes,
    }
}

fn require_success(reaped: &ReapedTransport) -> Result<(), SupervisorError> {
    if reaped.status.success() {
        Ok(())
    } else {
        Err(unexpected_outcome())
    }
}

fn require_exit_78(status: &std::process::ExitStatus) -> Result<(), SupervisorError> {
    if status.code() == Some(PRIVATE_REJECTED_EXIT_CODE) {
        Ok(())
    } else {
        Err(unexpected_outcome())
    }
}

fn verify_contained_failure(
    observed: &TransportFailure,
    expected_stage: TransportStage,
    expected_failure: ExpectedFailure,
    expected_input: u64,
    expected_output: u64,
) -> Result<(), SupervisorError> {
    if !observed.reaped
        || observed.terminal_status.is_none()
        || observed.stage != expected_stage
        || !matches_failure(&observed.error, expected_failure)
    {
        return Err(unexpected_outcome());
    }
    if expected_failure == ExpectedFailure::RequestAllocationRefusal {
        require_exit_78(
            observed
                .terminal_status
                .as_ref()
                .expect("terminal status checked above"),
        )?;
    }
    require_accounting(
        observed.sent_bytes,
        observed.received_bytes,
        expected_input,
        expected_output,
    )
}

fn matches_failure(error: &SupervisorError, expected: ExpectedFailure) -> bool {
    match expected {
        ExpectedFailure::Deadline => matches!(error, SupervisorError::DeadlineExceeded),
        ExpectedFailure::TrailingOutput => matches!(
            error,
            SupervisorError::InvalidState("parser worker emitted trailing protocol output")
        ),
        ExpectedFailure::ProspectiveOutputCap => matches!(
            error,
            SupervisorError::InvalidState("parser worker output exceeds its cumulative byte limit")
        ),
        ExpectedFailure::RequestAllocationRefusal => matches!(
            error,
            SupervisorError::InvalidState(
                "parser worker output ended before the fixed read completed"
            )
        ),
    }
}

fn require_accounting(
    sent_bytes: u64,
    received_bytes: u64,
    expected_input: u64,
    expected_output: u64,
) -> Result<(), SupervisorError> {
    if sent_bytes == expected_input && received_bytes == expected_output {
        Ok(())
    } else {
        Err(SupervisorError::InvalidState(
            "QueryV1 mutant lifetime byte accounting drifted",
        ))
    }
}

fn decode_reaped_error(reaped: &ReapedTransport) -> Result<ParseFrameError, SupervisorError> {
    reaped.prepared_request.verify_exact()?;
    let request = ParseRequestV1::decode_exact_for_nonce(
        reaped.prepared_request.encoded(),
        reaped.prepared_request.nonce(),
    )?;
    match ParseResultV1::decode_exact_for(&reaped.encoded_result, &request) {
        Ok(_) => Err(unexpected_outcome()),
        Err(error) => Ok(error),
    }
}

const fn unexpected_outcome() -> SupervisorError {
    SupervisorError::InvalidState("QueryV1 mutant did not exhibit its exact expected outcome")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_mutants_have_the_exact_terminal_stage_and_output_accounting() {
        let outcomes = QueryV1TransportMutant::ALL.map(expected_outcome);
        assert_eq!(
            outcomes,
            [
                post_reap(ParseFrameError::NonceMismatch),
                post_reap(ParseFrameError::SourceDigestMismatch),
                post_reap(ParseFrameError::PayloadDigestMismatch),
                post_reap(ParseFrameError::InvalidQueryPayload),
                ExpectedOutcome {
                    terminal: ExpectedTerminal::Exit78,
                    output_bytes: 412,
                },
                contained(TransportStage::OutputEof, ExpectedFailure::Deadline, 412,),
                contained(
                    TransportStage::OutputEof,
                    ExpectedFailure::TrailingOutput,
                    412,
                ),
                ExpectedOutcome {
                    terminal: ExpectedTerminal::PostReap(ParseFrameError::InvalidQueryPayload),
                    output_bytes: 8_388_920,
                },
                contained(
                    TransportStage::ProspectiveOutput,
                    ExpectedFailure::ProspectiveOutputCap,
                    312,
                ),
                contained(
                    TransportStage::ResultHeaderRead,
                    ExpectedFailure::RequestAllocationRefusal,
                    184,
                ),
            ]
        );
    }

    #[test]
    fn input_accounting_includes_directive_hello_and_complete_request() {
        use crate::parser_isolation::protocol::HandshakeNonce;
        use crate::parser_isolation::query_v1_mutant::MAX_MUTANT_SOURCE_BYTES_V1;

        let empty = PreparedParseRequestV1::new(HandshakeNonce::new([3; 32]), "").unwrap();
        assert_eq!(expected_input_bytes(&empty).unwrap(), 2 + 184 + 96);

        let source = "#".repeat(MAX_MUTANT_SOURCE_BYTES_V1);
        let exact = PreparedParseRequestV1::new(HandshakeNonce::new([5; 32]), &source).unwrap();
        assert_eq!(
            expected_input_bytes(&exact).unwrap(),
            1_048_856,
            "the directive reduces source capacity without changing the whole-life cap"
        );
    }
}
