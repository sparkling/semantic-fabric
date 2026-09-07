use std::fmt::Write as _;
use std::sync::Arc;

use axum::body::{to_bytes, Body, Bytes};
use axum::http::{header, HeaderMap, Method, Request};
use sf_serve::{router, Backend, IntrospectedSource, SemanticOntology, ServeConfig};
use tower::ServiceExt;

const CREATE_SQL: &str = r#"
CREATE TABLE "People" (
    "id" INTEGER PRIMARY KEY,
    "name" TEXT NOT NULL,
    "age" INTEGER NOT NULL,
    "email" TEXT
);
INSERT INTO "People" VALUES
    (1, 'Alice', 30, 'alice@example.test'),
    (2, 'Bob', 25, NULL);
"#;

const MAPPING_TTL: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#People> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "People" ] ;
  rr:subjectMap [ rr:template "http://ex/person/{id}" ; rr:class ex:Person ] ;
  rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] ;
  rr:predicateObjectMap [ rr:predicate ex:age ; rr:objectMap [ rr:column "age" ] ] ;
  rr:predicateObjectMap [ rr:predicate ex:email ; rr:objectMap [ rr:column "email" ] ] .
"#;

const ONTOLOGY_TTL: &str = r#"
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex: <http://ex/> .
ex:Person a owl:Class .
ex:name a rdf:Property .
ex:age a rdf:Property .
ex:email a rdf:Property .
"#;

pub const SELECT_NAMES: &str = "SELECT ?name WHERE { ?s <http://ex/name> ?name }";
pub const ASK_TRUE: &str = "ASK { ?s <http://ex/name> \"Alice\" }";
pub const CONSTRUCT_LABELS: &str =
    "CONSTRUCT { ?s <http://ex/label> ?name } WHERE { ?s <http://ex/name> ?name }";

pub struct ResponseSnapshot {
    pub status: u16,
    pub media_type: String,
    pub allow: Option<String>,
    pub correlation_id: Option<String>,
    pub content_length: Option<String>,
    pub body: Bytes,
}

pub fn config(max_query_len: Option<usize>) -> Arc<ServeConfig> {
    let connection = rusqlite::Connection::open_in_memory().expect("open isolated SQLite fixture");
    connection
        .execute_batch(CREATE_SQL)
        .expect("load isolated SQLite fixture");
    let backend = Backend::sqlite(connection);
    let source = IntrospectedSource::observe_sqlite(backend).expect("observe SQLite fixture");
    let ontology = SemanticOntology::from_turtle(ONTOLOGY_TTL).expect("parse fixture ontology");
    let mut config = ServeConfig::from_authored_r2rml(source, MAPPING_TTL, ontology)
        .expect("admit fixture mapping and ontology");
    if let Some(maximum) = max_query_len {
        config
            .set_max_query_len(maximum)
            .expect("set representable query bound");
    }
    Arc::new(config)
}

pub fn request(
    method: Method,
    uri: impl AsRef<str>,
    content_type: Option<&str>,
    accept: Option<&str>,
    body: Body,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri.as_ref());
    if let Some(content_type) = content_type {
        builder = builder.header(header::CONTENT_TYPE, content_type);
    }
    if let Some(accept) = accept {
        builder = builder.header(header::ACCEPT, accept);
    }
    builder.body(body).expect("build bounded fixture request")
}

pub fn raw_post(query: &str, accept: &str) -> Request<Body> {
    request(
        Method::POST,
        "/sparql",
        Some("application/sparql-query"),
        Some(accept),
        Body::from(query.to_owned()),
    )
}

pub fn form_post(query: &str, accept: &str) -> Request<Body> {
    let body = encoded_pairs(&[("query", query)]);
    request(
        Method::POST,
        "/sparql",
        Some("application/x-www-form-urlencoded"),
        Some(accept),
        Body::from(body),
    )
}

pub fn get_query(query: &str, accept: &str) -> Request<Body> {
    let query = encoded_pairs(&[("query", query)]);
    request(
        Method::GET,
        format!("/sparql?{query}"),
        None,
        Some(accept),
        Body::empty(),
    )
}

pub fn encoded_pairs(pairs: &[(&str, &str)]) -> String {
    let mut output = String::new();
    for (index, (key, value)) in pairs.iter().enumerate() {
        if index > 0 {
            output.push('&');
        }
        encode_component(key, &mut output);
        output.push('=');
        encode_component(value, &mut output);
    }
    output
}

fn encode_component(value: &str, output: &mut String) {
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(char::from(byte));
        } else if byte == b' ' {
            output.push('+');
        } else {
            write!(output, "%{byte:02X}").expect("String writes cannot fail");
        }
    }
}

pub async fn send(
    config: Arc<ServeConfig>,
    request: Request<Body>,
) -> Result<ResponseSnapshot, String> {
    let response = router(config)
        .oneshot(request)
        .await
        .map_err(|_| "public router rejected the in-process request".to_owned())?;
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .map_err(|_| "public response body failed or exceeded 64 KiB".to_owned())?;
    Ok(ResponseSnapshot {
        status,
        media_type: media_type(&headers),
        allow: header_value(&headers, header::ALLOW),
        correlation_id: header_value(
            &headers,
            header::HeaderName::from_static("x-correlation-id"),
        ),
        content_length: header_value(&headers, header::CONTENT_LENGTH),
        body,
    })
}

fn media_type(headers: &HeaderMap) -> String {
    header_value(headers, header::CONTENT_TYPE)
        .and_then(|value| {
            value
                .split(';')
                .next()
                .map(str::trim)
                .map(str::to_ascii_lowercase)
        })
        .unwrap_or_default()
}

fn header_value(headers: &HeaderMap, name: header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}
