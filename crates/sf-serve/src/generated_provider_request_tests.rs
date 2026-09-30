//! Preparation-only contract tests for [`crate::generated_provider_request`].
//! Every fixture here proves rendering, bounds and digests only. None is parsed
//! admission, a source/owner/grant check or a live endpoint, and none may be
//! handed off as evidence of live admission.

use sha2::{Digest, Sha256};

use crate::generated_provider_request::{self as request, PreparedGeneratedRequest as Prepared};
use crate::generated_provider_templates::{ENUMERATE_TEMPLATE, ROOT_TEMPLATE};

type Error = request::GeneratedProviderRequestError;

const STYLE: &str = "STYLE-001";
const ENUMERATE_TEMPLATE_SHA256: &str =
    "sha256:3459cb4a268ffc242de3c3afcc032f8001792958537b1303901c76a75ae2e2ec";
const ROOT_TEMPLATE_SHA256: &str =
    "sha256:7fb994ca6015966d8be3eb823e61df6080926b0cbcd991b118e86f6f7a74b2a3";
const JSON_HEAD: &str = r#"{"schemaVersion":"semantic-fabric.generated-provider-request.v1","#;
const ROOT_JSON_TAIL: &str = r#""selector":{"C2SuccessorRootV1":{"styleNumber":"STYLE-001"}}}"#;
const FIRST_JSON_TAIL: &str = r#""selector":{"C2SuccessorEnumerateV1":{"afterStyleNumber":""}}}"#;

const ENUMERATE_FIRST: &str = r#"SELECT DISTINCT ?styleNumber ?styleCount
WHERE {
  {
    SELECT (COUNT(DISTINCT ?countedStyle) AS ?styleCount)
    WHERE {
      ?countedSubject
        a <https://hm.com/ns/semantic-product-mock/product-design/Style> ;
        <https://hm.com/ns/semantic-product-mock/product-design/Style/field/StyleNumber> ?countedStyle .
    }
  }
  ?subject
    a <https://hm.com/ns/semantic-product-mock/product-design/Style> ;
    <https://hm.com/ns/semantic-product-mock/product-design/Style/field/StyleNumber> ?styleNumber .
  FILTER(STR(?styleNumber) > "")
}
ORDER BY ?styleNumber
LIMIT 65
"#;

const INJECTED: &[&str] = &[
    "INJECT\"quote",
    "INJECT\\backslash",
    "INJECT space",
    "INJECT\nnewline",
    "INJECT\rreturn",
    "INJECT\ttab",
    "INJECT\0nul",
    "INJECT\u{7f}delete",
    "INJECT\u{e9}accent",
    "INJECT{{STYLE_NUMBER}}",
    "INJECT\") } UNION { ?s ?p ?o } #",
    "INJECT'single",
    "INJECT.dot",
    "INJECT>angle",
];

fn limit(bytes: usize) -> request::QueryByteLimit {
    request::QueryByteLimit::new(bytes).unwrap()
}

fn widest() -> request::QueryByteLimit {
    limit(request::MAX_QUERY_BYTES_CEILING)
}

fn digest(bytes: &[u8]) -> String {
    let hex: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("sha256:{hex}")
}

fn member(key: &str, value: &str) -> String {
    format!("\"{key}\":{value}")
}

fn object(members: &[&str]) -> String {
    format!("{{{}}}", members.join(","))
}

fn document(selector: &str) -> String {
    let version = request::SCHEMA_VERSION;
    format!(r#"{{"schemaVersion":"{version}","selector":{selector}}}"#)
}

fn refused(document: &[u8]) -> Error {
    Prepared::from_json(document, widest()).unwrap_err()
}

/// A schema-v1 document whose parameter `field` holds the JSON text `value`.
fn request_json(selector: &str, field: &str, value: &str) -> String {
    let parameter = object(&[&member(field, value)]);
    document(&object(&[&member(selector, &parameter)]))
}

/// A quoted JSON string with every character escaped, so it decodes only
/// through the parser's scratch buffer.
fn escaped(text: &str) -> String {
    let escape = |c: char| format!("\\u{:04x}", u32::from(c));
    let body: String = text.chars().map(escape).collect();
    format!("\"{body}\"")
}

#[test]
fn generated_provider_template_assets_match_pinned_query_hashes() {
    let enumerate = ENUMERATE_TEMPLATE;
    let root = ROOT_TEMPLATE;
    assert_eq!(enumerate.len(), 605);
    assert_eq!(root.len(), 7528);
    assert_eq!(digest(enumerate.as_bytes()), ENUMERATE_TEMPLATE_SHA256);
    assert_eq!(digest(root.as_bytes()), ROOT_TEMPLATE_SHA256);
    let pinned = request::C2_SUCCESSOR_ENUMERATE_V1_DIGEST;
    assert_eq!(pinned, ENUMERATE_TEMPLATE_SHA256);
    let pinned = request::C2_SUCCESSOR_ROOT_V1_DIGEST;
    assert_eq!(pinned, ROOT_TEMPLATE_SHA256);
    let id = request::C2_SUCCESSOR_ENUMERATE_V1_ID;
    assert_eq!(id, "c2-successor-enumerate-v1");
    let id = request::C2_SUCCESSOR_ROOT_V1_ID;
    assert_eq!(id, "c2-successor-root-v1");
    assert_eq!(enumerate.matches("{{AFTER_STYLE_NUMBER}}").count(), 1);
    assert_eq!(enumerate.matches("{{STYLE_NUMBER}}").count(), 0);
    assert_eq!(root.matches("{{STYLE_NUMBER}}").count(), 10);
    assert_eq!(root.matches("{{AFTER_STYLE_NUMBER}}").count(), 0);
    assert!(enumerate.ends_with("ORDER BY ?styleNumber\nLIMIT 65\n"));
    assert!(root.ends_with("}\nLIMIT 11\n"));
    assert!(!enumerate.contains("FROM") && !root.contains("FROM"));
}

#[test]
fn generated_provider_enumerate_first_page_is_exact() {
    let max = widest();
    let prepared = Prepared::c2_successor_enumerate("", max).unwrap();
    assert_eq!(prepared.query(), ENUMERATE_FIRST);
    assert_eq!(prepared.query().len(), 583);
    assert_eq!(prepared.query_bytes(), ENUMERATE_FIRST.as_bytes());
    let expected = digest(ENUMERATE_FIRST.as_bytes());
    assert_eq!(prepared.query_digest(), expected);
    let selector = request::RequestSelector::C2SuccessorEnumerateV1;
    assert_eq!(prepared.selector(), selector);
    assert_eq!(prepared.template_id(), Some("c2-successor-enumerate-v1"));
    let pinned = Some(ENUMERATE_TEMPLATE_SHA256);
    assert_eq!(prepared.template_digest(), pinned);
}

#[test]
fn generated_provider_enumerate_next_page_renders_cursor() {
    let max = widest();
    let cursor = "HM-0001_b";
    let first = Prepared::c2_successor_enumerate("", max).unwrap();
    let next = Prepared::c2_successor_enumerate(cursor, max).unwrap();
    let expected = ENUMERATE_FIRST.replace("\"\"", "\"HM-0001_b\"");
    assert_eq!(next.query(), expected);
    assert_eq!(next.query().len(), 583 + 9);
    assert!(next.query().contains("> \"HM-0001_b\")\n}\nORDER BY"));
    let expected_digest = digest(expected.as_bytes());
    assert_eq!(next.query_digest(), expected_digest);
    assert_ne!(next.query_digest(), first.query_digest());
    assert_eq!(next.template_digest(), first.template_digest());
    assert_eq!(next.template_id(), first.template_id());
}

#[test]
fn generated_provider_root_replaces_every_placeholder() {
    let max = widest();
    let prepared = Prepared::c2_successor_root(STYLE, max).unwrap();
    let expected = ROOT_TEMPLATE.replace("{{STYLE_NUMBER}}", STYLE);
    assert_eq!(prepared.query(), expected);
    assert_eq!(prepared.query().len(), 7458);
    assert_eq!(prepared.query().matches("\"STYLE-001\"").count(), 10);
    assert!(!prepared.query().contains("{{"));
    assert!(prepared.query().ends_with("}\nLIMIT 11\n"));
    let expected_digest = digest(expected.as_bytes());
    assert_eq!(prepared.query_digest(), expected_digest);
    assert_eq!(prepared.template_id(), Some("c2-successor-root-v1"));
    let pinned = Some(ROOT_TEMPLATE_SHA256);
    assert_eq!(prepared.template_digest(), pinned);
    let other = Prepared::c2_successor_root("STYLE-002", max).unwrap();
    assert_ne!(other.query_digest(), prepared.query_digest());
    assert_eq!(other.template_digest(), prepared.template_digest());
}

#[test]
fn generated_provider_raw_query_keeps_exact_bytes_and_digest() {
    let max = widest();
    let abc = Prepared::generated_query("abc", max).unwrap();
    let golden = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    assert_eq!(abc.query_digest(), golden);
    assert_eq!(abc.template_id(), None);
    assert_eq!(abc.template_digest(), None);
    assert_eq!(abc.selector(), request::RequestSelector::GeneratedQuery);
    let unicode = "SELECT ?x WHERE { ?x <urn:ex:name> \"Grüße 🚀 名前\" }";
    let prepared = Prepared::generated_query(unicode, max).unwrap();
    assert_eq!(prepared.query_bytes(), unicode.as_bytes());
    assert_eq!(prepared.query_digest(), digest(unicode.as_bytes()));
    // Not scanned or parsed here: parsing and admission stay in the engine.
    let unchecked = [
        "DELETE WHERE { ?s ?p ?o }",
        "CONSTRUCT WHERE { ?s ?p ?o }",
        "not sparql at all",
    ];
    for text in unchecked {
        let prepared = Prepared::generated_query(text, max).unwrap();
        assert_eq!(prepared.query(), text);
    }
}

#[test]
fn generated_provider_json_decodes_unicode_escapes_exactly() {
    let max = widest();
    let selector = r#"{"GeneratedQuery":{"sparql":"ASK { ?s ?p \"ü🚀\" }"}}"#;
    let json = document(selector);
    let parsed = Prepared::from_json(json.as_bytes(), max).unwrap();
    let expected = "ASK { ?s ?p \"ü🚀\" }";
    assert_eq!(parsed.query(), expected);
    assert_eq!(parsed.query_digest(), digest(expected.as_bytes()));
    let direct = Prepared::generated_query(expected, max).unwrap();
    assert_eq!(parsed, direct);
}

#[test]
fn generated_provider_json_rejects_unknown_versions_fields_and_selectors() {
    let version = request::SCHEMA_VERSION;
    let schema = member("schemaVersion", &format!("\"{version}\""));
    let sparql_member = member("sparql", "\"ASK {}\"");
    let sparql = object(&[&sparql_member]);
    let style = object(&[&member("styleNumber", "\"A1\"")]);
    let query_entry = member("GeneratedQuery", &sparql);
    let root_entry = member("C2SuccessorRootV1", &style);
    let selected = member("selector", &object(&[&query_entry]));

    let top_level = [
        object(&[]),
        "[]".to_owned(),
        "null".to_owned(),
        object(&[&schema]),
        object(&[&selected]),
        object(&[&schema, &selected, "\"extra\":1"]),
        object(&[&schema, &schema, &selected]),
        object(&[&schema, &selected, &selected]),
        object(&[&member("schemaVersion", "1"), &selected]),
        format!("{} x", object(&[&schema, &selected])),
    ];
    for case in &top_level {
        let error = refused(case.as_bytes());
        assert_eq!(error, Error::MalformedDocument, "{case}");
    }
    let error = refused(b"\xff");
    assert_eq!(error, Error::MalformedDocument);

    let extra = object(&[&sparql_member, "\"graph\":\"x\""]);
    let duplicate = object(&[&sparql_member, &sparql_member]);
    let number = object(&[&member("styleNumber", "7")]);
    let null = object(&[&member("styleNumber", "null")]);
    let selectors = [
        object(&[]),
        "\"GeneratedQuery\"".to_owned(),
        object(&[&member("C2SuccessorRootV2", &style)]),
        object(&[&member("generatedQuery", &sparql)]),
        object(&[&query_entry, &root_entry]),
        object(&[&query_entry, &query_entry]),
        object(&[&member("GeneratedQuery", "[\"ASK {}\"]")]),
        object(&[&member("GeneratedQuery", "null")]),
        object(&[&member("GeneratedQuery", &extra)]),
        object(&[&member("GeneratedQuery", &duplicate)]),
        object(&[&member("C2SuccessorRootV1", &sparql)]),
        object(&[&member("C2SuccessorRootV1", &number)]),
        object(&[&member("C2SuccessorRootV1", &null)]),
    ];
    for selector in &selectors {
        let case = object(&[&schema, &member("selector", selector)]);
        let error = refused(case.as_bytes());
        assert_eq!(error, Error::MalformedDocument, "{case}");
    }

    let versions = [
        "semantic-fabric.generated-provider-request.v2",
        "semantic-fabric.generated-provider-request.V1",
        " semantic-fabric.generated-provider-request.v1",
        "semantic-fabric.generated-provider-request",
        "",
    ];
    for other in versions {
        let other = member("schemaVersion", &format!("\"{other}\""));
        let case = object(&[&other, &selected]);
        assert_eq!(refused(case.as_bytes()), Error::UnsupportedSchema);
    }

    let expected = Prepared::generated_query("ASK {}", widest()).unwrap();
    let reordered = object(&[&selected, &schema]);
    let spaced = format!(" \n{}\t ", object(&[&schema, &selected]));
    for case in [reordered, spaced] {
        let parsed = Prepared::from_json(case.as_bytes(), widest());
        assert_eq!(parsed.unwrap(), expected);
    }
}

#[test]
fn generated_provider_json_round_trips_with_exact_bytes() {
    let max = widest();
    let root = Prepared::c2_successor_root(STYLE, max).unwrap();
    assert_eq!(root.to_json(), [JSON_HEAD, ROOT_JSON_TAIL].concat());
    let first = Prepared::c2_successor_enumerate("", max).unwrap();
    assert_eq!(first.to_json(), [JSON_HEAD, FIRST_JSON_TAIL].concat());
    let raw = "ASK { \"q\\\"\" \n\t\u{1}ü }";
    let generated = Prepared::generated_query(raw, max).unwrap();
    for prepared in [root, first, generated] {
        let json = prepared.to_json();
        let parsed = Prepared::from_json(json.as_bytes(), max).unwrap();
        assert_eq!(parsed, prepared);
    }
}

#[test]
fn generated_provider_style_parameters_reject_injection_and_redact() {
    let max = widest();
    let overlong = format!("INJECT{}", "a".repeat(59));
    let mut cases: Vec<&str> = INJECTED.to_vec();
    cases.push(&overlong);
    for case in cases {
        let value = serde_json::Value::from(case).to_string();
        let enumerate = request_json("C2SuccessorEnumerateV1", "afterStyleNumber", &value);
        let root = request_json("C2SuccessorRootV1", "styleNumber", &value);
        let errors = [
            Prepared::c2_successor_enumerate(case, max).unwrap_err(),
            Prepared::c2_successor_root(case, max).unwrap_err(),
            refused(enumerate.as_bytes()),
            refused(root.as_bytes()),
        ];
        for error in errors {
            assert_eq!(error, Error::InvalidStyleNumber);
            let shown = format!("{error} {error:?}");
            assert!(!shown.contains("INJECT"));
        }
    }
    let empty = Prepared::c2_successor_root("", max).unwrap_err();
    assert_eq!(empty, Error::InvalidStyleNumber);
    let empty = request_json("C2SuccessorRootV1", "styleNumber", "\"\"");
    assert_eq!(refused(empty.as_bytes()), Error::InvalidStyleNumber);
    let empty = request_json("GeneratedQuery", "sparql", "\"\"");
    assert_eq!(refused(empty.as_bytes()), Error::EmptyQuery);
}

#[test]
fn generated_provider_escaped_fields_are_checked_before_copying() {
    let (small, max) = (limit(16), widest());
    let over = request_json("GeneratedQuery", "sparql", &escaped(&"A".repeat(17)));
    let error = Prepared::from_json(over.as_bytes(), small).unwrap_err();
    assert_eq!(error, Error::QueryTooLarge);
    // A field refusal never outranks a malformed document.
    let trailing = format!("{over} x");
    let error = Prepared::from_json(trailing.as_bytes(), small).unwrap_err();
    assert_eq!(error, Error::MalformedDocument);
    let fits = request_json("GeneratedQuery", "sparql", &escaped(&"A".repeat(16)));
    let parsed = Prepared::from_json(fits.as_bytes(), small).unwrap();
    assert_eq!(parsed.query(), "A".repeat(16));
    // Escaped keys and schema version are compared after decoding, uncopied.
    let keys = fits
        .replace("sparql", "sparq\\u006c")
        .replace("GeneratedQuery", "Generated\\u0051uery")
        .replace(".v1", "\\u002ev1");
    let decoded = Prepared::from_json(keys.as_bytes(), small).unwrap();
    assert_eq!(decoded, parsed);
    let unknown = fits.replace("sparql", "sparq\\u006c2");
    assert_eq!(refused(unknown.as_bytes()), Error::MalformedDocument);
    let styles = [
        ("C2SuccessorEnumerateV1", "afterStyleNumber"),
        ("C2SuccessorRootV1", "styleNumber"),
    ];
    let invalid = [escaped(&"a".repeat(65)), r#""A\"B""#.to_owned()];
    for (selector, field) in styles {
        for value in &invalid {
            let error = refused(request_json(selector, field, value).as_bytes());
            assert_eq!(error, Error::InvalidStyleNumber);
        }
        let wide = request_json(selector, field, &escaped(&"A".repeat(64)));
        assert!(Prepared::from_json(wide.as_bytes(), max).is_ok());
    }
}

#[test]
fn generated_provider_debug_redacts_query_and_parameters() {
    let max = widest();
    let text = "ASK { SECRETQUERY }";
    let raw = Prepared::generated_query(text, max).unwrap();
    let root = Prepared::c2_successor_root("SECRETSTYLE", max).unwrap();
    for prepared in [raw, root] {
        let shown = format!("{prepared:?}");
        assert!(!shown.contains("SECRET"), "{shown}");
        assert!(!shown.contains(prepared.query_digest()));
        assert!(!shown.contains("sha256:"));
    }
}

#[test]
fn generated_provider_query_limits_are_explicit_and_exact() {
    let ceiling = request::MAX_QUERY_BYTES_CEILING;
    assert_eq!(ceiling, 1024 * 1024);
    let invalid = [0, ceiling + 1, usize::MAX];
    for bytes in invalid {
        let error = request::QueryByteLimit::new(bytes).unwrap_err();
        assert_eq!(error, Error::InvalidQueryLimit);
    }
    assert_eq!(limit(ceiling).max_query_bytes(), ceiling);
    assert_eq!(limit(16).max_document_bytes(), 16 * 6 + 4096);
    let small = limit(8);
    assert!(Prepared::generated_query("12345678", small).is_ok());
    let nine = "123456789";
    let error = Prepared::generated_query(nine, small).unwrap_err();
    assert_eq!(error, Error::QueryTooLarge);
    // Bytes, not characters: a two-byte character exceeds a one-byte limit.
    let error = Prepared::generated_query("ü", limit(1)).unwrap_err();
    assert_eq!(error, Error::QueryTooLarge);
    let error = Prepared::generated_query("", small).unwrap_err();
    assert_eq!(error, Error::EmptyQuery);
    // Rendered templates obey the same limit, measured before allocation.
    let (fits, short) = (limit(583), limit(582));
    assert!(Prepared::c2_successor_enumerate("", fits).is_ok());
    let error = Prepared::c2_successor_enumerate("", short).unwrap_err();
    assert_eq!(error, Error::QueryTooLarge);
    let wide = "a".repeat(64);
    let (fits, short) = (limit(8008), limit(8007));
    assert!(Prepared::c2_successor_root(&wide, fits).is_ok());
    let error = Prepared::c2_successor_root(&wide, short).unwrap_err();
    assert_eq!(error, Error::QueryTooLarge);
}

#[test]
fn generated_provider_document_bound_is_checked_before_parsing() {
    let small = limit(16);
    let bound = small.max_document_bytes();
    let oversized = vec![b'x'; bound + 1];
    let error = Prepared::from_json(&oversized, small).unwrap_err();
    assert_eq!(error, Error::DocumentTooLarge);
    let at_bound = vec![b'x'; bound];
    let error = Prepared::from_json(&at_bound, small).unwrap_err();
    assert_eq!(error, Error::MalformedDocument);
    let valid = document(r#"{"GeneratedQuery":{"sparql":"ASK {}"}}"#);
    let padded = format!("{valid:<width$}", width = bound + 1);
    let bytes = padded.as_bytes();
    let error = Prepared::from_json(bytes, small).unwrap_err();
    assert_eq!(error, Error::DocumentTooLarge);
    let fitted = format!("{valid:<width$}", width = bound);
    assert!(Prepared::from_json(fitted.as_bytes(), small).is_ok());
    let long = document(r#"{"GeneratedQuery":{"sparql":"ASK { 1234567890 }"}}"#);
    let error = Prepared::from_json(long.as_bytes(), small).unwrap_err();
    assert_eq!(error, Error::QueryTooLarge);
}

#[test]
fn generated_provider_document_bound_admits_worst_case_escapes() {
    // A control byte is escaped as six bytes, the widest expansion per byte.
    for bytes in [1, 4096, request::MAX_QUERY_BYTES_CEILING] {
        let max = limit(bytes);
        let control = "\u{1}".repeat(bytes);
        let prepared = Prepared::generated_query(&control, max).unwrap();
        let json = prepared.to_json();
        assert!(json.len() <= max.max_document_bytes());
        let parsed = Prepared::from_json(json.as_bytes(), max).unwrap();
        assert_eq!(parsed, prepared);
    }
}

#[test]
fn generated_provider_preparation_is_pure_and_errors_carry_no_input() {
    let max = widest();
    let first = Prepared::c2_successor_root(STYLE, max).unwrap();
    // Refusals in between leave no state behind.
    for case in INJECTED {
        assert!(Prepared::c2_successor_root(case, max).is_err());
    }
    assert!(Prepared::c2_successor_root(STYLE, limit(1)).is_err());
    let again = Prepared::c2_successor_root(STYLE, max).unwrap();
    assert_eq!(first, again);
    assert_eq!(first.clone().into_query(), first.query());
    assert_eq!(digest(ROOT_TEMPLATE.as_bytes()), ROOT_TEMPLATE_SHA256);
    assert_eq!(std::mem::size_of::<Error>(), 1);
}

#[test]
fn generated_provider_transport_constants_name_the_existing_endpoint() {
    assert_eq!(request::DELIVERY_METHOD, "POST");
    assert_eq!(request::DELIVERY_PATH, "/sparql");
    let content_type = request::DELIVERY_CONTENT_TYPE;
    assert_eq!(content_type, "application/sparql-query");
    let accept = request::DELIVERY_ACCEPT;
    assert_eq!(accept, "application/sparql-results+json");
    let selector = request::RequestSelector::C2SuccessorEnumerateV1;
    assert_eq!(selector.name(), "C2SuccessorEnumerateV1");
    let selector = request::RequestSelector::C2SuccessorRootV1;
    assert_eq!(selector.name(), "C2SuccessorRootV1");
    let selector = request::RequestSelector::GeneratedQuery;
    assert_eq!(selector.name(), "GeneratedQuery");
}
