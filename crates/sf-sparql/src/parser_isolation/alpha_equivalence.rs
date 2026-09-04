//! Provisional, private fresh-parse evidence comparison.
//!
//! This module is compiled only for the non-default parser-worker evidence
//! feature. It compares typed `spargebra::Query` values and intentionally has
//! no transport, admission, cache, witness, serving, or receipt authority.

mod compare;

use sha2::{Digest, Sha256};
use spargebra::Query;

use super::profile::ParserWorkerEvidenceProfileV1;

const VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SourceDigestV1([u8; 32]);

impl SourceDigestV1 {
    pub(super) fn of(source: &str) -> Self {
        Self(Sha256::digest(source.as_bytes()).into())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ParseOutcomeV1<'query> {
    Parsed(&'query Query),
    Syntax,
    QueryEnvelope,
    Resource,
    Protocol,
    Deadline,
    Cancellation,
    Signal,
    AbnormalExit,
    Limit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ParseObservationV1<'query> {
    version: u16,
    source: SourceDigestV1,
    profile: ParserWorkerEvidenceProfileV1,
    outcome: ParseOutcomeV1<'query>,
}

impl<'query> ParseObservationV1<'query> {
    pub(super) fn new(
        source: SourceDigestV1,
        profile: ParserWorkerEvidenceProfileV1,
        outcome: ParseOutcomeV1<'query>,
    ) -> Self {
        Self {
            version: VERSION,
            source,
            profile,
            outcome,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AlphaVerdictV1 {
    Equivalent,
    Different,
    Inconclusive,
    Ineligible,
}

pub(super) fn compare(
    left: ParseObservationV1<'_>,
    right: ParseObservationV1<'_>,
) -> AlphaVerdictV1 {
    if left.version != VERSION
        || right.version != VERSION
        || left.source != right.source
        || left.profile != right.profile
    {
        return AlphaVerdictV1::Ineligible;
    }
    match (left.outcome, right.outcome) {
        (ParseOutcomeV1::Syntax, ParseOutcomeV1::Syntax) => AlphaVerdictV1::Equivalent,
        (ParseOutcomeV1::Parsed(_), ParseOutcomeV1::Syntax)
        | (ParseOutcomeV1::Syntax, ParseOutcomeV1::Parsed(_)) => AlphaVerdictV1::Different,
        (ParseOutcomeV1::Parsed(left), ParseOutcomeV1::Parsed(right)) => {
            compare::queries(left, right)
        }
        (left, right) if is_inconclusive(left) || is_inconclusive(right) => {
            AlphaVerdictV1::Inconclusive
        }
        _ => AlphaVerdictV1::Different,
    }
}

fn is_inconclusive(outcome: ParseOutcomeV1<'_>) -> bool {
    matches!(
        outcome,
        ParseOutcomeV1::QueryEnvelope
            | ParseOutcomeV1::Resource
            | ParseOutcomeV1::Protocol
            | ParseOutcomeV1::Deadline
            | ParseOutcomeV1::Cancellation
            | ParseOutcomeV1::Signal
            | ParseOutcomeV1::AbnormalExit
            | ParseOutcomeV1::Limit
    )
}

#[cfg(test)]
mod tests;
