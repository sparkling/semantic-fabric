//! Parser-worker isolation protocol foundations (ADR-0053).
//!
//! A held-descriptor supervisor can exercise a post-exec control envelope and
//! exact Hello/Ready/EOF exchange through a non-default evidence feature. A raw
//! or malformed reserved invocation fails closed. The policy and profile are
//! control-ready candidates only. Private canonical request/result and QueryV1
//! codecs are implemented but not connected to worker I/O; parser-qualified
//! confinement, actual parser invocation, admission witnesses, permits, and
//! serving remain absent.

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod build_identity;
mod parse_protocol;
mod profile;
pub(crate) mod protocol;
mod query_v1;
mod supervisor;
mod worker;

#[cfg(feature = "parser-worker-evidence")]
mod alpha_equivalence;

pub use worker::dispatch_private_parser_worker_v1;

/// Non-default Rust evidence seam for the exact worker control handshake.
///
/// This does not parse a query, mint an admission witness, or expose a serving
/// fallback. The caller supplies an absolute regular-file path, which is opened
/// once and thereafter launched only through the held descriptor.
#[cfg(all(
    feature = "parser-worker-evidence",
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
))]
pub fn exercise_private_parser_worker_handshake_for_evidence(
    executable: &std::path::Path,
    source: &str,
) -> Result<(), String> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Component;

    if !executable.is_absolute()
        || executable
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("evidence executable path must be absolute and normalized".into());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(executable)
        .map_err(|error| format!("open evidence executable: {error}"))?;
    supervisor::exercise_handshake_for_evidence(file, source).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
