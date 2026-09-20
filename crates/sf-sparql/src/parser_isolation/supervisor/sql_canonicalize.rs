//! Parent sequencing for the governed SQL-canonicalization peer. Every
//! pre-terminal failure is contained (signaled, swept, reaped) before this
//! module returns, matching the existing QueryV1 transport's discipline.
//!
//! Async and bounded-step by construction: every wait against the child
//! (handshake, request write, result read, wait/reap) is one `<=10ms`-capped
//! poll plus at most one syscall attempt (`io::{write_step, read_step,
//! expect_eof_step}`, `lifecycle::wait_step`), looped here with a real
//! `checkpoint()` against the caller's own control and a cooperative yield
//! (`exec_core::driver::cooperative_yield`) between `Pending` steps -- never
//! a monolithic blocking call, and never a thread-local held across an
//! `.await` (the real control is threaded explicitly as a parameter).

use sf_core::query_control::QueryControl;
use sf_sql::source_work::SourceWork;

use super::executable::PreparedParserExecutable;
use super::io::StepOutcome;
use super::lifecycle::PidfdPollStep;
use super::linux;
use super::SupervisorError;
use crate::parser_isolation::profile::{control_ready_profile_candidate_digest, v1_limits};
use crate::parser_isolation::protocol::{
    verify_ready, HandshakeNonce, HelloFrame, ReadyFrame, DIGEST_LEN, FRAME_LEN,
};
use crate::parser_isolation::sql_canonicalize_protocol::{
    allocate_exact, ResultHeaderV1, SqlCanonicalizeOutcomeV1, SqlCanonicalizeRequestV1,
    SqlDialectCodeV1, REQUEST_HEADER_LEN, RESULT_HEADER_LEN,
};

/// Every bounded step is capped at this poll timeout, independent of the
/// legacy thread-local `runtime::poll_timeout`: short enough that no single
/// step can meaningfully block the calling async task's own thread.
const STEP_TIMEOUT_MS: i32 = 10;

/// Independent copy of `handshake::generate_nonce`, kept local so this new
/// peer does not widen a pre-existing shared supervisor module's visibility.
fn generate_nonce() -> Result<HandshakeNonce, SupervisorError> {
    let mut nonce = [0_u8; DIGEST_LEN];
    let mut offset = 0;
    while offset < nonce.len() {
        crate::parser_isolation::runtime::checkpoint()?;
        let count = unsafe {
            libc::getrandom(nonce[offset..].as_mut_ptr().cast(), nonce.len() - offset, 0)
        };
        if count > 0 {
            offset += count as usize;
            continue;
        }
        if count == 0 {
            return Err(SupervisorError::InvalidState(
                "kernel entropy returned a zero-byte SQL canonicalize nonce",
            ));
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(SupervisorError::operation(
                "generate SQL canonicalize handshake nonce",
            )(error));
        }
    }
    Ok(HandshakeNonce::new(nonce))
}

fn checkpoint(control: &dyn QueryControl) -> Result<(), SupervisorError> {
    control
        .checkpoint()
        .map_err(SupervisorError::RequestControl)
}

/// Map a `SourceWork` charge failure. A real request-control terminal cause
/// is preserved exactly; any other accounting failure becomes one fixed
/// static reason, so this never leaks a per-call allocation.
fn charge_error(error: sf_sql::Error) -> SupervisorError {
    match error {
        sf_sql::Error::QueryControl(cause) => SupervisorError::RequestControl(cause),
        _ => SupervisorError::InvalidState("SQL canonicalize source accounting failed"),
    }
}

/// Checkpoint the caller's real control mid-flight, routing a failure through
/// containment so a cleanup error takes precedence and the child is always
/// signalled/swept/reaped before we return (never left to the `Drop`
/// backstop, which cannot surface its own failure).
fn contained_checkpoint(
    process: &mut super::lifecycle::ParserWorkerProcess,
    control: &dyn QueryControl,
) -> Result<(), SupervisorError> {
    match control.checkpoint() {
        Ok(()) => Ok(()),
        Err(cause) => Err(process.contain_caller_failure(SupervisorError::RequestControl(cause))),
    }
}

/// Drive one admitted outbound transfer to completion a bounded step at a
/// time, checkpointing the real control and cooperatively yielding between
/// `Pending` steps. The whole frame is admitted against the immutable
/// cumulative input cap by `reserve_write` BEFORE the first byte moves.
async fn write_bounded(
    process: &mut super::lifecycle::ParserWorkerProcess,
    bytes: &[u8],
    control: &dyn QueryControl,
) -> Result<(), SupervisorError> {
    let mut transfer = match process.reserve_write(bytes) {
        Ok(transfer) => transfer,
        Err(error) => return Err(process.contain_caller_failure(error)),
    };
    loop {
        contained_checkpoint(process, control)?;
        if process.write_step_contained(bytes, &mut transfer, STEP_TIMEOUT_MS)?
            == StepOutcome::Complete
        {
            return match process.verify_write_total(transfer) {
                Ok(()) => Ok(()),
                Err(error) => Err(process.contain_caller_failure(error)),
            };
        }
        crate::exec_core::cooperative_yield().await;
    }
}

async fn read_bounded(
    process: &mut super::lifecycle::ParserWorkerProcess,
    output: &mut [u8],
    control: &dyn QueryControl,
) -> Result<(), SupervisorError> {
    let mut transfer = match process.reserve_read(output.len()) {
        Ok(transfer) => transfer,
        Err(error) => return Err(process.contain_caller_failure(error)),
    };
    loop {
        contained_checkpoint(process, control)?;
        if process.read_step_contained(output, &mut transfer, STEP_TIMEOUT_MS)?
            == StepOutcome::Complete
        {
            return match process.verify_read_total(transfer) {
                Ok(()) => Ok(()),
                Err(error) => Err(process.contain_caller_failure(error)),
            };
        }
        crate::exec_core::cooperative_yield().await;
    }
}

async fn expect_eof_bounded(
    process: &mut super::lifecycle::ParserWorkerProcess,
    control: &dyn QueryControl,
) -> Result<(), SupervisorError> {
    loop {
        contained_checkpoint(process, control)?;
        match process.expect_eof_step_contained(STEP_TIMEOUT_MS)? {
            StepOutcome::Complete => return Ok(()),
            StepOutcome::Pending => crate::exec_core::cooperative_yield().await,
        }
    }
}

async fn wait_bounded(
    process: &mut super::lifecycle::ParserWorkerProcess,
    control: &dyn QueryControl,
) -> Result<std::process::ExitStatus, SupervisorError> {
    loop {
        contained_checkpoint(process, control)?;
        // `wait_step` itself clamps to the immutable deadline and
        // terminates/reaps on expiry, so no separate pre-check is needed.
        match process.wait_step(STEP_TIMEOUT_MS)? {
            PidfdPollStep::Exited => return process.finish_wait_after_exit(),
            PidfdPollStep::Pending => crate::exec_core::cooperative_yield().await,
        }
    }
}

pub(super) async fn canonicalize(
    executable: &PreparedParserExecutable,
    dialect: SqlDialectCodeV1,
    skeleton: &str,
    control: &dyn QueryControl,
    work: SourceWork<'_>,
) -> Result<String, SupervisorError> {
    canonicalize_with_peer(
        executable,
        dialect,
        skeleton,
        control,
        work,
        Peer::Production(std::marker::PhantomData),
    )
    .await
}

#[cfg(feature = "sql-canonicalize-evidence")]
pub(super) async fn canonicalize_evidence(
    executable: &PreparedParserExecutable,
    mode: crate::parser_isolation::SqlCanonicalizeEvidenceMode,
    state: &crate::parser_isolation::SqlCanonicalizeEvidenceState,
    control: &dyn QueryControl,
    work: SourceWork<'_>,
) -> Result<String, SupervisorError> {
    canonicalize_with_peer(
        executable,
        SqlDialectCodeV1::Sqlite,
        "SELECT 1 AS c0",
        control,
        work,
        Peer::Evidence { mode, state },
    )
    .await
}

#[derive(Clone, Copy)]
enum Peer<'a> {
    Production(std::marker::PhantomData<&'a ()>),
    #[cfg(feature = "sql-canonicalize-evidence")]
    Evidence {
        mode: crate::parser_isolation::SqlCanonicalizeEvidenceMode,
        state: &'a crate::parser_isolation::SqlCanonicalizeEvidenceState,
    },
}

async fn canonicalize_with_peer(
    executable: &PreparedParserExecutable,
    dialect: SqlDialectCodeV1,
    skeleton: &str,
    control: &dyn QueryControl,
    work: SourceWork<'_>,
    peer: Peer<'_>,
) -> Result<String, SupervisorError> {
    checkpoint(control)?;
    let limits = v1_limits();
    let nonce = generate_nonce()?;
    let hello = HelloFrame::new(
        nonce,
        executable.identity().build_identity(),
        control_ready_profile_candidate_digest(),
        limits,
    );
    // Prospective request-side charge: `SqlCanonicalizeRequestV1::new`'s one
    // SHA-256 digest pass over `skeleton`, plus `encode`'s one allocation of
    // `REQUEST_HEADER_LEN + skeleton.len()` and one copy of `skeleton` into
    // it -- charged before either operation runs, not approximated.
    work.charge(REQUEST_HEADER_LEN).map_err(charge_error)?;
    work.product(skeleton.len(), 2).map_err(charge_error)?;
    let request = SqlCanonicalizeRequestV1::new(nonce, dialect, skeleton).map_err(|_| {
        SupervisorError::InvalidLimits("SQL canonicalize skeleton exceeds the whole-frame ceiling")
    })?;
    let encoded_request = request.encode().map_err(SupervisorError::from)?;
    let source_digest = request.source_digest();

    let mut process = match peer {
        Peer::Production(_) => linux::spawn_sql_canonicalize_v1(executable, limits)?,
        #[cfg(feature = "sql-canonicalize-evidence")]
        Peer::Evidence { .. } => linux::spawn_sql_canonicalize_evidence_v1(executable, limits)?,
    };
    #[cfg(feature = "sql-canonicalize-evidence")]
    if let Peer::Evidence { mode, .. } = peer {
        write_bounded(&mut process, &mode.encode(), control).await?;
    }
    write_bounded(&mut process, &hello.encode(), control).await?;

    let mut encoded_ready = [0_u8; FRAME_LEN];
    read_bounded(&mut process, &mut encoded_ready, control).await?;
    let ready = match ReadyFrame::decode(&encoded_ready) {
        Ok(ready) => ready,
        Err(error) => return Err(process.contain_live_failure(error.into())),
    };
    if let Err(error) = verify_ready(&hello, &ready) {
        return Err(process.contain_live_failure(error.into()));
    }

    write_bounded(&mut process, &encoded_request, control).await?;
    process.close_stdin();

    #[cfg(feature = "sql-canonicalize-evidence")]
    if let Peer::Evidence { state, .. } = peer {
        let mut request_ready = [0_u8; 1];
        read_bounded(&mut process, &mut request_ready, control).await?;
        if request_ready != [0xa5] {
            return Err(process.contain_live_failure(SupervisorError::InvalidState(
                "SQL evidence peer emitted an invalid request barrier",
            )));
        }
        let pidfd = match process.duplicate_pidfd() {
            Ok(pidfd) => pidfd,
            Err(error) => return Err(process.contain_live_failure(error)),
        };
        state.mark_request_ready(pidfd);
    }

    let mut encoded_header = [0_u8; RESULT_HEADER_LEN];
    read_bounded(&mut process, &mut encoded_header, control).await?;
    let header = match ResultHeaderV1::preflight(&encoded_header) {
        Ok(header) => header,
        Err(error) => return Err(process.contain_live_failure(error.into())),
    };

    // Charge the peer's declared body against the remaining whole-worker
    // output budget (existing, independent byte-cap accounting)...
    process.ensure_can_receive(header.body_len())?;
    // ...and PROSPECTIVELY charge SourceWork for the four actual linear body
    // operations: zero-initializing the exact allocation, copying from the
    // pipe, validating UTF-8, and copying into the returned owned String.
    // The result echoes the request digest; it does not hash its own body.
    if let Err(error) = work
        .charge(RESULT_HEADER_LEN)
        .and_then(|()| work.product(header.body_len(), 4))
    {
        return Err(process.contain_caller_failure(charge_error(error)));
    }

    let mut body = match allocate_exact(header.body_len()) {
        Ok(body) => body,
        Err(error) => return Err(process.contain_live_failure(error.into())),
    };
    read_bounded(&mut process, &mut body, control).await?;
    expect_eof_bounded(&mut process, control).await?;

    let status = wait_bounded(&mut process, control).await?;
    if !status.success() {
        return Err(SupervisorError::InvalidState(
            "SQL canonicalize peer did not exit successfully",
        ));
    }

    let outcome = header
        .decode_result_body(&encoded_header, &body, nonce, source_digest)
        .map_err(SupervisorError::from)?;
    match outcome {
        SqlCanonicalizeOutcomeV1::Success(sql) => Ok(sql),
        SqlCanonicalizeOutcomeV1::Rejected(rejection) => {
            Err(SupervisorError::SqlRejected(rejection))
        }
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
#[path = "sql_canonicalize_tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_three_serve_wired_dialects_have_a_wire_code() {
        // A real spawn round trip needs the held executable's own
        // `dispatch_private_parser_worker_v1` entry, which only exists in a
        // product binary's `main` (see `crates/sf-cli/tests/
        // sql_canonicalize_isolation.rs`, mirroring `describe_parser_
        // determinism.rs`) -- not this crate's own test binary. This module
        // keeps only the pure, executable-free coverage.
        for dialect in [
            sf_sql::Dialect::Postgres,
            sf_sql::Dialect::Sqlite,
            sf_sql::Dialect::MySql,
        ] {
            assert!(SqlDialectCodeV1::from_dialect(dialect).is_some());
        }
        for dialect in [
            sf_sql::Dialect::Redshift,
            sf_sql::Dialect::DuckDb,
            sf_sql::Dialect::Oracle,
        ] {
            assert!(SqlDialectCodeV1::from_dialect(dialect).is_none());
        }
    }
}
