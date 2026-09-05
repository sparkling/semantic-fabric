//! Fixed, qualification-only corpus and terminal parser observations.
//!
//! This module never serializes or returns a parsed query. The deliberately
//! small V1 corpus is a bounded observation starter, not the complete grammar-
//! family corpus required to qualify a parser policy or admit serving.

use sha2::{Digest, Sha256};

use super::protocol::{HandshakeNonce, DIGEST_LEN};

pub(super) const OBSERVATION_FRAME_LEN: usize = 80;
const OBSERVATION_MAGIC: [u8; 8] = *b"SFPOBS01";
const OBSERVATION_VERSION: u16 = 1;
const OUTCOME_OFFSET: usize = 10;
const FLAGS_OFFSET: usize = 11;
const FRAME_LEN_OFFSET: usize = 12;
const NONCE_OFFSET: usize = 16;
const SOURCE_DIGEST_OFFSET: usize = NONCE_OFFSET + DIGEST_LEN;

const _: () = assert!(SOURCE_DIGEST_OFFSET + DIGEST_LEN == OBSERVATION_FRAME_LEN);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum ParserObservationOutcomeV1 {
    Parsed = 1,
    SyntaxRejected = 2,
}

impl ParserObservationOutcomeV1 {
    fn decode(value: u8) -> Result<Self, &'static str> {
        match value {
            1 => Ok(Self::Parsed),
            2 => Ok(Self::SyntaxRejected),
            _ => Err("parser observation outcome is unknown"),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ParserObservationCaseV1 {
    pub(super) source: &'static str,
    pub(super) expected: ParserObservationOutcomeV1,
}

pub(super) const PARSER_OBSERVATION_CORPUS_V1: &[ParserObservationCaseV1] = &[
    ParserObservationCaseV1 {
        source: concat!(
            "PREFIX ex: <https://example.test/> ",
            "SELECT DISTINCT ?s (COUNT(?o) AS ?count) WHERE { ",
            "VALUES ?s { ex:a ex:b } OPTIONAL { ?s ex:p ?o . FILTER(?o != ex:z) } ",
            "} GROUP BY ?s ORDER BY ?s LIMIT 5",
        ),
        expected: ParserObservationOutcomeV1::Parsed,
    },
    ParserObservationCaseV1 {
        source: concat!(
            "PREFIX ex: <https://example.test/> ",
            "ASK FROM <https://example.test/default> WHERE { ?s (ex:p/ex:q)+ ?o }",
        ),
        expected: ParserObservationOutcomeV1::Parsed,
    },
    ParserObservationCaseV1 {
        source: "DESCRIBE ?s WHERE { ?s <https://example.test/p> ?o }",
        expected: ParserObservationOutcomeV1::Parsed,
    },
    ParserObservationCaseV1 {
        source: concat!(
            "CONSTRUCT { ?s <https://example.test/copy> _:result } WHERE { ",
            "?s <https://example.test/source> ?o }",
        ),
        expected: ParserObservationOutcomeV1::Parsed,
    },
    ParserObservationCaseV1 {
        source: concat!(
            "SELECT * WHERE { <<( ?s <https://example.test/p> \"nested\" )>> ",
            "<https://example.test/asserted> ?certainty }",
        ),
        expected: ParserObservationOutcomeV1::Parsed,
    },
    ParserObservationCaseV1 {
        source: concat!(
            "BASE <https://example.test/root/> SELECT * WHERE { ",
            "{ ?s <p> <relative> } UNION { GRAPH ?g { ?s <q> ?o } } ",
            "MINUS { ?s <removed> ?x } }",
        ),
        expected: ParserObservationOutcomeV1::Parsed,
    },
    ParserObservationCaseV1 {
        source: "SELECT * WHERE { ?s ?p }",
        expected: ParserObservationOutcomeV1::SyntaxRejected,
    },
];

/// Aggregate-only result from fresh-child observation of the fixed corpus.
///
/// It carries no query, parser error text, admission witness, or policy claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParserObservationSummaryV1 {
    case_count: u16,
    parsed_count: u16,
    syntax_rejection_count: u16,
    corpus_digest: [u8; DIGEST_LEN],
}

impl ParserObservationSummaryV1 {
    pub(super) fn from_verified_counts(
        parsed_count: u16,
        syntax_rejection_count: u16,
    ) -> Result<Self, &'static str> {
        let case_count = u16::try_from(PARSER_OBSERVATION_CORPUS_V1.len())
            .map_err(|_| "parser observation corpus count exceeds u16")?;
        if parsed_count
            .checked_add(syntax_rejection_count)
            .ok_or("parser observation outcome count overflowed")?
            != case_count
        {
            return Err("parser observation outcome count drifted");
        }
        Ok(Self {
            case_count,
            parsed_count,
            syntax_rejection_count,
            corpus_digest: corpus_digest(),
        })
    }

    pub const fn case_count(self) -> u16 {
        self.case_count
    }

    pub const fn parsed_count(self) -> u16 {
        self.parsed_count
    }

    pub const fn syntax_rejection_count(self) -> u16 {
        self.syntax_rejection_count
    }

    pub const fn corpus_digest(self) -> [u8; DIGEST_LEN] {
        self.corpus_digest
    }
}

pub(super) fn sealed_expected_outcome(source: &str) -> Option<ParserObservationOutcomeV1> {
    PARSER_OBSERVATION_CORPUS_V1
        .iter()
        .find(|case| case.source == source)
        .map(|case| case.expected)
}

pub(super) fn encode_observation(
    nonce: HandshakeNonce,
    source: &str,
    outcome: ParserObservationOutcomeV1,
) -> [u8; OBSERVATION_FRAME_LEN] {
    let mut frame = [0_u8; OBSERVATION_FRAME_LEN];
    frame[..OBSERVATION_MAGIC.len()].copy_from_slice(&OBSERVATION_MAGIC);
    frame[8..10].copy_from_slice(&OBSERVATION_VERSION.to_be_bytes());
    frame[OUTCOME_OFFSET] = outcome as u8;
    frame[FRAME_LEN_OFFSET..NONCE_OFFSET]
        .copy_from_slice(&(OBSERVATION_FRAME_LEN as u32).to_be_bytes());
    frame[NONCE_OFFSET..SOURCE_DIGEST_OFFSET].copy_from_slice(nonce.correlation_bytes());
    frame[SOURCE_DIGEST_OFFSET..].copy_from_slice(&Sha256::digest(source.as_bytes()));
    frame
}

pub(super) fn decode_observation_for(
    frame: &[u8],
    nonce: HandshakeNonce,
    source: &str,
) -> Result<ParserObservationOutcomeV1, &'static str> {
    if frame.len() != OBSERVATION_FRAME_LEN {
        return Err("parser observation frame length is invalid");
    }
    if frame[..OBSERVATION_MAGIC.len()] != OBSERVATION_MAGIC
        || u16::from_be_bytes(frame[8..10].try_into().expect("fixed slice")) != OBSERVATION_VERSION
        || frame[FLAGS_OFFSET] != 0
        || u32::from_be_bytes(
            frame[FRAME_LEN_OFFSET..NONCE_OFFSET]
                .try_into()
                .expect("fixed slice"),
        ) != OBSERVATION_FRAME_LEN as u32
    {
        return Err("parser observation frame header is invalid");
    }
    if frame[NONCE_OFFSET..SOURCE_DIGEST_OFFSET] != nonce.correlation_bytes()[..] {
        return Err("parser observation nonce does not match");
    }
    if frame[SOURCE_DIGEST_OFFSET..] != Sha256::digest(source.as_bytes())[..] {
        return Err("parser observation source digest does not match");
    }
    ParserObservationOutcomeV1::decode(frame[OUTCOME_OFFSET])
}

fn corpus_digest() -> [u8; DIGEST_LEN] {
    let mut digest = Sha256::new();
    digest.update(b"semantic-fabric/parser-observation-corpus/v1\0");
    for case in PARSER_OBSERVATION_CORPUS_V1 {
        digest.update((case.source.len() as u64).to_be_bytes());
        digest.update(case.source.as_bytes());
        digest.update([case.expected as u8]);
    }
    digest.finalize().into()
}

#[cfg(test)]
mod tests {
    use spargebra::SparqlParser;

    use super::*;

    #[test]
    fn sealed_outcomes_match_the_pinned_parser_without_exposing_queries() {
        for case in PARSER_OBSERVATION_CORPUS_V1 {
            let actual = if SparqlParser::new().parse_query(case.source).is_ok() {
                ParserObservationOutcomeV1::Parsed
            } else {
                ParserObservationOutcomeV1::SyntaxRejected
            };
            assert_eq!(actual, case.expected);
        }
        assert_eq!(sealed_expected_outcome("ASK {}"), None);
    }

    #[test]
    fn observation_frame_binds_nonce_source_and_closed_outcome() {
        let nonce = HandshakeNonce::new([7; DIGEST_LEN]);
        let source = PARSER_OBSERVATION_CORPUS_V1[0].source;
        let frame = encode_observation(nonce, source, ParserObservationOutcomeV1::Parsed);
        assert_eq!(
            decode_observation_for(&frame, nonce, source),
            Ok(ParserObservationOutcomeV1::Parsed)
        );

        let mut wrong_nonce = frame;
        wrong_nonce[NONCE_OFFSET] ^= 1;
        assert_eq!(
            decode_observation_for(&wrong_nonce, nonce, source),
            Err("parser observation nonce does not match")
        );

        let mut unknown = frame;
        unknown[OUTCOME_OFFSET] = 3;
        assert_eq!(
            decode_observation_for(&unknown, nonce, source),
            Err("parser observation outcome is unknown")
        );
    }
}
