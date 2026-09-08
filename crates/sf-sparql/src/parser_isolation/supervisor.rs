//! Product-owned Linux parser-worker supervisor foundation.
//!
//! This is intentionally separate from `sf-conformance`'s run-to-exit evidence
//! capture: a parser worker is an interactive control-protocol peer with
//! materially different descriptor, lifecycle, and containment invariants.
//! The explicit ParserRuntime uses a fail-closed private entry discriminator
//! and the real-parser QueryV1 exchange. Independently gated peers retain the
//! parser-free transport, corpus differential and mutation diagnostics. None
//! grants GovernedV1, credential or release authority.
//!
//! The foundation pins one opened current-executable inode, observes bounded
//! bytes, applies exact OS limits, prevents descendants/group escape, and owns
//! pidfd/group cleanup. It does **not** attest release provenance or dynamic
//! libraries, restrict filesystem/network/ioctl access, drop OS privilege,
//! implement a general syscall sandbox, or make reap bounded under
//! uninterruptible kernel sleep. Parent pipe operations are cumulative-byte
//! bounded, nonblocking, and share the immutable spawn deadline across both the
//! handshake and synthetic frame. The default-allow stage-one filter is safe
//! only because the worker verifies its inherited state and stacks a default-kill
//! control-ready candidate before reading peer-controlled bytes. That candidate
//! is not a complete syscall/dependency qualification or credential boundary.

use std::fmt;
use std::io as std_io;

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
use super::parse_protocol::ParseFrameError;
#[cfg(test)]
use super::profile::v1_limits;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
use super::protocol::HandshakeError;

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod executable;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod handshake;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod io;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod lifecycle;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod linux;
#[cfg(all(
    feature = "parser-worker-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]
mod parser_observation;
#[cfg(all(
    feature = "query-v1-transport-mutant-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]
mod query_v1_mutant;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
pub(super) mod query_v1_transport;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod seccomp;

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
pub(super) use executable::PreparedParserExecutable;

/// Closed failure categories for the private supervisor boundary.
#[derive(Debug)]
pub(crate) enum SupervisorError {
    UnsupportedPlatform,
    InvalidExecutable(&'static str),
    InvalidLimits(&'static str),
    InvalidState(&'static str),
    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    Protocol(HandshakeError),
    #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
    ParseFrame(ParseFrameError),
    DeadlineExceeded,
    RequestControl(sf_core::query_control::QueryControlError),
    ParseRejected(super::parse_protocol::ParseRejectionV1),
    Operation {
        operation: &'static str,
        source: std_io::Error,
    },
}

impl SupervisorError {
    pub(super) fn operation(operation: &'static str) -> impl FnOnce(std_io::Error) -> Self {
        move |source| Self::Operation { operation, source }
    }
}

impl fmt::Display for SupervisorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestControl(_) => formatter.write_str("parser request terminated"),
            Self::ParseRejected(_) => formatter.write_str("parser rejected query"),
            Self::UnsupportedPlatform => {
                formatter.write_str("parser supervisor requires qualified GNU x86-64 Linux")
            }
            Self::InvalidExecutable(reason) => {
                write!(formatter, "invalid held parser executable: {reason}")
            }
            Self::InvalidLimits(reason) => {
                write!(formatter, "invalid parser containment limits: {reason}")
            }
            Self::InvalidState(reason) => write!(formatter, "invalid parser child state: {reason}"),
            #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
            Self::Protocol(error) => write!(formatter, "parser worker protocol: {error}"),
            #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
            Self::ParseFrame(error) => write!(formatter, "parser request preparation: {error}"),
            Self::DeadlineExceeded => formatter.write_str("parser worker wall deadline exceeded"),
            Self::Operation { operation, source } => write!(formatter, "{operation}: {source}"),
        }
    }
}

impl std::error::Error for SupervisorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Operation { source, .. } => Some(source),
            #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
            Self::Protocol(source) => Some(source),
            #[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
            Self::ParseFrame(source) => Some(source),
            _ => None,
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
impl From<HandshakeError> for SupervisorError {
    fn from(error: HandshakeError) -> Self {
        Self::Protocol(error)
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
impl From<ParseFrameError> for SupervisorError {
    fn from(error: ParseFrameError) -> Self {
        Self::ParseFrame(error)
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
impl PreparedParserExecutable {
    /// The production-unreachable control-handshake launch primitive.
    ///
    /// `sf-cli` must not call this until its private worker dispatch runs before
    /// Clap/application thread-pool initialization and installs/verifies the
    /// control-ready policy candidate before emitting the handshake response.
    fn launch_private_worker(
        &self,
        source: &str,
    ) -> Result<handshake::ControlReadyWorker, SupervisorError> {
        let prepared = handshake::prepare(self, source)?;
        handshake::launch(self, prepared)
    }

    #[cfg(feature = "parser-worker-evidence")]
    fn launch_parser_observation(
        &self,
        source: &str,
    ) -> Result<handshake::ControlReadyWorker, SupervisorError> {
        let prepared = handshake::prepare(self, source)?;
        handshake::launch_parser_observation(self, prepared)
    }

    fn launch_parser_query_v1(
        &self,
        source: &str,
    ) -> Result<handshake::ControlReadyWorker, SupervisorError> {
        let prepared = handshake::prepare(self, source)?;
        handshake::launch_parser_query_v1(self, prepared)
    }

    pub(in crate::parser_isolation) fn parse_public(
        &self,
        source: &str,
    ) -> Result<spargebra::Query, SupervisorError> {
        query_v1_transport::finish_public(self.launch_parser_query_v1(source)?)
    }

    #[cfg(feature = "query-v1-transport-evidence")]
    fn launch_query_v1_transport(
        &self,
        source: &str,
    ) -> Result<handshake::ControlReadyWorker, SupervisorError> {
        let prepared = handshake::prepare(self, source)?;
        handshake::launch_query_v1_transport(self, prepared)
    }

    #[cfg(feature = "query-v1-transport-mutant-evidence")]
    fn launch_query_v1_transport_mutant(
        &self,
        source: &str,
        mutant: super::query_v1_mutant::QueryV1TransportMutant,
    ) -> Result<handshake::ControlReadyWorker, SupervisorError> {
        let prepared = handshake::prepare_query_v1_mutant(self, source)?;
        handshake::launch_query_v1_transport_mutant(self, prepared, mutant)
    }
}

#[cfg(all(
    feature = "parser-worker-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]
pub(super) fn exercise_handshake_for_evidence(
    file: std::fs::File,
    source: &str,
) -> Result<(), SupervisorError> {
    let prepared = PreparedParserExecutable::from_file_for_evidence(file)?;
    prepared
        .launch_private_worker(source)?
        .finish_without_query()
}

#[cfg(all(
    feature = "parser-worker-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]
pub(super) fn exercise_parser_observation_corpus_for_evidence(
    file: std::fs::File,
) -> Result<super::parser_observation::ParserObservationSummaryV1, SupervisorError> {
    let prepared = PreparedParserExecutable::from_file_for_evidence(file)?;
    parser_observation::exercise_corpus(&prepared)
}

#[cfg(all(
    feature = "parser-worker-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]
pub(super) fn exercise_parser_query_v1_corpus_for_evidence(
    file: std::fs::File,
) -> Result<super::parser_observation::ParserObservationSummaryV1, SupervisorError> {
    let prepared = PreparedParserExecutable::from_file_for_evidence(file)?;
    query_v1_transport::exercise_parser_corpus(&prepared)
}

#[cfg(all(
    feature = "query-v1-transport-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]
pub(super) fn exercise_query_v1_transport_for_evidence(
    file: std::fs::File,
    source: &str,
) -> Result<(), SupervisorError> {
    let prepared = PreparedParserExecutable::from_file_for_evidence(file)?;
    query_v1_transport::finish(prepared.launch_query_v1_transport(source)?)
}

#[cfg(all(
    feature = "query-v1-transport-mutant-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]
pub(super) fn exercise_query_v1_transport_mutant_for_evidence(
    file: std::fs::File,
    source: &str,
    mutant: super::query_v1_mutant::QueryV1TransportMutant,
) -> Result<(), SupervisorError> {
    let prepared = PreparedParserExecutable::from_file_for_evidence(file)?;
    query_v1_mutant::finish(
        prepared.launch_query_v1_transport_mutant(source, mutant)?,
        mutant,
    )
}

#[cfg(all(
    feature = "query-v1-transport-mutant-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]
pub(super) fn exercise_query_v1_mutant_matrix_for_evidence(
    file: std::fs::File,
    source: &str,
) -> Result<(), SupervisorError> {
    let prepared = PreparedParserExecutable::from_file_for_evidence(file)?;
    query_v1_mutant::exercise_matrix(&prepared, source)
}

#[cfg(all(
    feature = "query-v1-transport-mutant-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]
pub(super) fn exercise_query_v1_malformed_directives_for_evidence(
    file: std::fs::File,
) -> Result<(), SupervisorError> {
    let prepared = PreparedParserExecutable::from_file_for_evidence(file)?;
    query_v1_mutant::exercise_malformed_directives(&prepared)
}

#[cfg(all(
    feature = "query-v1-transport-mutant-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]
pub(super) fn exercise_query_v1_request_eof_order_for_evidence(
    file: std::fs::File,
) -> Result<(), SupervisorError> {
    let prepared = PreparedParserExecutable::from_file_for_evidence(file)?;
    query_v1_mutant::exercise_request_eof_order(&prepared)
}

/// Buildable fail-closed stub for every unqualified target.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu")))]
pub(super) struct PreparedParserExecutable;

#[cfg(not(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu")))]
impl PreparedParserExecutable {
    pub(super) fn current() -> Result<Self, SupervisorError> {
        Err(SupervisorError::UnsupportedPlatform)
    }

    pub(super) fn current_for_runtime() -> Result<Self, SupervisorError> {
        Err(SupervisorError::UnsupportedPlatform)
    }

    pub(super) fn parse_public(&self, _: &str) -> Result<spargebra::Query, SupervisorError> {
        Err(SupervisorError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod io_tests;
#[cfg(test)]
mod tests;

#[cfg(all(test, target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod runtime_control_tests;
