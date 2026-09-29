//! Dependency oracle for the SPARQL Results XML codec (`sparesults` over `quick-xml`).
//!
//! Exercises the real `QueryResultsParser` (reader and slice routes) and
//! `QueryResultsSerializer` against independently built `oxrdf` terms. Test-only
//! (`cfg(test)`): no production code path or runtime feature depends on it.
//!
//! Passing this corpus proves compatibility only. It does NOT close
//! RUSTSEC-2026-0194/0195 while the lock resolves the vulnerable quick-xml 0.37.5;
//! a dependency replacement must rerun this module unchanged.

use oxrdf::{BlankNode, Literal, NamedNode, Term, Variable};
use sparesults::{
    QueryResultsFormat, QueryResultsParser, QueryResultsSerializer, QuerySolution,
    ReaderQueryResultsParserOutput, SliceQueryResultsParserOutput,
};
use std::fmt::Display;

const NS: &str = "http://www.w3.org/2005/sparql-results#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const MAX_ROWS: usize = 1000;

type Row = Vec<Option<Term>>;

#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Solutions {
        variables: Vec<Variable>,
        rows: Vec<Row>,
    },
    Boolean(bool),
}

fn var(name: &str) -> Variable {
    Variable::new_unchecked(name)
}

fn iri(value: &str) -> Term {
    NamedNode::new_unchecked(value).into()
}

fn plain(value: &str) -> Term {
    Literal::new_simple_literal(value).into()
}

fn typed(value: &str, datatype: &str) -> Term {
    Literal::new_typed_literal(value, NamedNode::new_unchecked(datatype)).into()
}

fn lang(value: &str, tag: &str) -> Term {
    Literal::new_language_tagged_literal_unchecked(value, tag).into()
}

fn bnode(id: &str) -> Term {
    BlankNode::new_unchecked(id).into()
}

fn select(vars: &[&str], rows: Vec<Row>) -> Outcome {
    Outcome::Solutions {
        variables: vars.iter().map(|v| var(v)).collect(),
        rows,
    }
}

fn select_doc(vars: &[&str], body: &str) -> String {
    let head: String = vars
        .iter()
        .map(|v| format!("<variable name=\"{v}\"/>"))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<sparql xmlns=\"{NS}\">\
         <head>{head}</head><results>{body}</results></sparql>"
    )
}

fn bool_doc(value: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><sparql xmlns=\"{NS}\">\
         <head></head><boolean>{value}</boolean></sparql>"
    )
}

fn bind(name: &str, inner: &str) -> String {
    format!("<binding name=\"{name}\">{inner}</binding>")
}

fn result(bindings: &[String]) -> String {
    format!("<result>{}</result>", bindings.concat())
}

/// One `<result>` binding only `?x` to `inner`.
fn one_binding(inner: &str) -> String {
    result(&[bind("x", inner)])
}

fn collect_rows<E: Display>(
    variables: Vec<Variable>,
    solutions: impl Iterator<Item = Result<QuerySolution, E>>,
) -> Result<Outcome, String> {
    let mut rows: Vec<Row> = Vec::new();
    for solution in solutions {
        let solution = solution.map_err(|e| e.to_string())?;
        if rows.len() >= MAX_ROWS {
            return Err("row bound exceeded".to_owned());
        }
        let row = variables.iter().map(|v| solution.get(v).cloned());
        rows.push(row.collect());
    }
    Ok(Outcome::Solutions { variables, rows })
}

fn parse_reader(xml: &[u8]) -> Result<Outcome, String> {
    let parser = QueryResultsParser::from_format(QueryResultsFormat::Xml);
    match parser.for_reader(xml).map_err(|e| e.to_string())? {
        ReaderQueryResultsParserOutput::Solutions(solutions) => {
            let variables = solutions.variables().to_vec();
            collect_rows(variables, solutions)
        }
        ReaderQueryResultsParserOutput::Boolean(value) => Ok(Outcome::Boolean(value)),
    }
}

fn parse_slice(xml: &[u8]) -> Result<Outcome, String> {
    let parser = QueryResultsParser::from_format(QueryResultsFormat::Xml);
    match parser.for_slice(xml).map_err(|e| e.to_string())? {
        SliceQueryResultsParserOutput::Solutions(solutions) => {
            let variables = solutions.variables().to_vec();
            collect_rows(variables, solutions)
        }
        SliceQueryResultsParserOutput::Boolean(value) => Ok(Outcome::Boolean(value)),
    }
}

/// Parses through both routes, fully consuming each, and requires agreement.
fn parse_both(xml: &[u8]) -> Result<Outcome, String> {
    let reader = parse_reader(xml);
    let slice = parse_slice(xml);
    match (&reader, &slice) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "reader and slice routes disagree"),
        (Err(_), Err(_)) => {}
        _ => panic!("routes disagree: reader={reader:?} slice={slice:?}"),
    }
    reader
}

fn expect_ok(xml: &str) -> Outcome {
    match parse_both(xml.as_bytes()) {
        Ok(outcome) => outcome,
        Err(e) => panic!("parse failed: {e}\n{xml}"),
    }
}

fn expect_err(name: &str, xml: &[u8]) {
    let outcome = parse_both(xml);
    assert!(outcome.is_err(), "{name}: expected error: {outcome:?}");
}

fn serialize_solutions(variables: &[&str], rows: &[Row]) -> String {
    let vars: Vec<Variable> = variables.iter().map(|v| var(v)).collect();
    let serializer = QueryResultsSerializer::from_format(QueryResultsFormat::Xml);
    let mut writer = serializer
        .serialize_solutions_to_writer(Vec::new(), vars.clone())
        .expect("serializer start");
    for row in rows {
        let solution: Vec<_> = vars
            .iter()
            .zip(row)
            .filter_map(|(v, t)| t.as_ref().map(|t| (v.as_ref(), t.as_ref())))
            .collect();
        writer.serialize(solution).expect("serializer row");
    }
    let bytes = writer.finish().expect("serializer finish");
    String::from_utf8(bytes).expect("utf8")
}

fn serialize_boolean(value: bool) -> String {
    let serializer = QueryResultsSerializer::from_format(QueryResultsFormat::Xml);
    let out = serializer
        .serialize_boolean_to_writer(Vec::new(), value)
        .expect("serialize boolean");
    String::from_utf8(out).expect("utf8")
}

#[test]
fn should_parse_select_bindings_to_exact_terms() {
    let int = format!("{XSD}integer");
    let r1 = result(&[
        bind("x", "<uri>http://example.org/s1</uri>"),
        bind("y", "<literal>plain</literal>"),
    ]);
    let r2 = result(&[
        bind("x", "<bnode>b0</bnode>"),
        bind("y", &format!("<literal datatype=\"{int}\">42</literal>")),
    ]);
    let xml = select_doc(&["x", "y"], &[r1, r2].concat());
    let expected = select(
        &["x", "y"],
        vec![
            vec![Some(iri("http://example.org/s1")), Some(plain("plain"))],
            vec![Some(bnode("b0")), Some(typed("42", &int))],
        ],
    );
    assert_eq!(expect_ok(&xml), expected);
}

#[test]
fn should_preserve_bag_duplicates_and_row_order() {
    let a = result(&[bind("x", "<literal>a</literal>")]);
    let b = result(&[bind("x", "<literal>b</literal>")]);
    let xml = select_doc(&["x"], &[a.clone(), a.clone(), b, a].concat());
    let row = |v: &str| vec![Some(plain(v))];
    let expected = select(&["x"], vec![row("a"), row("a"), row("b"), row("a")]);
    assert_eq!(expect_ok(&xml), expected);
}

#[test]
fn should_report_unbound_variables_as_none() {
    let r1 = result(&[bind("x", "<literal>only-x</literal>")]);
    let r2 = result(&[bind("y", "<literal>only-y</literal>")]);
    let r3 = "<result></result>";
    let xml = select_doc(&["x", "y"], &[r1, r2, r3.to_owned()].concat());
    let expected = select(
        &["x", "y"],
        vec![
            vec![Some(plain("only-x")), None],
            vec![None, Some(plain("only-y"))],
            vec![None, None],
        ],
    );
    assert_eq!(expect_ok(&xml), expected);
}

#[test]
fn should_keep_variables_when_solutions_are_empty() {
    let expected = select(&["x", "y"], vec![]);
    assert_eq!(expect_ok(&select_doc(&["x", "y"], "")), expected);
}

#[test]
fn should_parse_typed_and_language_literals_exactly() {
    let dec = format!("{XSD}decimal");
    let boolean = format!("{XSD}boolean");
    let string = format!("{XSD}string");
    let custom = "http://example.org/dt#custom";
    let lit = |attrs: &str, text: &str| {
        let literal = format!("<literal {attrs}>{text}</literal>");
        one_binding(&literal)
    };
    let rows = [
        lit(&format!("datatype=\"{dec}\""), "1.50"),
        lit(&format!("datatype=\"{boolean}\""), "true"),
        lit(&format!("datatype=\"{string}\""), "as-string"),
        lit(&format!("datatype=\"{custom}\""), "opaque"),
        lit("xml:lang=\"en\"", "chat"),
        lit("xml:lang=\"fr-CA\"", "chat"),
        lit("xml:lang=\"EN\"", "shout"),
    ];
    let xml = select_doc(&["x"], &rows.concat());
    // sparesults normalises language tags to the lowercase oxrdf canonical form.
    let expected = select(
        &["x"],
        vec![
            vec![Some(typed("1.50", &dec))],
            vec![Some(typed("true", &boolean))],
            vec![Some(plain("as-string"))],
            vec![Some(typed("opaque", custom))],
            vec![Some(lang("chat", "en"))],
            vec![Some(lang("chat", "fr-ca"))],
            vec![Some(lang("shout", "en"))],
        ],
    );
    assert_eq!(expect_ok(&xml), expected);
}

#[test]
fn should_decode_unicode_escapes_and_numeric_references() {
    let cases = [
        ("héllo — 日本 \u{1F600}", "héllo — 日本 \u{1F600}"),
        (
            "&lt;a href=&quot;x&quot;&gt;Tom &amp; Jerry&apos;s&lt;/a&gt;",
            "<a href=\"x\">Tom & Jerry's</a>",
        ),
        ("&#233;&#x1F600;&#x65E5;&#10;end", "é\u{1F600}日\nend"),
        ("x&lt;&amp;&#x41;&#66;y", "x<&ABy"),
    ];
    let body: String = cases
        .iter()
        .map(|(raw, _)| one_binding(&format!("<literal>{raw}</literal>")))
        .collect();
    let mut expected: Vec<Row> = Vec::new();
    for (_, value) in cases {
        expected.push(vec![Some(plain(value))]);
    }
    let got = expect_ok(&select_doc(&["x"], &body));
    assert_eq!(got, select(&["x"], expected));

    let uri_body = one_binding("<uri>http://example.org/?a=1&amp;b=2</uri>");
    let expected_uri = vec![vec![Some(iri("http://example.org/?a=1&b=2"))]];
    let got = expect_ok(&select_doc(&["x"], &uri_body));
    assert_eq!(got, select(&["x"], expected_uri));
}

#[test]
fn should_keep_escaped_xml_like_literal_as_one_literal() {
    let raw = "&lt;/literal&gt;&lt;/binding&gt;&lt;binding name=&quot;evil&quot;&gt;\
               &lt;uri&gt;http://evil.example/&lt;/uri&gt;&lt;/binding&gt;";
    let body = one_binding(&format!("<literal>{raw}</literal>"));
    let text = "</literal></binding><binding name=\"evil\">\
                <uri>http://evil.example/</uri></binding>";
    let expected = select(&["x"], vec![vec![Some(plain(text))]]);
    assert_eq!(expect_ok(&select_doc(&["x"], &body)), expected);
}

#[test]
fn should_parse_ask_true_false_including_numeric_references() {
    for (raw, expected) in [
        ("true", true),
        ("false", false),
        ("&#116;rue", true),
        ("&#x66;alse", false),
    ] {
        let outcome = expect_ok(&bool_doc(raw));
        assert_eq!(outcome, Outcome::Boolean(expected), "boolean {raw}");
    }
}

#[test]
fn should_roundtrip_serialized_solutions_without_injection() {
    let hostile = "</literal></binding></result><result><binding name=\"evil\">\
                   <uri>http://evil.example/</uri></binding></result>\
                   <!DOCTYPE x [<!ENTITY e \"boom\">]>&e;&#x41;<![CDATA[z]]>";
    // Valid IRI (the parser validates datatype IRIs) that still needs escaping.
    let hostile_dt = "http://example.org/dt?x=1&y='2'";
    let query_iri = iri("http://example.org/?a=1&b=2");
    let rows: Vec<Row> = vec![
        vec![Some(plain(hostile)), Some(query_iri)],
        vec![None, Some(lang("héllo \u{1F600}", "en-gb"))],
        vec![Some(typed("<v>&", hostile_dt)), Some(bnode("b7"))],
    ];
    let xml = serialize_solutions(&["x", "y"], &rows);
    assert_eq!(xml.matches("<binding name=\"").count(), 5, "{xml}");
    assert!(!xml.contains("<binding name=\"evil\""), "{xml}");
    assert!(!xml.contains("<!DOCTYPE"), "{xml}");
    assert!(!xml.contains("<!ENTITY"), "{xml}");
    assert!(!xml.contains("<![CDATA["), "{xml}");
    assert!(xml.contains("&amp;e;"), "{xml}");
    assert_eq!(expect_ok(&xml), select(&["x", "y"], rows));
}

#[test]
fn should_roundtrip_empty_solutions_and_ask() {
    let xml = serialize_solutions(&["x"], &[]);
    assert_eq!(expect_ok(&xml), select(&["x"], vec![]));
    for value in [true, false] {
        let xml = serialize_boolean(value);
        assert_eq!(expect_ok(&xml), Outcome::Boolean(value), "{xml}");
    }
}

#[test]
fn should_reject_malformed_documents_on_both_routes() {
    let good = select_doc(&["x"], &one_binding("<uri>http://e/a</uri>"));
    let mismatched = good.replacen("</binding>", "</bindin>", 1);
    let undefined = select_doc(&["x"], &one_binding("<literal>&nope;</literal>"));
    let surrogate = select_doc(&["x"], &one_binding("<literal>&#xD800;</literal>"));
    let nameless_body = "<result><binding><literal>v</literal></binding></result>";
    let nameless = select_doc(&["x"], nameless_body);
    let cases = [
        ("not xml", "this is not xml".to_owned()),
        ("empty input", String::new()),
        ("wrong root", format!("<foo xmlns=\"{NS}\"></foo>")),
        ("mismatched end tag", mismatched),
        ("undefined entity", undefined),
        ("surrogate reference", surrogate),
        ("unknown boolean", bool_doc("maybe")),
        ("binding without name", nameless),
    ];
    for (name, xml) in &cases {
        expect_err(name, xml.as_bytes());
    }
}

#[test]
fn should_reject_truncation_inside_markup() {
    let xml = select_doc(&["x"], &one_binding("<uri>http://e/a</uri>"));
    let markers = [
        "<head>",
        "<variable",
        "<results>",
        "<result>",
        "<binding",
        "<uri>",
        "</uri>",
    ];
    for marker in markers {
        let cut = xml.find(marker).expect("marker present") + 3;
        expect_err(marker, &xml.as_bytes()[..cut]);
    }
    let boolean = bool_doc("true");
    let cut = boolean.find("<boolean>").expect("marker present") + 3;
    expect_err("<boolean>", &boolean.as_bytes()[..cut]);
}

#[test]
fn should_not_fabricate_rows_from_any_truncated_prefix() {
    let r1 = result(&[
        bind("x", "<uri>http://e/a</uri>"),
        bind("y", "<literal>1</literal>"),
    ]);
    let r2 = result(&[bind("x", "<bnode>b1</bnode>")]);
    let xml = select_doc(&["x", "y"], &[r1, r2].concat());
    let (full_vars, full_rows) = match expect_ok(&xml) {
        Outcome::Solutions { variables, rows } => (variables, rows),
        other => panic!("expected solutions, got {other:?}"),
    };
    assert_eq!(full_rows.len(), 2);
    let bytes = xml.as_bytes();
    for len in 0..bytes.len() {
        match parse_both(&bytes[..len]) {
            Err(_) => {}
            Ok(Outcome::Boolean(value)) => panic!("prefix {len} became boolean {value}"),
            Ok(Outcome::Solutions { variables, rows }) => {
                assert_eq!(variables, full_vars, "prefix {len}");
                assert!(rows.len() <= full_rows.len(), "prefix {len}");
                assert_eq!(rows[..], full_rows[..rows.len()], "prefix {len}");
            }
        }
    }
}

#[test]
fn should_characterize_duplicate_head_variable_attribute() {
    let xml = format!(
        "<sparql xmlns=\"{NS}\"><head><variable name=\"x\" name=\"y\"/></head>\
         <results></results></sparql>"
    );
    // Either rejected on both routes, or exactly one (first or last) name wins.
    if let Ok(Outcome::Solutions { variables, rows }) = parse_both(xml.as_bytes()) {
        let allowed = [vec![var("x")], vec![var("y")]];
        assert!(allowed.contains(&variables), "{variables:?}");
        assert!(rows.is_empty());
    }
}

#[test]
fn should_characterize_duplicate_binding_name_attribute() {
    let uri = "<uri>http://e/a</uri>";
    let target = iri("http://e/a");
    let dup = format!("<binding name=\"x\" name=\"y\">{uri}</binding>");
    let flood_attrs = "name=\"x\" ".repeat(64);
    let flood = format!("<binding {flood_attrs}>{uri}</binding>");
    for binding in [dup, flood] {
        let xml = select_doc(&["x", "y"], &format!("<result>{binding}</result>"));
        // Either rejected on both routes, or no binding is duplicated/injected.
        if let Ok(Outcome::Solutions { rows, .. }) = parse_both(xml.as_bytes()) {
            assert_eq!(rows.len(), 1);
            assert!(rows[0].iter().flatten().count() <= 1);
            assert!(rows[0].iter().flatten().all(|t| *t == target));
        }
    }
}

#[test]
fn should_characterize_duplicate_literal_attributes() {
    let int = format!("{XSD}integer");
    let dec = format!("{XSD}decimal");
    let lang_xml = String::from("<literal xml:lang=\"en\" xml:lang=\"fr\">v</literal>");
    let dt_attrs = format!("datatype=\"{int}\" datatype=\"{dec}\"");
    let dt_xml = format!("<literal {dt_attrs}>1</literal>");
    let cases = [
        (lang_xml, vec![lang("v", "en"), lang("v", "fr")]),
        (dt_xml, vec![typed("1", &int), typed("1", &dec)]),
    ];
    for (literal, allowed) in cases {
        let xml = select_doc(&["x"], &one_binding(&literal));
        // Either rejected on both routes, or exactly one declared value wins.
        if let Ok(Outcome::Solutions { rows, .. }) = parse_both(xml.as_bytes()) {
            assert_eq!(rows.len(), 1);
            let term = rows[0][0].clone().expect("bound");
            assert!(allowed.contains(&term), "unexpected term {term:?}");
        }
    }
}
