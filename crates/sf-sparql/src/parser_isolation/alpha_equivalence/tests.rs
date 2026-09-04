use spargebra::{Query, SparqlParser};

use super::super::profile::{parser_worker_evidence_profile_v1, ParserWorkerEvidenceProfileV1};
use super::super::protocol::ParserProfileDigest;
use super::*;

mod variants;

fn parse(source: &str) -> Query {
    SparqlParser::new()
        .parse_query(source)
        .unwrap_or_else(|error| panic!("invalid alpha-equivalence fixture: {error}"))
}

fn observation<'query>(
    source: &str,
    outcome: ParseOutcomeV1<'query>,
) -> ParseObservationV1<'query> {
    ParseObservationV1::new(
        SourceDigestV1::of(source),
        parser_worker_evidence_profile_v1(),
        outcome,
    )
}

#[test]
fn outcome_matrix_is_closed_before_typed_comparison() {
    let query = parse("ASK { ?s ?p ?o }");
    let source = "ASK { ?s ?p ?o }";
    assert_eq!(
        compare(
            observation(source, ParseOutcomeV1::Syntax),
            observation(source, ParseOutcomeV1::Syntax)
        ),
        AlphaVerdictV1::Equivalent
    );
    assert_eq!(
        compare(
            observation(source, ParseOutcomeV1::Parsed(&query)),
            observation(source, ParseOutcomeV1::Syntax)
        ),
        AlphaVerdictV1::Different
    );
    for outcome in [
        ParseOutcomeV1::QueryEnvelope,
        ParseOutcomeV1::Resource,
        ParseOutcomeV1::Protocol,
        ParseOutcomeV1::Deadline,
        ParseOutcomeV1::Cancellation,
        ParseOutcomeV1::Signal,
        ParseOutcomeV1::AbnormalExit,
        ParseOutcomeV1::Limit,
    ] {
        assert_eq!(
            compare(
                observation(source, outcome),
                observation(source, ParseOutcomeV1::Syntax)
            ),
            AlphaVerdictV1::Inconclusive
        );
    }
}

#[test]
fn correlation_precedes_every_outcome_result() {
    let query = parse("ASK {}");
    let profile = ParserWorkerEvidenceProfileV1::new(ParserProfileDigest::new([7; 32]));
    assert_eq!(
        compare(
            observation("ASK {}", ParseOutcomeV1::Syntax),
            observation("ASK { }", ParseOutcomeV1::Syntax),
        ),
        AlphaVerdictV1::Ineligible
    );
    assert_eq!(
        compare(
            observation("ASK {}", ParseOutcomeV1::Parsed(&query)),
            ParseObservationV1::new(
                SourceDigestV1::of("ASK {}"),
                profile,
                ParseOutcomeV1::Syntax
            ),
        ),
        AlphaVerdictV1::Ineligible
    );
}

#[test]
fn generated_describe_variables_and_pattern_blanks_alpha_map() {
    let source = "DESCRIBE <https://example.test/s> WHERE { [] <https://example.test/p> ?o FILTER EXISTS { ?o <https://example.test/q> [] } }";
    let left = parse(source);
    let right = parse(source);
    assert_eq!(
        compare(
            observation(source, ParseOutcomeV1::Parsed(&left)),
            observation(source, ParseOutcomeV1::Parsed(&right))
        ),
        AlphaVerdictV1::Equivalent
    );
}

#[test]
fn select_aliases_are_exact_and_query_variables_are_one_bijection() {
    let left = parse("SELECT (?s AS ?output) WHERE { ?s <https://example.test/p> ?s }");
    let renamed_output = parse("SELECT (?s AS ?other) WHERE { ?s <https://example.test/p> ?s }");
    let same_source = "synthetic same source";
    assert_eq!(
        compare(
            observation(same_source, ParseOutcomeV1::Parsed(&left)),
            observation(same_source, ParseOutcomeV1::Parsed(&renamed_output))
        ),
        AlphaVerdictV1::Different
    );

    let shared = parse("ASK { ?s <https://example.test/p> ?s }");
    let alpha = parse("ASK { ?x <https://example.test/p> ?x }");
    let split = parse("ASK { ?x <https://example.test/p> ?y }");
    assert_eq!(
        compare(
            observation(same_source, ParseOutcomeV1::Parsed(&shared)),
            observation(same_source, ParseOutcomeV1::Parsed(&alpha))
        ),
        AlphaVerdictV1::Equivalent
    );
    assert_eq!(
        compare(
            observation(same_source, ParseOutcomeV1::Parsed(&shared)),
            observation(same_source, ParseOutcomeV1::Parsed(&split))
        ),
        AlphaVerdictV1::Different
    );
}

#[test]
fn construct_template_blank_nodes_are_disjoint_from_pattern_blank_nodes() {
    let left = parse("CONSTRUCT { _:template <https://example.test/p> ?o } WHERE { _:pattern <https://example.test/q> ?o }");
    let right = parse("CONSTRUCT { _:pattern <https://example.test/p> ?o } WHERE { _:template <https://example.test/q> ?o }");
    let source = "synthetic construct namespace check";
    assert_eq!(
        compare(
            observation(source, ParseOutcomeV1::Parsed(&left)),
            observation(source, ParseOutcomeV1::Parsed(&right))
        ),
        AlphaVerdictV1::Equivalent
    );
}

#[test]
fn exact_discriminants_options_and_scalar_values_do_not_normalize() {
    let source = "synthetic exactness check";
    let distinct =
        parse("SELECT DISTINCT ?s WHERE { ?s <https://example.test/p> \"one\"@en } LIMIT 2");
    let reduced =
        parse("SELECT REDUCED ?s WHERE { ?s <https://example.test/p> \"one\"@en } LIMIT 2");
    let changed_literal =
        parse("SELECT DISTINCT ?s WHERE { ?s <https://example.test/p> \"two\"@en } LIMIT 2");
    assert_eq!(
        compare(
            observation(source, ParseOutcomeV1::Parsed(&distinct)),
            observation(source, ParseOutcomeV1::Parsed(&reduced))
        ),
        AlphaVerdictV1::Different
    );
    assert_eq!(
        compare(
            observation(source, ParseOutcomeV1::Parsed(&distinct)),
            observation(source, ParseOutcomeV1::Parsed(&changed_literal))
        ),
        AlphaVerdictV1::Different
    );
}

#[test]
fn bounded_or_failed_traversal_reservation_is_inconclusive() {
    let source = "ASK { ?s ?p ?o }";
    let left = parse(source);
    let right = parse(source);
    assert_eq!(
        super::compare::queries_with_limits(&left, &right, 0, 0),
        AlphaVerdictV1::Inconclusive
    );

    super::compare::fail_next_work_reservation();
    assert_eq!(
        compare(
            observation(source, ParseOutcomeV1::Parsed(&left)),
            observation(source, ParseOutcomeV1::Parsed(&right))
        ),
        AlphaVerdictV1::Inconclusive
    );

    assert_eq!(
        super::compare::identity_budget_exhaustion_verdict(),
        AlphaVerdictV1::Inconclusive
    );
}
