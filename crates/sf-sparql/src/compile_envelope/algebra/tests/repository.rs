use spargebra::SparqlParser;

use super::*;

#[test]
fn should_accept_every_checked_in_ontop_query() {
    const QUERIES: &[(&str, &str)] = &[
        (
            "dump",
            include_str!("../../../../../../scripts/ontop/dump.rq"),
        ),
        ("q1", include_str!("../../../../../../scripts/ontop/q1.rq")),
        ("q2", include_str!("../../../../../../scripts/ontop/q2.rq")),
        ("q3", include_str!("../../../../../../scripts/ontop/q3.rq")),
        ("q4", include_str!("../../../../../../scripts/ontop/q4.rq")),
        ("q5", include_str!("../../../../../../scripts/ontop/q5.rq")),
        ("q6", include_str!("../../../../../../scripts/ontop/q6.rq")),
        ("q7", include_str!("../../../../../../scripts/ontop/q7.rq")),
        ("q8", include_str!("../../../../../../scripts/ontop/q8.rq")),
        ("q9", include_str!("../../../../../../scripts/ontop/q9.rq")),
        (
            "q10",
            include_str!("../../../../../../scripts/ontop/q10.rq"),
        ),
        (
            "q11",
            include_str!("../../../../../../scripts/ontop/q11.rq"),
        ),
        (
            "q12",
            include_str!("../../../../../../scripts/ontop/q12.rq"),
        ),
        (
            "q13",
            include_str!("../../../../../../scripts/ontop/q13.rq"),
        ),
        (
            "q14",
            include_str!("../../../../../../scripts/ontop/q14.rq"),
        ),
        (
            "q15",
            include_str!("../../../../../../scripts/ontop/q15.rq"),
        ),
    ];

    for (name, source) in QUERIES {
        let query = SparqlParser::new()
            .parse_query(source)
            .unwrap_or_else(|error| panic!("{name} must parse: {error}"));
        let envelope = AlgebraEnvelopeV1::validate(&query)
            .unwrap_or_else(|error| panic!("{name} must fit: {error}"));

        assert!(envelope.algebra_nodes > 0, "{name}");
        assert!(envelope.max_depth > 0, "{name}");
    }
}

#[test]
fn should_measure_base_and_dataset_payload_without_formatting() {
    let source = concat!(
        "BASE <https://example.test/base/> ",
        "SELECT ?s FROM <https://example.test/default> ",
        "FROM NAMED <https://example.test/named> WHERE { ?s ?p ?o }",
    );
    let query = SparqlParser::new()
        .parse_query(source)
        .expect("query with dataset and base parses");

    let envelope = AlgebraEnvelopeV1::validate(&query).expect("small query fits");

    assert!(envelope.collection_slots >= 3);
    assert!(envelope.retained_payload_bytes >= "https://example.test/base/".len());
}
