use spargebra::{Query, SparqlParser};

use super::*;
use crate::compile_envelope::algebra::AlgebraEnvelopeV1;

fn parse(input: &str) -> Query {
    SparqlParser::new()
        .parse_query(input)
        .expect("fixture must be accepted by the pinned parser")
}

fn generous_limits() -> Limits {
    Limits {
        prefix_declarations: 32,
        prefix_bindings: 32,
        direct_iri_materialization_bytes: 64 * 1024,
    }
}

fn assert_limit(error: CompileEnvelopeError, dimension: CompileEnvelopeLimit) {
    assert!(matches!(
        error,
        CompileEnvelopeError::LimitExceeded {
            dimension: actual,
            ..
        } if actual == dimension
    ));
}

#[test]
fn parser_view_matches_global_unicode_decoding_behavior() {
    assert_eq!(parser_view::parser_view("x\\u0041\\U0001F600y"), "xA😀y");
    assert_eq!(parser_view::parser_view("x\\uD800y"), "x\\uD800y");
    assert_eq!(parser_view::parser_view("x\\u12"), "x\\u12");
    assert_eq!(parser_view::parser_view("plain"), "plain");
}

#[test]
fn unicode_decoded_keyword_namespace_and_local_match_parser_view() {
    let input = concat!(
        "PREF\\u0049X ex\\u003A <https://example.test/\\u03B1/> ",
        "SELECT * WHERE { ex\\u003As ex:p ex:o }"
    );
    let parsed = parse(input);
    assert!(parsed.to_string().contains("https://example.test/α/s"));

    let envelope = DirectIriMaterializationEnvelopeV1::measure(input).expect("measurement fits");
    let prefix = "https://example.test/α/";
    let direct = ["s", "p", "o"]
        .into_iter()
        .map(|local| prefix.len() + local.len())
        .sum::<usize>();
    assert_eq!(envelope.prefix_declarations, 1);
    assert_eq!(envelope.unique_prefix_bindings, 1);
    assert_eq!(envelope.prefix_state_bytes, prefix.len());
    assert_eq!(envelope.direct_ast_iri_bytes, direct);
    assert_eq!(envelope.max_resolved_iri_bytes, prefix.len() + 1);
    assert_eq!(envelope.peak_direct_parser_iri_bytes, prefix.len() + direct);
}

#[test]
fn sequential_base_and_prefix_state_matches_pinned_parser() {
    let input = concat!(
        "BASE <https://one.test/root/> ",
        "PREFIX early: <terms/> ",
        "PREFIX replace: <old/> ",
        "BASE <https://two.test/root/> ",
        "PREFIX replace: <new/> ",
        "VERSION \"1.2\" ",
        "SELECT * WHERE { early:s replace:p <relative> }"
    );
    let rendered = parse(input).to_string();
    let early = "https://one.test/root/terms/";
    let replacement = "https://two.test/root/new/";
    let final_base = "https://two.test/root/";
    assert!(rendered.contains(&format!("<{early}s>")));
    assert!(rendered.contains(&format!("<{replacement}p>")));
    assert!(rendered.contains("<https://two.test/root/relative>"));

    let envelope = DirectIriMaterializationEnvelopeV1::measure(input).expect("measurement fits");
    let direct = early.len()
        + 1
        + replacement.len()
        + 1
        + "https://two.test/root/relative".len()
        + final_base.len();
    assert_eq!(envelope.prefix_declarations, 3);
    assert_eq!(envelope.unique_prefix_bindings, 2);
    assert_eq!(envelope.prefix_state_bytes, early.len() + replacement.len());
    assert_eq!(envelope.direct_ast_iri_bytes, direct);
    assert!(
        envelope.peak_direct_parser_iri_bytes
            >= envelope.prefix_state_bytes + direct + final_base.len()
    );
}

#[test]
fn strings_and_comments_do_not_create_direct_iri_materializations() {
    let input = concat!(
        "PREFIX ex: <urn:example:> ",
        "SELECT (\"ex:hidden\" AS ?label) WHERE {\n",
        "# ex:commented\n",
        "ex:s ex:p ex:o . FILTER(STR(?label) = 'ex:hidden-too') }"
    );
    parse(input);
    let envelope = DirectIriMaterializationEnvelopeV1::measure(input).expect("measurement fits");
    let prefix = "urn:example:";
    assert_eq!(envelope.direct_ast_iri_bytes, 3 * (prefix.len() + 1));
}

#[test]
fn escaped_prefixed_local_counts_parser_materialized_bytes() {
    let input = "PREFIX ex: <urn:example:> SELECT * WHERE { ex:a\\~b ex:p ex:o }";
    let rendered = parse(input).to_string();
    assert!(rendered.contains("<urn:example:a~b>"));

    let envelope = DirectIriMaterializationEnvelopeV1::measure(input).expect("measurement fits");
    let prefix = "urn:example:";
    assert_eq!(
        envelope.direct_ast_iri_bytes,
        prefix.len() + "a~b".len() + 2 * (prefix.len() + 1)
    );
}

#[test]
fn default_prefix_expands_but_unknown_prefix_does_not_fabricate_bytes() {
    let input = "PREFIX : <urn:default:> SELECT * WHERE { :s :p :o }";
    parse(input);
    let envelope = DirectIriMaterializationEnvelopeV1::measure(input).expect("measurement fits");
    assert_eq!(
        envelope.direct_ast_iri_bytes,
        3 * ("urn:default:".len() + 1)
    );

    // The real parser rejects an unknown prefix. This pre-parser measurement
    // intentionally leaves it unexpanded instead of inventing parser state.
    let unknown = DirectIriMaterializationEnvelopeV1::measure(
        "SELECT * WHERE { missing:s missing:p missing:o }",
    )
    .expect("unknown prefixes are left to the real parser");
    assert_eq!(unknown.direct_ast_iri_bytes, 0);
}

#[test]
fn direct_materialization_limit_is_prospective_and_exact() {
    let input = "SELECT * WHERE { <urn:s> <urn:p> <urn:o> }";
    parse(input);
    let expected = "urn:s".len() + "urn:p".len() + "urn:o".len();
    let exact = Limits {
        direct_iri_materialization_bytes: expected,
        ..generous_limits()
    };
    let envelope = DirectIriMaterializationEnvelopeV1::measure_with_limits(input, exact)
        .expect("exact direct byte ceiling fits");
    assert_eq!(envelope.direct_ast_iri_bytes, expected);
    assert_eq!(envelope.peak_direct_parser_iri_bytes, expected);

    let rejected = Limits {
        direct_iri_materialization_bytes: expected - 1,
        ..exact
    };
    let error = DirectIriMaterializationEnvelopeV1::measure_with_limits(input, rejected)
        .expect_err("next direct byte must be rejected before commit");
    assert_limit(error, CompileEnvelopeLimit::DirectIriMaterializationBytes);
}

#[test]
fn relative_resolution_accounts_for_simultaneous_input_and_output() {
    let base = "https://example.test/root/";
    let relative = "segment/".repeat(20);
    let resolved = format!("{base}{relative}");
    let input = format!("BASE <{base}> ASK {{ <{relative}> ?p ?o }}");
    parse(&input);

    let resolution_peak = base.len() + relative.len() + resolved.len();
    let final_base_clone_peak = 2 * base.len() + resolved.len();
    assert!(resolution_peak > final_base_clone_peak);
    let exact = Limits {
        direct_iri_materialization_bytes: resolution_peak,
        ..generous_limits()
    };
    let envelope = DirectIriMaterializationEnvelopeV1::measure_with_limits(&input, exact)
        .expect("exact simultaneous resolution payload fits");
    assert_eq!(envelope.peak_direct_parser_iri_bytes, resolution_peak);

    let error = DirectIriMaterializationEnvelopeV1::measure_with_limits(
        &input,
        Limits {
            direct_iri_materialization_bytes: resolution_peak - 1,
            ..exact
        },
    )
    .expect_err("simultaneous unescaped input and resolved output must be bounded");
    assert_limit(error, CompileEnvelopeLimit::DirectIriMaterializationBytes);
}

#[test]
fn duplicate_prefix_declarations_are_counted_before_last_write_wins() {
    let input = concat!(
        "PREFIX ex: <urn:old:> PREFIX ex: <urn:new:> ",
        "SELECT * WHERE { ex:s ex:p ex:o }"
    );
    parse(input);
    let exact = Limits {
        prefix_declarations: 2,
        prefix_bindings: 1,
        ..generous_limits()
    };
    let envelope = DirectIriMaterializationEnvelopeV1::measure_with_limits(input, exact)
        .expect("duplicate declaration fits exact declaration ceiling");
    assert_eq!(envelope.prefix_declarations, 2);
    assert_eq!(envelope.unique_prefix_bindings, 1);
    assert_eq!(envelope.prefix_state_bytes, "urn:new:".len());

    let error = DirectIriMaterializationEnvelopeV1::measure_with_limits(
        input,
        Limits {
            prefix_declarations: 1,
            ..exact
        },
    )
    .expect_err("duplicate still consumes a declaration slot");
    assert_limit(error, CompileEnvelopeLimit::PrefixDeclarations);
}

#[test]
fn unique_prefix_binding_limit_is_prospective() {
    let input = concat!(
        "PREFIX one: <urn:one:> PREFIX two: <urn:two:> ",
        "SELECT * WHERE { one:s one:p two:o }"
    );
    parse(input);
    let error = DirectIriMaterializationEnvelopeV1::measure_with_limits(
        input,
        Limits {
            prefix_bindings: 1,
            ..generous_limits()
        },
    )
    .expect_err("second retained prefix must be rejected before insertion");
    assert_limit(error, CompileEnvelopeLimit::PrefixBindings);
}

#[test]
fn property_list_clone_amplification_exceeds_direct_source_bytes() {
    let prefix = format!("https://example.test/{}/", "segment".repeat(24));
    let input = format!(
        "PREFIX ex: <{prefix}> SELECT * WHERE {{ \
         ex:subject ex:p1 ex:o1 ; ex:p2 ex:o2 ; ex:p3 ex:o3 . }}"
    );
    let parsed = parse(&input);
    let direct = DirectIriMaterializationEnvelopeV1::measure(&input).expect("measurement fits");
    let retained = AlgebraEnvelopeV1::validate(&parsed).expect("algebra envelope fits");

    let direct_occurrence_bytes = prefix.len() * 7
        + "subject".len()
        + "p1".len()
        + "o1".len()
        + "p2".len()
        + "o2".len()
        + "p3".len()
        + "o3".len();
    assert_eq!(direct.direct_ast_iri_bytes, direct_occurrence_bytes);
    assert!(
        retained.retained_payload_bytes > direct.direct_ast_iri_bytes,
        "parser-generated subject clones are outside the direct-byte measure"
    );
}

#[test]
fn checked_in_ontop_queries_fit_the_direct_measurement() {
    const QUERIES: &[(&str, &str)] = &[
        ("dump", include_str!("../../../../../scripts/ontop/dump.rq")),
        ("q1", include_str!("../../../../../scripts/ontop/q1.rq")),
        ("q2", include_str!("../../../../../scripts/ontop/q2.rq")),
        ("q3", include_str!("../../../../../scripts/ontop/q3.rq")),
        ("q4", include_str!("../../../../../scripts/ontop/q4.rq")),
        ("q5", include_str!("../../../../../scripts/ontop/q5.rq")),
        ("q6", include_str!("../../../../../scripts/ontop/q6.rq")),
        ("q7", include_str!("../../../../../scripts/ontop/q7.rq")),
        ("q8", include_str!("../../../../../scripts/ontop/q8.rq")),
        ("q9", include_str!("../../../../../scripts/ontop/q9.rq")),
        ("q10", include_str!("../../../../../scripts/ontop/q10.rq")),
        ("q11", include_str!("../../../../../scripts/ontop/q11.rq")),
        ("q12", include_str!("../../../../../scripts/ontop/q12.rq")),
        ("q13", include_str!("../../../../../scripts/ontop/q13.rq")),
        ("q14", include_str!("../../../../../scripts/ontop/q14.rq")),
        ("q15", include_str!("../../../../../scripts/ontop/q15.rq")),
    ];

    for (name, source) in QUERIES {
        parse(source);
        DirectIriMaterializationEnvelopeV1::measure(source)
            .unwrap_or_else(|error| panic!("{name} must fit direct measurement: {error}"));
    }
}
