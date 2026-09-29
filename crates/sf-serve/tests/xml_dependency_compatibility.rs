//! Public XML SPARQL-results compatibility/security regression for the G6
//! quick-xml remediation: real in-memory SQLite behind the ordinary Router, XML
//! negotiated via `Accept`, response parsed with sparesults and compared to
//! exact RDF terms and to the JSON response. Re-run unchanged against any
//! replacement quick-xml source; passing does not close open advisories.
//! Standalone malformed/duplicate-attribute parser cases live in the accepted
//! library corpus and are deliberately not duplicated here.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use oxrdf::{Literal, NamedNode, Term};
use rusqlite::params;
use sf_serve::{router, Backend, ServeConfig};
use sparesults::{QueryResultsFormat, QueryResultsParser, ReaderQueryResultsParserOutput};
use tower::ServiceExt;

mod support;

const XML: &str = "application/sparql-results+xml";
const JSON: &str = "application/sparql-results+json";
const RESULTS_NS: &str = "http://www.w3.org/2005/sparql-results#";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const CODE_DT: &str = "http://ex/code";

const S1: &str = r#"<a href="x">&amp; 'q' ]]> &lt;</a>"#;
const S2: &str = r#"</literal></binding><binding name="evil"><literal>x</literal>"#;
const S3: &str = r#"<!DOCTYPE x [<!ENTITY e "BOOM">]>&e;&#x41;<![CDATA[c]]>"#;
const S4: &str = "Zoë – 日本語 😀 ñ";
const NICK_MARKUP: &str = "<x/> & <y a='1'/>";

// (id, name, age, kind, nick)
const ROWS: &[(i64, &str, i64, &str, Option<&str>)] = &[
    (1, "Alice", 30, "x", Some("Al")),
    (2, "Bob", 25, "x", None),
    (3, S1, 41, "y", None),
    (4, S2, 42, "y", Some(NICK_MARKUP)),
    (5, S3, 43, "x", None),
    (6, S4, 44, "z", None),
];

/// Raw markup from stored literals that must never appear unescaped.
const RAW_MARKUP: &[&str] = &[
    "<binding name=\"evil\"",
    "<!DOCTYPE",
    "<!ENTITY",
    "<![CDATA[",
    "<a href",
    "<x/>",
];

const XML_ACCEPTS: &[&str] = &[
    "APPLICATION/SPARQL-RESULTS+XML",
    "application/xml",
    "text/xml",
    "application/sparql-results+json;q=0.1, application/sparql-results+xml",
];

const CREATE_SQL: &str = r#"
CREATE TABLE "Items" ("id" INTEGER PRIMARY KEY, "name" TEXT, "label" TEXT,
  "code" TEXT, "age" INTEGER, "kind" TEXT, "nick" TEXT);
"#;

const INSERT_SQL: &str = r#"INSERT INTO "Items" VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6)"#;

const BULK_SQL: &str = r#"
WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < ?1)
INSERT INTO "Items" SELECT 1000 + x, 'Bulk ' || x || ' <é&>', 'l', 'c' || (1000 + x),
  x % 100, 'bulk', NULL FROM c"#;

const MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<#Items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "Items" ] ;
  rr:subjectMap [ rr:template "http://ex/person/{id}" ; rr:class ex:Item ] ;
  rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] ;
  rr:predicateObjectMap [ rr:predicate ex:label ; rr:objectMap [ rr:column "label" ; rr:language "fr" ] ] ;
  rr:predicateObjectMap [ rr:predicate ex:code ; rr:objectMap [ rr:column "code" ; rr:datatype ex:code ] ] ;
  rr:predicateObjectMap [ rr:predicate ex:age ; rr:objectMap [ rr:column "age" ; rr:datatype xsd:integer ] ] ;
  rr:predicateObjectMap [ rr:predicate ex:kind ; rr:objectMap [ rr:column "kind" ] ] ;
  rr:predicateObjectMap [ rr:predicate ex:nick ; rr:objectMap [ rr:column "nick" ] ] .
"#;

fn config(bulk: i64) -> Arc<ServeConfig> {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(CREATE_SQL).unwrap();
    for &(id, name, age, kind, nick) in ROWS {
        let code = format!("c{id}");
        let inserted = conn.execute(INSERT_SQL, params![id, name, code, age, kind, nick]);
        assert_eq!(inserted.unwrap(), 1);
    }
    if bulk > 0 {
        conn.execute(BULK_SQL, [bulk]).unwrap();
    }
    Arc::new(support::serve_config(Backend::sqlite(conn), MAPPING_TTL))
}

type Response = (StatusCode, String, Vec<u8>);

async fn fetch(cfg: &Arc<ServeConfig>, query: &str, accept: &str) -> Response {
    let req = Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, accept)
        .body(Body::from(query.to_owned()))
        .unwrap();
    let resp = router(cfg.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let ctype = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, ctype, bytes.to_vec())
}

#[derive(Debug, PartialEq)]
enum Parsed {
    Solutions {
        vars: Vec<String>,
        rows: Vec<Vec<Option<Term>>>,
    },
    Boolean(bool),
}

fn sort_rows(rows: &mut [Vec<Option<Term>>]) {
    rows.sort_by_cached_key(|r| format!("{r:?}"));
}

fn parse(format: QueryResultsFormat, bytes: &[u8]) -> Result<Parsed, String> {
    let output = QueryResultsParser::from_format(format)
        .for_reader(bytes)
        .map_err(|e| e.to_string())?;
    match output {
        ReaderQueryResultsParserOutput::Solutions(solutions) => {
            let vars: Vec<String> = solutions
                .variables()
                .iter()
                .map(|v| v.as_str().to_owned())
                .collect();
            let mut rows: Vec<Vec<Option<Term>>> = Vec::new();
            for solution in solutions {
                let solution = solution.map_err(|e| e.to_string())?;
                let row = vars.iter().map(|v| solution.get(v.as_str()).cloned());
                rows.push(row.collect());
            }
            sort_rows(&mut rows);
            Ok(Parsed::Solutions { vars, rows })
        }
        ReaderQueryResultsParserOutput::Boolean(b) => Ok(Parsed::Boolean(b)),
    }
}

/// Run `query` for XML and JSON, require 200 + negotiated content types, and
/// require both parsed responses to be identical. Returns the XML parse and body.
async fn xml_and_json(cfg: &Arc<ServeConfig>, query: &str) -> (Parsed, Vec<u8>) {
    let (sx, cx, bx) = fetch(cfg, query, XML).await;
    assert_eq!(sx, StatusCode::OK, "{}", String::from_utf8_lossy(&bx));
    assert!(cx.starts_with(XML), "xml ctype={cx}");
    let text = String::from_utf8(bx.clone()).expect("XML body is UTF-8");
    assert!(text.contains("<sparql"), "{text}");
    assert!(text.contains(RESULTS_NS), "{text}");
    let (sj, cj, bj) = fetch(cfg, query, JSON).await;
    assert_eq!(sj, StatusCode::OK, "{}", String::from_utf8_lossy(&bj));
    assert!(cj.starts_with(JSON), "json ctype={cj}");
    let xml = parse(QueryResultsFormat::Xml, &bx).expect("XML response must parse");
    let json = parse(QueryResultsFormat::Json, &bj).expect("JSON response must parse");
    assert_eq!(xml, json, "XML/JSON terms differ");
    (xml, bx)
}

fn solutions(vars: &[&str], mut rows: Vec<Vec<Option<Term>>>) -> Parsed {
    sort_rows(&mut rows);
    Parsed::Solutions {
        vars: vars.iter().map(|v| (*v).to_owned()).collect(),
        rows,
    }
}

fn column(values: &[&str]) -> Vec<Vec<Option<Term>>> {
    values.iter().map(|v| vec![Some(plain(v))]).collect()
}

fn iri(s: &str) -> Term {
    NamedNode::new_unchecked(s).into()
}

fn plain(s: &str) -> Term {
    Literal::new_simple_literal(s).into()
}

fn lang(value: &str, tag: &str) -> Term {
    Literal::new_language_tagged_literal_unchecked(value, tag).into()
}

fn typed(value: impl Into<String>, datatype: &str) -> Term {
    Literal::new_typed_literal(value, NamedNode::new_unchecked(datatype)).into()
}

fn person(id: i64) -> Term {
    iri(&format!("http://ex/person/{id}"))
}

#[tokio::test]
async fn select_exact_terms_typed_lang_unbound_and_special_characters() {
    let cfg = config(0);
    let query = "SELECT ?s ?name ?label ?code ?age ?nick WHERE { \
        ?s <http://ex/name> ?name ; <http://ex/label> ?label ; \
        <http://ex/code> ?code ; <http://ex/age> ?age . \
        OPTIONAL { ?s <http://ex/nick> ?nick } }";
    let (xml, _) = xml_and_json(&cfg, query).await;
    let expected = ROWS
        .iter()
        .map(|&(id, name, age, _, nick)| {
            vec![
                Some(person(id)),
                Some(plain(name)),
                Some(lang(name, "fr")),
                Some(typed(format!("c{id}"), CODE_DT)),
                Some(typed(age.to_string(), XSD_INTEGER)),
                nick.map(plain),
            ]
        })
        .collect();
    let vars = ["s", "name", "label", "code", "age", "nick"];
    assert_eq!(xml, solutions(&vars, expected));
}

#[tokio::test]
async fn select_bag_multiplicity_and_distinct() {
    let cfg = config(0);
    let query = "SELECT ?kind WHERE { ?s <http://ex/kind> ?kind }";
    let (bag, _) = xml_and_json(&cfg, query).await;
    let kinds: Vec<&str> = ROWS.iter().map(|r| r.3).collect();
    assert_eq!(bag, solutions(&["kind"], column(&kinds)));

    let query = "SELECT DISTINCT ?kind WHERE { ?s <http://ex/kind> ?kind }";
    let (distinct, _) = xml_and_json(&cfg, query).await;
    assert_eq!(distinct, solutions(&["kind"], column(&["x", "y", "z"])));
}

#[tokio::test]
async fn select_all_unbound_row_is_preserved() {
    let cfg = config(0);
    let query = "SELECT ?nick WHERE { ?s <http://ex/name> \"Bob\" . \
        OPTIONAL { ?s <http://ex/nick> ?nick } }";
    let (xml, _) = xml_and_json(&cfg, query).await;
    assert_eq!(xml, solutions(&["nick"], vec![vec![None]]));
}

#[tokio::test]
async fn xml_like_literals_stay_literal_and_cannot_inject() {
    let cfg = config(0);
    let query = "SELECT ?name ?nick WHERE { ?s <http://ex/name> ?name . \
        OPTIONAL { ?s <http://ex/nick> ?nick } }";
    let (xml, body) = xml_and_json(&cfg, query).await;
    let expected = ROWS
        .iter()
        .map(|r| vec![Some(plain(r.1)), r.4.map(plain)])
        .collect();
    assert_eq!(xml, solutions(&["name", "nick"], expected));

    let text = String::from_utf8(body).unwrap();
    let nicks = ROWS.iter().filter(|r| r.4.is_some()).count();
    let results = text.matches("<result>").count();
    let bindings = text.matches("<binding ").count();
    assert_eq!(results, ROWS.len(), "{text}");
    assert_eq!(bindings, ROWS.len() + nicks, "{text}");
    for raw in RAW_MARKUP {
        assert!(!text.contains(*raw), "raw `{raw}` leaked:\n{text}");
    }

    // The same markup as a query literal binds exactly the one row storing it.
    let query = format!("SELECT ?s WHERE {{ ?s <http://ex/name> '''{S2}''' }}");
    let (hit, _) = xml_and_json(&cfg, &query).await;
    assert_eq!(hit, solutions(&["s"], vec![vec![Some(person(4))]]));
}

#[tokio::test]
async fn ask_true_and_false() {
    let cfg = config(0);
    for (literal, expected) in [("Alice", true), ("Nobody", false)] {
        let query = format!("ASK {{ ?s <http://ex/name> \"{literal}\" }}");
        let (xml, body) = xml_and_json(&cfg, &query).await;
        assert_eq!(xml, Parsed::Boolean(expected));
        let text = String::from_utf8(body).unwrap();
        let boolean = format!("<boolean>{expected}</boolean>");
        assert!(text.contains(&boolean), "{text}");
    }
}

#[tokio::test]
async fn empty_result_keeps_head_and_has_no_rows() {
    let cfg = config(0);
    let query = "SELECT ?s ?name WHERE { ?s <http://ex/name> \"Nobody\" ; \
        <http://ex/name> ?name }";
    let (xml, _) = xml_and_json(&cfg, query).await;
    assert_eq!(xml, solutions(&["s", "name"], Vec::new()));
}

#[tokio::test]
async fn multi_chunk_stream_completes_with_every_row() {
    let bulk = 1500;
    let cfg = config(bulk);
    let query = "SELECT ?name WHERE { ?s <http://ex/name> ?name }";
    let (xml, body) = xml_and_json(&cfg, query).await;
    assert!(body.len() > 16 * 1024, "{} bytes", body.len());
    let text = String::from_utf8(body).unwrap();
    assert!(text.trim_end().ends_with("</sparql>"));
    let mut names: Vec<String> = ROWS.iter().map(|r| r.1.to_owned()).collect();
    names.extend((1..=bulk).map(|x| format!("Bulk {x} <é&>")));
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(xml, solutions(&["name"], column(&names)));
}

#[tokio::test]
async fn accept_variants_select_xml() {
    let cfg = config(0);
    let query = "SELECT ?kind WHERE { ?s <http://ex/kind> ?kind }";
    let (reference, _) = xml_and_json(&cfg, query).await;
    for accept in XML_ACCEPTS {
        let (status, ctype, body) = fetch(&cfg, query, accept).await;
        assert_eq!(status, StatusCode::OK, "accept={accept}");
        assert!(ctype.starts_with(XML), "accept={accept} ctype={ctype}");
        let parsed = parse(QueryResultsFormat::Xml, &body).unwrap();
        assert_eq!(parsed, reference, "accept={accept}");
    }
}

#[tokio::test]
async fn truncated_real_response_is_rejected() {
    let cfg = config(0);
    let query = "SELECT ?name WHERE { ?s <http://ex/name> ?name }";
    let (_, body) = xml_and_json(&cfg, query).await;
    let text = String::from_utf8(body).unwrap();
    let cut = text.rfind("<result").expect("result element") + "<result".len() + 3;
    assert!(cut < text.len());
    let truncated = &text.as_bytes()[..cut];
    let parsed = parse(QueryResultsFormat::Xml, truncated);
    assert!(parsed.is_err(), "truncated XML parsed: {parsed:?}");
}
