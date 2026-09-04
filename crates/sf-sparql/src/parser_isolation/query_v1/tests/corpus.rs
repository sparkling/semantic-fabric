use oxrdf::NamedNode;
use spargebra::SparqlParser;

use super::*;

#[test]
fn all_query_forms_preserve_exact_ast_and_canonical_bytes() {
    let cases = [
        (
            "select dataset base modifiers",
            tag::QUERY_SELECT,
            concat!(
                "BASE <https://example.test/root/> ",
                "SELECT DISTINCT ?s FROM <default> FROM NAMED <named> WHERE { ",
                "GRAPH <named> { ?s <p> <relative> } } ",
                "ORDER BY ASC(?s) DESC(STR(?s)) LIMIT 7 OFFSET 2",
            ),
        ),
        (
            "ask",
            tag::QUERY_ASK,
            "ASK FROM <https://example.test/default> WHERE { ?s ?p ?o }",
        ),
        (
            "describe",
            tag::QUERY_DESCRIBE,
            "DESCRIBE ?s WHERE { ?s <https://example.test/p> ?o }",
        ),
        (
            "construct template",
            tag::QUERY_CONSTRUCT,
            concat!(
                "CONSTRUCT { ?s <https://example.test/copy> _:result } WHERE { ",
                "?s <https://example.test/source> ?o }",
            ),
        ),
        (
            "construct where",
            tag::QUERY_CONSTRUCT,
            "CONSTRUCT WHERE { ?s <https://example.test/p> ?o }",
        ),
    ];
    for (label, query_tag, source) in cases {
        let wire = assert_source_round_trip(label, source);
        assert_eq!(record_tag(&wire, record_count(&wire) - 1), query_tag);
        if label == "select dataset base modifiers" {
            assert_has_tags(
                &wire,
                &[
                    tag::DATASET,
                    tag::BASE_IRI,
                    tag::GRAPH_ORDER_BY,
                    tag::GRAPH_PROJECT,
                    tag::GRAPH_DISTINCT,
                    tag::GRAPH_SLICE,
                ],
            );
        }
    }
}

#[test]
fn empty_ask_has_a_stable_independent_golden_vector() {
    let expected = [
        // Header: magic, version, header/record lengths, root, and section counts.
        0x53, 0x46, 0x50, 0x51, 0x57, 0x30, 0x30, 0x31, 0x00, 0x01, 0x00, 0x20, 0x00, 0x20, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
        0x00, 0x00, // Empty BGP record.
        0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, // ASK root: one edge at edge-table offset zero.
        0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x01, // Root edge to the BGP.
        0x00, 0x00, 0x00, 0x00,
    ];
    let query = spargebra::Query::Ask {
        dataset: None,
        pattern: spargebra::algebra::GraphPattern::Bgp {
            patterns: Vec::new(),
        },
        base_iri: None,
    };
    let encoded = encode(&query).expect("golden query must encode");
    assert_eq!(encoded, expected);
    assert_eq!(
        decode_exact(&expected).expect("golden wire must decode"),
        query
    );
}

#[test]
fn graph_algebra_and_values_undef_round_trip() {
    let wire = assert_source_round_trip(
        "graph algebra",
        concat!(
            "PREFIX ex: <https://example.test/> ",
            "SELECT REDUCED ?s ?calc WHERE { ",
            "VALUES (?s ?seed) { (ex:a 1) (UNDEF 2) } ",
            "{ ?s ex:p ?o } UNION { ?s ex:q ?o } ",
            "OPTIONAL { ?s ex:optional ?v . FILTER(?v > 0) } ",
            "MINUS { ?s ex:removed ?x } ",
            "GRAPH ?g { ?s ex:inside ?x } ",
            "BIND((?seed + 2) AS ?calc) ",
            "FILTER(EXISTS { ?s ex:exists ?z }) ",
            "SERVICE SILENT <https://service.example/sparql> { ?s ex:remote ?r } ",
            "}",
        ),
    );
    assert_has_tags(
        &wire,
        &[
            tag::GRAPH_BGP,
            tag::GRAPH_JOIN,
            tag::GRAPH_LEFT_JOIN,
            tag::GRAPH_FILTER,
            tag::GRAPH_UNION,
            tag::GRAPH_NAMED,
            tag::GRAPH_EXTEND,
            tag::GRAPH_MINUS,
            tag::GRAPH_VALUES,
            tag::GRAPH_PROJECT,
            tag::GRAPH_REDUCED,
            tag::GRAPH_SERVICE,
            tag::NAMED_PATTERN_VARIABLE,
            tag::GROUND_TERM_NAMED_NODE,
            tag::GROUND_TERM_LITERAL,
            tag::VALUES_ROW,
        ],
    );
}

#[test]
fn lateral_and_every_property_path_shape_round_trip() {
    let cases = [
        (
            "lateral",
            concat!(
                "PREFIX ex: <https://example.test/> SELECT * WHERE { ",
                "?s ex:p ?o LATERAL { ?x ex:q ?z } }",
            ),
        ),
        (
            "property paths",
            concat!(
                "PREFIX ex: <https://example.test/> SELECT * WHERE { ",
                "?s ((^ex:p/ex:q)|!(ex:r|^ex:s))* ?o . ",
                "?s ex:one+ ?x . ?x ex:maybe? ?y }",
            ),
        ),
    ];
    for (label, source) in cases {
        let wire = assert_source_round_trip(label, source);
        if label == "lateral" {
            assert_has_tags(&wire, &[tag::GRAPH_LATERAL]);
        } else {
            assert_has_tags(
                &wire,
                &[
                    tag::GRAPH_PATH,
                    tag::PATH_NAMED_NODE,
                    tag::PATH_REVERSE,
                    tag::PATH_SEQUENCE,
                    tag::PATH_ALTERNATIVE,
                    tag::PATH_ZERO_OR_MORE,
                    tag::PATH_ONE_OR_MORE,
                    tag::PATH_ZERO_OR_ONE,
                    tag::PATH_NEGATED_SET,
                ],
            );
        }
    }
}

#[test]
fn expression_tree_shapes_round_trip() {
    let wire = assert_source_round_trip(
        "expression operators",
        concat!(
            "SELECT * WHERE { VALUES (?a ?b) { (1 2) } ",
            "BIND((?a || ?b) AS ?or) BIND((?a && ?b) AS ?and) ",
            "BIND((?a = ?b) AS ?eq) BIND(sameTerm(?a, ?b) AS ?same) ",
            "BIND((?a > ?b) AS ?gt) BIND((?a >= ?b) AS ?ge) ",
            "BIND((?a < ?b) AS ?lt) BIND((?a <= ?b) AS ?le) ",
            "BIND((?a IN (1, 2, 3)) AS ?in) ",
            "BIND((?a + ?b - 1) AS ?sub) BIND((?a * ?b / 2) AS ?div) ",
            "BIND((+?a) AS ?plus) BIND((-?b) AS ?minus) BIND((!?a) AS ?not) ",
            "BIND(BOUND(?a) AS ?bound) BIND(IF(?a, ?b, 0) AS ?if) ",
            "BIND(COALESCE(?missing, ?a, ?b) AS ?coalesce) ",
            "FILTER(NOT EXISTS { ?x <https://example.test/p> ?y }) }",
        ),
    );
    assert_has_tags(
        &wire,
        &[
            tag::EXPR_LITERAL,
            tag::EXPR_VARIABLE,
            tag::EXPR_OR,
            tag::EXPR_AND,
            tag::EXPR_EQUAL,
            tag::EXPR_SAME_TERM,
            tag::EXPR_GREATER,
            tag::EXPR_GREATER_EQUAL,
            tag::EXPR_LESS,
            tag::EXPR_LESS_EQUAL,
            tag::EXPR_IN,
            tag::EXPR_ADD,
            tag::EXPR_SUBTRACT,
            tag::EXPR_MULTIPLY,
            tag::EXPR_DIVIDE,
            tag::EXPR_UNARY_PLUS,
            tag::EXPR_UNARY_MINUS,
            tag::EXPR_NOT,
            tag::EXPR_EXISTS,
            tag::EXPR_BOUND,
            tag::EXPR_IF,
            tag::EXPR_COALESCE,
        ],
    );
}

#[test]
fn every_builtin_and_custom_function_round_trips() {
    let expressions = [
        "STR(?x)",
        "LANG(?x)",
        "LANGMATCHES(?x, ?y)",
        "DATATYPE(?x)",
        "IRI(?x)",
        "BNODE()",
        "RAND()",
        "ABS(?x)",
        "CEIL(?x)",
        "FLOOR(?x)",
        "ROUND(?x)",
        "CONCAT(?x, ?y)",
        "SUBSTR(?x, 1, 2)",
        "STRLEN(?x)",
        "REPLACE(?x, ?y, ?z, \"i\")",
        "UCASE(?x)",
        "LCASE(?x)",
        "ENCODE_FOR_URI(?x)",
        "CONTAINS(?x, ?y)",
        "STRSTARTS(?x, ?y)",
        "STRENDS(?x, ?y)",
        "STRBEFORE(?x, ?y)",
        "STRAFTER(?x, ?y)",
        "YEAR(?x)",
        "MONTH(?x)",
        "DAY(?x)",
        "HOURS(?x)",
        "MINUTES(?x)",
        "SECONDS(?x)",
        "TIMEZONE(?x)",
        "TZ(?x)",
        "NOW()",
        "UUID()",
        "STRUUID()",
        "MD5(?x)",
        "SHA1(?x)",
        "SHA256(?x)",
        "SHA384(?x)",
        "SHA512(?x)",
        "STRLANG(?x, ?y)",
        "STRDT(?x, <https://example.test/type>)",
        "isIRI(?x)",
        "isBLANK(?x)",
        "isLITERAL(?x)",
        "isNUMERIC(?x)",
        "REGEX(?x, ?y, \"i\")",
        "TRIPLE(?x, <https://example.test/p>, ?y)",
        "SUBJECT(?t)",
        "PREDICATE(?t)",
        "OBJECT(?t)",
        "isTRIPLE(?t)",
        "LANGDIR(?x)",
        "hasLANG(?x)",
        "hasLANGDIR(?x)",
        "STRLANGDIR(?x, \"en\", \"ltr\")",
        "ADJUST(?x, ?y)",
        "<https://example.test/function>(?x)",
    ];
    let projections = expressions
        .iter()
        .enumerate()
        .map(|(index, expression)| format!("({expression} AS ?f{index})"))
        .collect::<Vec<_>>()
        .join(" ");
    let source = format!(
        "SELECT {projections} WHERE {{ VALUES (?x ?y ?z ?t) {{ (\"abc\" \"en\" \"z\" <<( <https://example.test/s> <https://example.test/p> \"o\" )>>) }} }}"
    );
    let wire = assert_source_round_trip("function surface", &source);
    assert_has_tags(
        &wire,
        &[tag::EXPR_NAMED_NODE, tag::EXPR_FUNCTION_CALL, tag::FUNCTION],
    );
    let actual = record_field_values(&wire, tag::FUNCTION, 0)
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    let expected = (0..=56).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual, expected, "function-code surface is incomplete");
}

#[test]
fn aggregates_orders_groups_and_slices_round_trip() {
    let builtins = assert_source_round_trip(
        "aggregate surface",
        concat!(
            "SELECT ?g (COUNT(*) AS ?count_all) (COUNT(DISTINCT *) AS ?count_distinct) ",
            "(COUNT(?v) AS ?count) (SUM(DISTINCT ?v) AS ?sum) (AVG(?v) AS ?avg) ",
            "(MIN(?v) AS ?min) (MAX(?v) AS ?max) (SAMPLE(?v) AS ?sample) ",
            "(GROUP_CONCAT(?v) AS ?joined_default) ",
            "(GROUP_CONCAT(DISTINCT ?v; SEPARATOR = \"|\") AS ?joined) ",
            "WHERE { ?s <https://example.test/group> ?g ; ",
            "<https://example.test/value> ?v } GROUP BY ?g ",
            "ORDER BY ASC(?g) DESC(?count_all) LIMIT 4 OFFSET 1",
        ),
    );
    assert_has_tags(
        &builtins,
        &[
            tag::GRAPH_ORDER_BY,
            tag::GRAPH_PROJECT,
            tag::GRAPH_SLICE,
            tag::GRAPH_GROUP,
            tag::AGGREGATE_COUNT_SOLUTIONS,
            tag::AGGREGATE_FUNCTION_CALL,
            tag::AGGREGATE_FUNCTION,
            tag::AGGREGATE_BINDING,
            tag::ORDER_ASC,
            tag::ORDER_DESC,
        ],
    );

    let aggregate = NamedNode::new("https://example.test/aggregate").expect("valid IRI");
    let query = SparqlParser::new()
        .with_custom_aggregate_function(aggregate)
        .parse_query(concat!(
            "PREFIX ex: <https://example.test/> ",
            "SELECT (ex:aggregate(?v) AS ?custom) ",
            "WHERE { ?s <https://example.test/value> ?v }",
        ))
        .expect("custom aggregate fixture");
    let custom = assert_query_round_trip("custom aggregate", &query);
    let mut codes = record_field_values(&builtins, tag::AGGREGATE_FUNCTION, 0);
    codes.extend(record_field_values(&custom, tag::AGGREGATE_FUNCTION, 0));
    let actual = codes.into_iter().collect::<std::collections::BTreeSet<_>>();
    let expected = (0..=7).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual, expected, "aggregate-code surface is incomplete");
}

#[test]
fn rdf_star_blank_nodes_unicode_and_literal_kinds_round_trip() {
    let wire = assert_source_round_trip(
        "term and literal surface",
        concat!(
            "SELECT * WHERE { ",
            "<https://example.test/named-subject> <https://example.test/p> ",
            "<https://example.test/named-object> . ",
            "<<( ?s <https://example.test/p> \"nested\" )>> ",
            "<https://example.test/asserted> ?certainty . ",
            "_:source <https://example.test/plain> \"東京 café\" ; ",
            "<https://example.test/typed> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> ; ",
            "<https://example.test/lang> \"hello\"@en ; ",
            "<https://example.test/ltr> \"hello\"@en--ltr ; ",
            "<https://example.test/rtl> \"שלום\"@he--rtl . ",
            "VALUES ?ground { <<( <https://example.test/s> ",
            "<https://example.test/p> \"object\" )>> } }",
        ),
    );
    let literal_kinds = record_field_values(&wire, tag::LITERAL, 4)
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(literal_kinds, (0..=4).collect());
    assert_has_tags(
        &wire,
        &[
            tag::TRIPLE_PATTERN,
            tag::TERM_NAMED_NODE,
            tag::TERM_BLANK_NODE,
            tag::TERM_LITERAL,
            tag::TERM_TRIPLE,
            tag::TERM_VARIABLE,
            tag::NAMED_PATTERN_NODE,
            tag::GROUND_TERM_TRIPLE,
            tag::GROUND_TRIPLE,
            tag::NAMED_NODE,
            tag::VARIABLE,
            tag::BLANK_NODE,
            tag::LITERAL,
        ],
    );
}

#[test]
fn query_dataset_preserves_some_empty_named_graph_set() {
    let mut query = parse("SELECT * WHERE { ?s ?p ?o }");
    let spargebra::Query::Select { dataset, .. } = &mut query else {
        unreachable!("fixture is SELECT")
    };
    *dataset = Some(spargebra::algebra::QueryDataset {
        default: Vec::new(),
        named: Some(Vec::new()),
    });
    assert_query_round_trip("empty explicit dataset", &query);
}

#[test]
fn slice_without_length_preserves_the_absent_length_flag() {
    let wire =
        assert_source_round_trip("offset-only slice", "SELECT * WHERE { ?s ?p ?o } OFFSET 3");
    let slice = find_record(&wire, tag::GRAPH_SLICE);
    assert_eq!(read_u16(&wire, record_offset(slice) + 2), 0);
    assert_eq!(read_u32(&wire, record_offset(slice) + 12), 0);
    assert_eq!(read_u32(&wire, record_offset(slice) + 16), 0);
}
