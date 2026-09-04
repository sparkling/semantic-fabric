//! Parent-owned transition from a spawned process to a control-ready peer.

use super::executable::PreparedParserExecutable;
use super::lifecycle::ParserWorkerProcess;
use super::{linux, SupervisorError};
#[cfg(feature = "query-v1-transport-mutant-evidence")]
use crate::parser_isolation::parse_protocol::ParseFrameError;
use crate::parser_isolation::parse_protocol::PreparedParseRequestV1;
use crate::parser_isolation::profile::{control_ready_profile_candidate_digest, v1_limits};
use crate::parser_isolation::protocol::{
    verify_ready, HandshakeNonce, HelloFrame, ReadyFrame, DIGEST_LEN, FRAME_LEN,
};
#[cfg(feature = "query-v1-transport-mutant-evidence")]
use crate::parser_isolation::query_v1_mutant::{
    QueryV1TransportMutant, MAX_MUTANT_SOURCE_BYTES_V1,
};

pub(super) struct ControlReadyWorker {
    pub(super) process: ParserWorkerProcess,
    pub(super) prepared_request: PreparedParseRequestV1,
}

pub(super) struct PreparedControlExchange {
    hello: HelloFrame,
    request: PreparedParseRequestV1,
}

pub(super) fn prepare(
    executable: &PreparedParserExecutable,
    source: &str,
) -> Result<PreparedControlExchange, SupervisorError> {
    let hello = HelloFrame::new(
        generate_nonce()?,
        executable.identity().build_identity(),
        control_ready_profile_candidate_digest(),
        v1_limits(),
    );
    let request = PreparedParseRequestV1::new(hello.nonce(), source)?;
    Ok(PreparedControlExchange { hello, request })
}

#[cfg(feature = "query-v1-transport-mutant-evidence")]
pub(super) fn prepare_query_v1_mutant(
    executable: &PreparedParserExecutable,
    source: &str,
) -> Result<PreparedControlExchange, SupervisorError> {
    if source.len() > MAX_MUTANT_SOURCE_BYTES_V1 {
        return Err(ParseFrameError::SourceLimitExceeded.into());
    }
    prepare(executable, source)
}

pub(super) fn launch(
    executable: &PreparedParserExecutable,
    prepared: PreparedControlExchange,
) -> Result<ControlReadyWorker, SupervisorError> {
    launch_with(executable, prepared, linux::spawn_private, None)
}

#[cfg(feature = "query-v1-transport-evidence")]
pub(super) fn launch_query_v1_transport(
    executable: &PreparedParserExecutable,
    prepared: PreparedControlExchange,
) -> Result<ControlReadyWorker, SupervisorError> {
    launch_with(executable, prepared, linux::spawn_query_v1_transport, None)
}

#[cfg(feature = "query-v1-transport-mutant-evidence")]
pub(super) fn launch_query_v1_transport_mutant(
    executable: &PreparedParserExecutable,
    prepared: PreparedControlExchange,
    mutant: QueryV1TransportMutant,
) -> Result<ControlReadyWorker, SupervisorError> {
    launch_with(
        executable,
        prepared,
        linux::spawn_query_v1_transport_mutant,
        Some(mutant.encode()),
    )
}

fn launch_with(
    executable: &PreparedParserExecutable,
    prepared: PreparedControlExchange,
    spawn: fn(
        &PreparedParserExecutable,
        crate::parser_isolation::protocol::ParserWorkerLimits,
    ) -> Result<ParserWorkerProcess, SupervisorError>,
    directive: Option<[u8; 2]>,
) -> Result<ControlReadyWorker, SupervisorError> {
    let mut process = spawn(executable, v1_limits())?;
    if let Some(directive) = directive {
        process.write_all_until_deadline(&directive)?;
    }
    process.write_all_until_deadline(&prepared.hello.encode())?;

    let mut encoded_ready = [0_u8; FRAME_LEN];
    process.read_exact_until_deadline(&mut encoded_ready)?;
    let ready = match ReadyFrame::decode(&encoded_ready) {
        Ok(ready) => ready,
        Err(error) => return Err(contain_protocol_failure(&mut process, error.into())),
    };
    if let Err(error) = verify_ready(&prepared.hello, &ready) {
        return Err(contain_protocol_failure(&mut process, error.into()));
    }
    Ok(ControlReadyWorker {
        process,
        prepared_request: prepared.request,
    })
}

impl ControlReadyWorker {
    /// Complete the control-only evidence exchange. QueryV1 will replace this
    /// EOF transition once its wire and parser differential are admitted.
    pub(super) fn finish_without_query(mut self) -> Result<(), SupervisorError> {
        self.process.close_stdin();
        self.process.expect_stdout_eof_until_deadline()?;
        let status = self.process.wait_until_deadline()?;
        if !status.success() {
            return Err(SupervisorError::InvalidState(
                "control-ready parser worker rejected EOF",
            ));
        }
        self.prepared_request.verify_exact()?;
        Ok(())
    }
}

fn contain_protocol_failure(
    process: &mut ParserWorkerProcess,
    primary: SupervisorError,
) -> SupervisorError {
    match process.terminate_and_reap() {
        Ok(_) => primary,
        Err(containment) => containment,
    }
}

fn generate_nonce() -> Result<HandshakeNonce, SupervisorError> {
    let mut nonce = [0_u8; DIGEST_LEN];
    let mut offset = 0;
    while offset < nonce.len() {
        let count = unsafe {
            libc::getrandom(nonce[offset..].as_mut_ptr().cast(), nonce.len() - offset, 0)
        };
        if count > 0 {
            offset += count as usize;
            continue;
        }
        if count == 0 {
            return Err(SupervisorError::InvalidState(
                "kernel entropy returned a zero-byte parser nonce",
            ));
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(SupervisorError::operation(
                "generate parser handshake nonce",
            )(error));
        }
    }
    Ok(HandshakeNonce::new(nonce))
}
