//! Parent-owned fresh-child observation of the sealed real-parser corpus.

use super::executable::PreparedParserExecutable;
use super::handshake::ControlReadyWorker;
use super::SupervisorError;
use crate::parser_isolation::parser_observation::{
    decode_observation_for, ParserObservationOutcomeV1, ParserObservationSummaryV1,
    OBSERVATION_FRAME_LEN, PARSER_OBSERVATION_CORPUS_V1,
};
use crate::parser_isolation::protocol::FRAME_LEN;

pub(super) fn exercise_corpus(
    executable: &PreparedParserExecutable,
) -> Result<ParserObservationSummaryV1, SupervisorError> {
    let mut parsed = 0_u16;
    let mut syntax_rejected = 0_u16;

    for case in PARSER_OBSERVATION_CORPUS_V1 {
        let worker = executable.launch_parser_observation(case.source)?;
        let outcome = finish_case(worker, case.source)?;
        if outcome != case.expected {
            return Err(SupervisorError::InvalidState(
                "sealed parser observation outcome drifted",
            ));
        }
        match outcome {
            ParserObservationOutcomeV1::Parsed => {
                parsed = parsed.checked_add(1).ok_or(SupervisorError::InvalidState(
                    "parser observation parsed count overflowed",
                ))?;
            }
            ParserObservationOutcomeV1::SyntaxRejected => {
                syntax_rejected =
                    syntax_rejected
                        .checked_add(1)
                        .ok_or(SupervisorError::InvalidState(
                            "parser observation syntax-rejection count overflowed",
                        ))?;
            }
        }
    }

    ParserObservationSummaryV1::from_verified_counts(parsed, syntax_rejected)
        .map_err(SupervisorError::InvalidState)
}

fn finish_case(
    worker: ControlReadyWorker,
    source: &str,
) -> Result<ParserObservationOutcomeV1, SupervisorError> {
    let ControlReadyWorker {
        mut process,
        prepared_request,
    } = worker;
    process.write_all_until_deadline(prepared_request.encoded())?;
    process.close_stdin();

    let mut frame = [0_u8; OBSERVATION_FRAME_LEN];
    process.read_exact_until_deadline(&mut frame)?;
    process.expect_stdout_eof_until_deadline()?;
    let status = process.wait_until_deadline()?;
    if !status.success() {
        return Err(SupervisorError::InvalidState(
            "parser observation peer did not exit successfully",
        ));
    }

    let expected_input = FRAME_LEN
        .checked_add(prepared_request.encoded().len())
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(SupervisorError::InvalidState(
            "parser observation input accounting overflowed",
        ))?;
    let expected_output = FRAME_LEN
        .checked_add(OBSERVATION_FRAME_LEN)
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(SupervisorError::InvalidState(
            "parser observation output accounting overflowed",
        ))?;
    if process.sent_bytes() != expected_input || process.received_bytes() != expected_output {
        return Err(SupervisorError::InvalidState(
            "parser observation lifetime byte accounting drifted",
        ));
    }

    prepared_request.verify_exact()?;
    decode_observation_for(&frame, prepared_request.nonce(), source)
        .map_err(SupervisorError::InvalidState)
}
