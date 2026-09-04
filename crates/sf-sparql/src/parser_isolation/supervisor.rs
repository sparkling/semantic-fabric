//! Product-owned Linux parser-worker supervisor foundation.
//!
//! This is intentionally separate from `sf-conformance`'s run-to-exit evidence
//! capture: a parser worker is an interactive control-protocol peer with
//! materially different descriptor, lifecycle, and containment invariants.
//! Nothing outside `parser_isolation` can launch this worker. The public binary
//! has a fail-closed private entry discriminator, and only a non-default Rust
//! evidence seam can reach the Hello/Ready/EOF exchange. No parser request can
//! yet reach the supervisor.
//!
//! The foundation pins one opened current-executable inode, observes bounded
//! bytes, applies exact OS limits, prevents descendants/group escape, and owns
//! pidfd/group cleanup. It does **not** attest release provenance or dynamic
//! libraries, restrict filesystem/network/ioctl access, drop OS privilege,
//! implement a general syscall sandbox, or make reap bounded under
//! uninterruptible kernel sleep. Parent pipe operations are cumulative-byte
//! bounded, nonblocking, and share the immutable spawn deadline, but no protocol
//! query exchange calls them yet. The default-allow stage-one filter is safe
//! only because the worker verifies its inherited state and stacks a
//! default-kill control-ready candidate before reading peer-controlled bytes.
//! That candidate is not parser-qualified and grants no query admission.

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
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod seccomp;

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
use executable::PreparedParserExecutable;

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

/// Buildable fail-closed stub for every unqualified target.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu")))]
struct PreparedParserExecutable;

#[cfg(not(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu")))]
impl PreparedParserExecutable {
    fn current() -> Result<Self, SupervisorError> {
        Err(SupervisorError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod io_tests;
#[cfg(test)]
mod tests;
