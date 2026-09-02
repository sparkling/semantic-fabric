use super::*;

// --- form_param ---------------------------------------------------------------

#[test]
fn form_param_extracts_the_named_key() {
    assert_eq!(
        form_param("query=SELECT+%2A", "query"),
        Some("SELECT *".to_owned())
    );
}

#[test]
fn form_param_returns_none_when_key_absent() {
    assert_eq!(form_param("other=1", "query"), None);
}

#[test]
fn form_param_rejects_the_key_when_repeated() {
    assert_eq!(form_param("query=first&query=second", "query"), None);
}

#[test]
fn form_param_rejects_every_extra_parameter() {
    for key in ["default-graph-uri", "named-graph-uri", "version", "other"] {
        assert_eq!(
            form_param(&format!("query=ASK+%7B%7D&{key}=1"), "query"),
            None
        );
    }
}

#[test]
fn form_param_decodes_plus_and_percent_encoding() {
    assert_eq!(
        form_param("query=a+b%26c", "query"),
        Some("a b&c".to_owned())
    );
}

#[test]
fn form_param_on_empty_string_is_none() {
    assert_eq!(form_param("", "query"), None);
}

// --- negotiate_results --------------------------------------------------------

#[test]
fn negotiate_results_defaults_to_json_when_no_accept_header() {
    assert_eq!(negotiate_results(None), QueryResultsFormat::Json);
}

#[test]
fn negotiate_results_defaults_to_json_for_an_unrecognised_accept() {
    assert_eq!(
        negotiate_results(Some("text/plain")),
        QueryResultsFormat::Json
    );
}

#[test]
fn negotiate_results_picks_xml_for_sparql_results_xml() {
    assert_eq!(
        negotiate_results(Some("application/sparql-results+xml")),
        QueryResultsFormat::Xml
    );
}

#[test]
fn negotiate_results_picks_xml_for_generic_application_xml() {
    assert_eq!(
        negotiate_results(Some("application/xml")),
        QueryResultsFormat::Xml
    );
}

#[test]
fn negotiate_results_picks_tsv() {
    assert_eq!(
        negotiate_results(Some("text/tab-separated-values")),
        QueryResultsFormat::Tsv
    );
}

#[test]
fn negotiate_results_picks_csv() {
    assert_eq!(negotiate_results(Some("text/csv")), QueryResultsFormat::Csv);
}

#[test]
fn negotiate_results_is_case_insensitive() {
    assert_eq!(negotiate_results(Some("TEXT/CSV")), QueryResultsFormat::Csv);
}

#[test]
fn negotiate_results_first_match_wins_on_a_multi_value_accept_header() {
    // XML is checked before TSV/CSV in negotiate_results's own ordering, so
    // a header offering both must resolve to XML, not whichever appears
    // first in the (unordered) Accept string.
    assert_eq!(
        negotiate_results(Some("text/csv, application/sparql-results+xml")),
        QueryResultsFormat::Xml
    );
}

// --- negotiate_rdf ------------------------------------------------------------

#[test]
fn negotiate_rdf_defaults_to_turtle_when_no_accept_header() {
    assert_eq!(negotiate_rdf(None), RdfFormat::Turtle);
}

#[test]
fn negotiate_rdf_defaults_to_turtle_for_an_unrecognised_accept() {
    assert_eq!(negotiate_rdf(Some("text/plain")), RdfFormat::Turtle);
}

#[test]
fn negotiate_rdf_picks_json_ld() {
    assert_eq!(
        negotiate_rdf(Some("application/ld+json")),
        RdfFormat::JsonLd
    );
}

#[test]
fn negotiate_rdf_picks_ntriples() {
    assert_eq!(
        negotiate_rdf(Some("application/n-triples")),
        RdfFormat::NTriples
    );
}

#[test]
fn negotiate_rdf_is_case_insensitive() {
    assert_eq!(
        negotiate_rdf(Some("APPLICATION/N-TRIPLES")),
        RdfFormat::NTriples
    );
}
