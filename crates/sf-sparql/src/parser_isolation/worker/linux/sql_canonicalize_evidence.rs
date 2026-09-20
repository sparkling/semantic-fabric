//! Closed hostile result behaviors for the SQL canonicalization peer.

use crate::parser_isolation::protocol::HandshakeNonce;
use crate::parser_isolation::sql_canonicalize_evidence::SqlCanonicalizeEvidenceMode;
use crate::parser_isolation::sql_canonicalize_protocol::{
    encode_oversized_result_for_evidence, encode_success, RESULT_HEADER_LEN,
};

use super::sql_canonicalize::run_with_request;
use super::{write_all, WorkerFailure};

const REQUEST_READY: [u8; 1] = [0xa5];

pub(super) fn run(
    mode: SqlCanonicalizeEvidenceMode,
    expected_nonce: HandshakeNonce,
) -> Result<(), WorkerFailure> {
    run_with_request(expected_nonce, |request| {
        write_all(libc::STDOUT_FILENO, &REQUEST_READY)?;
        match mode {
            SqlCanonicalizeEvidenceMode::HoldAfterRequest => stall_on_private_futex(),
            SqlCanonicalizeEvidenceMode::MalformedResult => {
                write_all(libc::STDOUT_FILENO, &[0; RESULT_HEADER_LEN])
            }
            SqlCanonicalizeEvidenceMode::TruncatedResult => {
                let result = encode_success(request, "SELECT 1").map_err(|_| WorkerFailure)?;
                write_all(libc::STDOUT_FILENO, &result[..result.len() - 1])
            }
            SqlCanonicalizeEvidenceMode::OversizedResult => write_all(
                libc::STDOUT_FILENO,
                &encode_oversized_result_for_evidence(request),
            ),
        }
    })
}

fn stall_on_private_futex() -> Result<(), WorkerFailure> {
    let word = 0_u32;
    loop {
        let result = unsafe {
            libc::syscall(
                libc::SYS_futex,
                &word as *const u32,
                libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
                0,
                std::ptr::null::<libc::timespec>(),
            )
        };
        if result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return Err(WorkerFailure);
    }
}
