//! Dormant, product-owned Linux parser-worker supervisor foundation.
//!
//! This is intentionally separate from `sf-conformance`'s run-to-exit evidence
//! capture: a parser worker will eventually be an interactive protocol peer and
//! has materially different descriptor, lifecycle, and containment invariants.
//! Nothing outside `parser_isolation` can launch this worker, and no production
//! worker entry point exists yet.
//!
//! The foundation pins one opened current-executable inode, observes bounded
//! bytes, applies exact OS limits, prevents descendants/group escape, and owns
//! pidfd/group cleanup. It does **not** attest release provenance or dynamic
//! libraries, restrict filesystem/network/ioctl access, drop OS privilege,
//! implement a general syscall sandbox, or make reap bounded under
//! uninterruptible kernel sleep. Parent pipe operations are cumulative-byte
//! bounded, nonblocking, and share the immutable spawn deadline, but no protocol
//! exchange calls them yet. The default-allow stage-one filter is safe only
//! because no peer-controlled bytes are accepted before a future worker verifies
//! and stacks its final policy.

use std::fmt;
use std::io as std_io;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use super::protocol::{ParserWorkerLimitValues, ParserWorkerLimits};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod executable;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod io;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod lifecycle;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod linux;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod seccomp;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use executable::PreparedParserExecutable;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use lifecycle::ParserWorkerProcess;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const V1_LIMIT_VALUES: ParserWorkerLimitValues = ParserWorkerLimitValues {
    stack_bytes: 16 * 1024 * 1024,
    address_space_bytes: 1024 * 1024 * 1024,
    cpu_time_millis: 10_000,
    wall_time_millis: 15_000,
    max_input_bytes: 1024 * 1024,
    max_output_bytes: 64 * 1024 * 1024,
    max_open_fds: 64,
    max_processes: 1,
    max_concurrency: 64,
};

/// Closed failure categories for the dormant supervisor boundary.
#[derive(Debug)]
pub(crate) enum SupervisorError {
    UnsupportedPlatform,
    InvalidExecutable(&'static str),
    InvalidLimits(&'static str),
    InvalidState(&'static str),
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
                formatter.write_str("parser supervisor requires qualified x86-64 Linux")
            }
            Self::InvalidExecutable(reason) => {
                write!(formatter, "invalid held parser executable: {reason}")
            }
            Self::InvalidLimits(reason) => {
                write!(formatter, "invalid parser containment limits: {reason}")
            }
            Self::InvalidState(reason) => write!(formatter, "invalid parser child state: {reason}"),
            Self::DeadlineExceeded => formatter.write_str("parser worker wall deadline exceeded"),
            Self::Operation { operation, source } => write!(formatter, "{operation}: {source}"),
        }
    }
}

impl std::error::Error for SupervisorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Operation { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn v1_limits() -> ParserWorkerLimits {
    ParserWorkerLimits::new(V1_LIMIT_VALUES).expect("the fixed V1 limits are valid")
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl PreparedParserExecutable {
    /// The intentionally unreachable production launch primitive.
    ///
    /// `sf-cli` must not call this until its private worker dispatch runs before
    /// Clap/thread initialization and installs/verifies the final worker policy
    /// before emitting `Ready`.
    fn launch_private_worker(&self) -> Result<ParserWorkerProcess, SupervisorError> {
        linux::spawn_private(self, v1_limits())
    }
}

/// Buildable fail-closed stub for every unqualified target.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
struct PreparedParserExecutable;

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
impl PreparedParserExecutable {
    fn current() -> Result<Self, SupervisorError> {
        Err(SupervisorError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod io_tests;
#[cfg(test)]
mod tests;
