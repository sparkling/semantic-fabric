use super::*;

fn assert_limit(
    error: CompileEnvelopeError,
    dimension: CompileEnvelopeLimit,
    observed: usize,
    maximum: usize,
) {
    assert_eq!(
        error,
        CompileEnvelopeError::LimitExceeded {
            dimension,
            observed,
            maximum,
        }
    );
}

fn operator_chain(length: usize) -> String {
    let mut input = String::from("?value");
    for _ in 0..length {
        input.push_str(" || ?value");
    }
    input
}

#[test]
fn should_count_utf8_lexemes_in_bytes() {
    let envelope = CompileEnvelopeV1::scan("\"é漢\"").expect("small string fits");

    assert_eq!(
        (
            envelope.input_bytes,
            envelope.tokens,
            envelope.max_lexeme_bytes
        ),
        (7, 1, 7)
    );
}

#[test]
fn should_shield_every_string_form_iri_and_comment_context() {
    let input = concat!(
        "<https://example.test/a/(b)?x=1#frag> ",
        "'single \\' still ) || <<' ",
        "\"double \\\" still ] && >>\" ",
        "'''long ' \" still } | <<''' ",
        "\"\"\"long \\\" quoted ( || >>\"\"\" ",
        "# comment ((( || << >>\n",
        "token",
    );

    let envelope = CompileEnvelopeV1::scan(input).expect("shielded punctuation fits");

    assert_eq!(
        (
            envelope.tokens,
            envelope.max_nesting_depth,
            envelope.max_rdf_star_depth,
            envelope.max_operators_per_scope,
        ),
        (7, 0, 0, 0)
    );
}

#[test]
fn should_keep_iri_unicode_escapes_fragment_and_path_operators_in_one_lexeme() {
    let input = r#"<https://example.test/\u0028?a=b/c#fragment>"#;
    let envelope = CompileEnvelopeV1::scan(input).expect("escaped IRI fits");

    assert_eq!(
        (
            envelope.tokens,
            envelope.max_lexeme_bytes,
            envelope.max_nesting_depth,
            envelope.max_operators_per_scope,
        ),
        (1, input.len(), 0, 0)
    );
}

#[test]
fn should_keep_escaped_prefixed_name_punctuation_in_one_lexeme() {
    let input = r"prefix:item\-part\/tail\#fragment";
    let envelope = CompileEnvelopeV1::scan(input).expect("escaped prefixed name fits");

    assert_eq!(
        (
            envelope.tokens,
            envelope.max_lexeme_bytes,
            envelope.max_operators_per_scope,
        ),
        (1, input.len(), 0)
    );
}

#[test]
fn should_leave_short_string_mode_at_an_unescaped_line_terminator() {
    let envelope = CompileEnvelopeV1::scan("'unterminated\n?a || ?b")
        .expect("bounded malformed input remains scanner-safe");

    assert_eq!(envelope.max_operators_per_scope, 1);
}

#[test]
fn should_end_a_comment_at_the_first_line_terminator() {
    let envelope = CompileEnvelopeV1::scan("# || << (\r\n?a || ?b")
        .expect("comment followed by expression fits");

    assert_eq!(
        (
            envelope.tokens,
            envelope.max_nesting_depth,
            envelope.max_rdf_star_depth,
            envelope.max_operators_per_scope,
        ),
        (4, 0, 0, 1)
    );
}

#[test]
fn should_count_nested_rdf_star_delimiters_as_tokens_and_depth() {
    let envelope =
        CompileEnvelopeV1::scan("<< << ?s ?p ?o >> >>").expect("small quoted triple fits");

    assert_eq!(
        (
            envelope.tokens,
            envelope.max_nesting_depth,
            envelope.max_rdf_star_depth,
            envelope.max_operators_per_scope,
        ),
        (7, 2, 2, 0)
    );
}

#[test]
fn should_count_balanced_mixed_delimiter_depth() {
    let envelope = CompileEnvelopeV1::scan("({[term]})").expect("small nesting fits");

    assert_eq!((envelope.tokens, envelope.max_nesting_depth), (7, 3));
}

#[test]
fn should_not_let_a_mismatched_closer_reduce_conservative_depth() {
    let envelope = CompileEnvelopeV1::scan("( ] (")
        .expect("small malformed delimiter sequence remains bounded");

    assert_eq!(envelope.max_nesting_depth, 2);
}

#[test]
fn should_count_operator_chains_across_operands() {
    let envelope = CompileEnvelopeV1::scan(&operator_chain(7)).expect("small chain fits");

    assert_eq!(envelope.max_operators_per_scope, 7);
}

#[test]
fn should_reset_operator_chains_at_structural_separators() {
    let envelope = CompileEnvelopeV1::scan("?a || ?b ; ?c && ?d , ?e / ?f")
        .expect("separate short chains fit");

    assert_eq!(envelope.max_operators_per_scope, 1);
}

#[test]
fn should_count_boolean_comparison_and_datatype_operators() {
    let envelope =
        CompileEnvelopeV1::scan("?a <= ?b && ?c != ?d || ?e >= 1 ^^ <https://example.test/type>")
            .expect("short mixed chain fits");

    assert_eq!(envelope.max_operators_per_scope, 6);
}

#[test]
fn should_not_reset_an_arithmetic_chain_at_decimal_dots() {
    let envelope =
        CompileEnvelopeV1::scan("?a + 1.0 + 2.0 + ?b").expect("short arithmetic chain fits");

    assert_eq!(envelope.max_operators_per_scope, 3);
}

#[test]
fn should_count_graph_operator_keywords_in_their_parent_scope() {
    let envelope =
        CompileEnvelopeV1::scan("{} UNION {} UNION {} MINUS {}").expect("short graph chain fits");

    assert_eq!(envelope.max_operators_per_scope, 3);
}

#[test]
fn should_resume_structural_scanning_after_an_invalid_iri_prefix() {
    let envelope = CompileEnvelopeV1::scan("<not-an-iri || ?a || ?b")
        .expect("bounded malformed input remains scanner-safe");

    assert_eq!(
        envelope.max_operators_per_scope, 6,
        "malformed IRI-like text must be scanned conservatively as operators"
    );
}

#[test]
fn should_not_hide_a_parser_accepted_operator_chain_as_an_unterminated_iri() {
    let mut expression = String::from("?x <1");
    for _ in 0..MAX_OPERATORS_PER_SCOPE_V1 {
        expression.push_str("+1");
    }
    let input = format!("SELECT * WHERE {{ ?s ?p ?x . FILTER({expression}) }}");
    spargebra::SparqlParser::new()
        .parse_query(&input)
        .expect("the pinned parser accepts the no-whitespace relational expression");

    let error = CompileEnvelopeV1::scan(&input).expect_err("operator chain must not be hidden");
    assert_limit(
        error,
        CompileEnvelopeLimit::OperatorsPerScope,
        MAX_OPERATORS_PER_SCOPE_V1 + 1,
        MAX_OPERATORS_PER_SCOPE_V1,
    );
}

fn compact_relational_iri(payload_len: usize) -> String {
    let iri = format!("<urn:{}>", "a".repeat(payload_len));
    format!("SELECT * WHERE {{ ?s ?p ?x . FILTER(?x<{iri}) }}")
}

#[test]
fn should_enforce_both_parser_interpretations_of_compact_double_less_than() {
    let input = compact_relational_iri(MAX_LEXEME_BYTES_V1 - 6);
    spargebra::SparqlParser::new()
        .parse_query(&input)
        .expect("the pinned parser accepts relational less-than followed by an IRI");

    let envelope = CompileEnvelopeV1::scan(&input).expect("exact IRI lexeme limit fits");
    assert_eq!(envelope.max_lexeme_bytes, MAX_LEXEME_BYTES_V1);
    assert!(envelope.max_operators_per_scope >= 1);
}

#[test]
fn should_reject_an_overlong_iri_after_compact_relational_less_than() {
    let input = compact_relational_iri(MAX_LEXEME_BYTES_V1 - 5);
    spargebra::SparqlParser::new()
        .parse_query(&input)
        .expect("the pinned parser accepts the over-envelope IRI syntax");

    let error = CompileEnvelopeV1::scan(&input).expect_err("overlong IRI must not be split");
    assert_limit(
        error,
        CompileEnvelopeLimit::LexemeBytes,
        MAX_LEXEME_BYTES_V1 + 1,
        MAX_LEXEME_BYTES_V1,
    );
}

#[test]
fn should_accept_the_exact_scanned_byte_limit() {
    let input = " ".repeat(MAX_SCANNED_BYTES_V1);
    let envelope = CompileEnvelopeV1::scan(&input).expect("exact byte limit fits");

    assert_eq!(envelope.input_bytes, MAX_SCANNED_BYTES_V1);
}

#[test]
fn should_reject_one_byte_beyond_the_scanned_byte_limit() {
    let input = " ".repeat(MAX_SCANNED_BYTES_V1 + 1);
    let error = CompileEnvelopeV1::scan(&input).expect_err("oversized input must fail");

    assert_limit(
        error,
        CompileEnvelopeLimit::InputBytes,
        MAX_SCANNED_BYTES_V1 + 1,
        MAX_SCANNED_BYTES_V1,
    );
}

#[test]
fn should_accept_the_exact_token_limit() {
    let input = "x ".repeat(MAX_TOKENS_V1);
    let envelope = CompileEnvelopeV1::scan(&input).expect("exact token limit fits");

    assert_eq!(envelope.tokens, MAX_TOKENS_V1);
}

#[test]
fn should_reject_one_token_beyond_the_token_limit() {
    let input = "x ".repeat(MAX_TOKENS_V1 + 1);
    let error = CompileEnvelopeV1::scan(&input).expect_err("excess token must fail");

    assert_limit(
        error,
        CompileEnvelopeLimit::Tokens,
        MAX_TOKENS_V1 + 1,
        MAX_TOKENS_V1,
    );
}

#[test]
fn should_accept_the_exact_lexeme_byte_limit() {
    let input = "x".repeat(MAX_LEXEME_BYTES_V1);
    let envelope = CompileEnvelopeV1::scan(&input).expect("exact lexeme limit fits");

    assert_eq!(envelope.max_lexeme_bytes, MAX_LEXEME_BYTES_V1);
}

#[test]
fn should_accept_string_comment_and_iri_at_the_exact_lexeme_limit() {
    let string = format!("\"{}\"", "x".repeat(MAX_LEXEME_BYTES_V1 - 2));
    let comment = format!("#{}", "x".repeat(MAX_LEXEME_BYTES_V1 - 1));
    let iri = format!("<{}>", "x".repeat(MAX_LEXEME_BYTES_V1 - 2));

    for input in [&string, &comment, &iri] {
        let envelope = CompileEnvelopeV1::scan(input).expect("exact lexical payload fits");
        assert_eq!(envelope.max_lexeme_bytes, MAX_LEXEME_BYTES_V1);
    }
}

#[test]
fn should_reject_one_byte_beyond_the_lexeme_limit() {
    let input = "x".repeat(MAX_LEXEME_BYTES_V1 + 1);
    let error = CompileEnvelopeV1::scan(&input).expect_err("long lexeme must fail");

    assert_limit(
        error,
        CompileEnvelopeLimit::LexemeBytes,
        MAX_LEXEME_BYTES_V1 + 1,
        MAX_LEXEME_BYTES_V1,
    );
}

#[test]
fn should_apply_the_lexeme_limit_to_comments() {
    let input = format!("#{}", "x".repeat(MAX_LEXEME_BYTES_V1));
    let error = CompileEnvelopeV1::scan(&input).expect_err("long comment must fail");

    assert_limit(
        error,
        CompileEnvelopeLimit::LexemeBytes,
        MAX_LEXEME_BYTES_V1 + 1,
        MAX_LEXEME_BYTES_V1,
    );
}

#[test]
fn should_apply_the_lexeme_limit_to_strings_and_iris() {
    let string = format!("\"{}\"", "x".repeat(MAX_LEXEME_BYTES_V1 - 1));
    let iri = format!("<{}>", "x".repeat(MAX_LEXEME_BYTES_V1 - 1));

    for input in [&string, &iri] {
        let error = CompileEnvelopeV1::scan(input).expect_err("long lexical payload must fail");
        assert_limit(
            error,
            CompileEnvelopeLimit::LexemeBytes,
            MAX_LEXEME_BYTES_V1 + 1,
            MAX_LEXEME_BYTES_V1,
        );
    }
}

#[test]
fn should_accept_the_exact_delimiter_nesting_limit() {
    let input = "(".repeat(MAX_NESTING_DEPTH_V1);
    let envelope = CompileEnvelopeV1::scan(&input).expect("exact nesting limit fits");

    assert_eq!(envelope.max_nesting_depth, MAX_NESTING_DEPTH_V1);
}

#[test]
fn should_reject_one_delimiter_beyond_the_nesting_limit() {
    let input = "(".repeat(MAX_NESTING_DEPTH_V1 + 1);
    let error = CompileEnvelopeV1::scan(&input).expect_err("deep nesting must fail");

    assert_limit(
        error,
        CompileEnvelopeLimit::NestingDepth,
        MAX_NESTING_DEPTH_V1 + 1,
        MAX_NESTING_DEPTH_V1,
    );
}

#[test]
fn should_accept_the_exact_rdf_star_nesting_limit() {
    let input = "<<".repeat(MAX_RDF_STAR_DEPTH_V1);
    let envelope = CompileEnvelopeV1::scan(&input).expect("exact RDF-star depth fits");

    assert_eq!(envelope.max_rdf_star_depth, MAX_RDF_STAR_DEPTH_V1);
}

#[test]
fn should_reject_one_rdf_star_level_beyond_its_limit() {
    let input = "<<".repeat(MAX_RDF_STAR_DEPTH_V1 + 1);
    let error = CompileEnvelopeV1::scan(&input).expect_err("deep RDF-star must fail");

    assert_limit(
        error,
        CompileEnvelopeLimit::RdfStarDepth,
        MAX_RDF_STAR_DEPTH_V1 + 1,
        MAX_RDF_STAR_DEPTH_V1,
    );
}

#[test]
fn should_accept_the_exact_operators_per_scope_limit() {
    let envelope = CompileEnvelopeV1::scan(&operator_chain(MAX_OPERATORS_PER_SCOPE_V1))
        .expect("exact operator chain fits");

    assert_eq!(envelope.max_operators_per_scope, MAX_OPERATORS_PER_SCOPE_V1);
}

#[test]
fn should_reject_one_operator_beyond_the_scope_limit() {
    let error = CompileEnvelopeV1::scan(&operator_chain(MAX_OPERATORS_PER_SCOPE_V1 + 1))
        .expect_err("long operator chain must fail");

    assert_limit(
        error,
        CompileEnvelopeLimit::OperatorsPerScope,
        MAX_OPERATORS_PER_SCOPE_V1 + 1,
        MAX_OPERATORS_PER_SCOPE_V1,
    );
}
