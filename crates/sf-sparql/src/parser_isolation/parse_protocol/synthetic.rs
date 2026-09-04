//! Fixed parser-free QueryV1 payload for synthetic transport evidence.

use super::binary::{encode_common_header, write_u16, ContentDigest};
use super::{
    ParseFrameError, ParseRequestV1, DIGEST_LEN, PAYLOAD_DIGEST_OFFSET, RESULT_HEADER_LEN,
    RESULT_MAGIC, RESULT_QUERY_VERSION_OFFSET, RESULT_SUCCESS_KIND,
};
use crate::parser_isolation::query_v1;

pub(crate) const SYNTHETIC_EMPTY_ASK_QUERY_V1: [u8; 100] = [
    // QueryV1 header.
    0x53, 0x46, 0x50, 0x51, 0x57, 0x30, 0x30, 0x31, 0x00, 0x01, 0x00, 0x20, 0x00, 0x20, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x00, // Empty BGP record.
    0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, // ASK root.
    0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x01, // Root edge to the BGP.
    0x00, 0x00, 0x00, 0x00,
];

/// Build the correlated outer header for the fixed payload without constructing
/// a `Query` or invoking the QueryV1 encoder in the child.
pub(crate) fn synthetic_empty_ask_result_header_for(
    request: &ParseRequestV1<'_>,
) -> Result<[u8; RESULT_HEADER_LEN], ParseFrameError> {
    result_header_for_body(
        request,
        SYNTHETIC_EMPTY_ASK_QUERY_V1.len(),
        *ContentDigest::of(&SYNTHETIC_EMPTY_ASK_QUERY_V1).as_bytes(),
    )
}

fn result_header_for_body(
    request: &ParseRequestV1<'_>,
    body_len: usize,
    payload_digest: [u8; DIGEST_LEN],
) -> Result<[u8; RESULT_HEADER_LEN], ParseFrameError> {
    let mut output = [0_u8; RESULT_HEADER_LEN];
    encode_common_header(
        &mut output,
        &RESULT_MAGIC,
        RESULT_SUCCESS_KIND,
        RESULT_HEADER_LEN,
        body_len,
        request.nonce,
        request.source_digest,
    )?;
    output[PAYLOAD_DIGEST_OFFSET..RESULT_QUERY_VERSION_OFFSET].copy_from_slice(&payload_digest);
    write_u16(
        &mut output,
        RESULT_QUERY_VERSION_OFFSET,
        query_v1::WIRE_VERSION,
    );
    Ok(output)
}

#[cfg(feature = "query-v1-transport-mutant-evidence")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SyntheticResultHeaderMutation {
    Nonce,
    SourceDigest,
    PayloadDigest,
}

/// Flip one deterministic bit in exactly one correlation or digest field.
#[cfg(feature = "query-v1-transport-mutant-evidence")]
pub(crate) fn mutate_synthetic_result_header(
    header: &mut [u8; RESULT_HEADER_LEN],
    mutation: SyntheticResultHeaderMutation,
) {
    let offset = match mutation {
        SyntheticResultHeaderMutation::Nonce => super::NONCE_OFFSET,
        SyntheticResultHeaderMutation::SourceDigest => super::SOURCE_DIGEST_OFFSET,
        SyntheticResultHeaderMutation::PayloadDigest => PAYLOAD_DIGEST_OFFSET,
    };
    header[offset] ^= 1;
}

/// Build a success header for a mutant's stack-bounded payload.
#[cfg(feature = "query-v1-transport-mutant-evidence")]
pub(crate) fn synthetic_mutant_result_header_for_payload(
    request: &ParseRequestV1<'_>,
    payload: &[u8],
) -> Result<[u8; RESULT_HEADER_LEN], ParseFrameError> {
    result_header_for_body(
        request,
        payload.len(),
        *ContentDigest::of(payload).as_bytes(),
    )
}

/// Build a success header for a bounded streamed body without allocating it.
#[cfg(feature = "query-v1-transport-mutant-evidence")]
pub(crate) fn synthetic_mutant_result_header_for_body(
    request: &ParseRequestV1<'_>,
    body_len: usize,
    payload_digest: [u8; DIGEST_LEN],
) -> Result<[u8; RESULT_HEADER_LEN], ParseFrameError> {
    result_header_for_body(request, body_len, payload_digest)
}

#[cfg(all(test, feature = "query-v1-transport-mutant-evidence"))]
mod tests {
    use super::*;
    use crate::parser_isolation::parse_protocol::{NONCE_OFFSET, SOURCE_DIGEST_OFFSET};
    use crate::parser_isolation::protocol::HandshakeNonce;

    fn request() -> ParseRequestV1<'static> {
        ParseRequestV1::new(HandshakeNonce::new([7; DIGEST_LEN]), "ASK {}").unwrap()
    }

    #[test]
    fn mutations_change_exactly_one_selected_header_byte() {
        let canonical = synthetic_empty_ask_result_header_for(&request()).unwrap();
        for (mutation, offset) in [
            (SyntheticResultHeaderMutation::Nonce, NONCE_OFFSET),
            (
                SyntheticResultHeaderMutation::SourceDigest,
                SOURCE_DIGEST_OFFSET,
            ),
            (
                SyntheticResultHeaderMutation::PayloadDigest,
                PAYLOAD_DIGEST_OFFSET,
            ),
        ] {
            let mut mutated = canonical;
            mutate_synthetic_result_header(&mut mutated, mutation);
            let changed = canonical
                .iter()
                .zip(mutated.iter())
                .enumerate()
                .filter_map(|(index, (left, right))| (left != right).then_some(index))
                .collect::<Vec<_>>();
            assert_eq!(changed, [offset]);
        }
    }

    #[test]
    fn mutant_payload_header_recomputes_its_digest() {
        let mut invalid = SYNTHETIC_EMPTY_ASK_QUERY_V1;
        invalid[0] ^= 1;
        let header = synthetic_mutant_result_header_for_payload(&request(), &invalid).unwrap();
        assert_eq!(
            &header[PAYLOAD_DIGEST_OFFSET..RESULT_QUERY_VERSION_OFFSET],
            ContentDigest::of(&invalid).as_bytes()
        );
    }
}
