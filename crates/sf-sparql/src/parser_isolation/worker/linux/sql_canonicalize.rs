//! Governed SQL-canonicalization peer: parse and re-render one bounded
//! skeleton through the existing `sf_sql::Dialect::emit_via_ast` primitive,
//! contained by this process's own rlimits/deadline rather than any new
//! semantic depth cap.

use crate::parser_isolation::protocol::HandshakeNonce;
use crate::parser_isolation::sql_canonicalize_protocol::{
    allocate_exact, encode_rejection, encode_success, RequestHeaderV1, SqlRejectionV1,
    REQUEST_HEADER_LEN,
};

use super::{read_exact, require_parent_eof, write_all, WorkerFailure};

pub(super) fn run(expected_nonce: HandshakeNonce) -> Result<(), WorkerFailure> {
    run_with_request(expected_nonce, |request| {
        match request.dialect.to_dialect().emit_via_ast(request.skeleton) {
            Ok(canonical) => match encode_success(request, &canonical) {
                Ok(result) => write_all(libc::STDOUT_FILENO, &result),
                Err(_) => {
                    let rejection = encode_rejection(request, SqlRejectionV1::ResourceExhausted);
                    write_all(libc::STDOUT_FILENO, &rejection)
                }
            },
            Err(_) => {
                let rejection = encode_rejection(request, SqlRejectionV1::Syntax);
                write_all(libc::STDOUT_FILENO, &rejection)
            }
        }
    })
}

pub(super) fn run_with_request(
    expected_nonce: HandshakeNonce,
    emit: impl FnOnce(
        &crate::parser_isolation::sql_canonicalize_protocol::DecodedSqlRequestV1<'_>,
    ) -> Result<(), WorkerFailure>,
) -> Result<(), WorkerFailure> {
    let mut encoded_header = [0_u8; REQUEST_HEADER_LEN];
    read_exact(libc::STDIN_FILENO, &mut encoded_header)?;
    let header = RequestHeaderV1::preflight(&encoded_header).map_err(|_| WorkerFailure)?;

    // The sole request-sized allocation owns exactly the declared skeleton
    // bytes; `preflight` already bounded `body_len` before this call.
    let mut body = allocate_exact(header.body_len()).map_err(|_| WorkerFailure)?;
    read_exact(libc::STDIN_FILENO, &mut body)?;
    require_parent_eof()?;

    let request = header
        .decode_body(&encoded_header, &body)
        .map_err(|_| WorkerFailure)?;
    if request.nonce != expected_nonce {
        return Err(WorkerFailure);
    }

    emit(&request)
}
