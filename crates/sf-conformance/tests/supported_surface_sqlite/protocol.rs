use std::collections::BTreeSet;

use axum::body::Body;
use axum::http::Method;
use oxjsonld::JsonLdParser;
use oxrdf::{GraphName, Triple};
use oxttl::{NTriplesParser, TurtleParser};
use sf_conformance::supported_surface::{Case, Manifest, Surface};
use sparesults::QueryResultsFormat;

use super::fixture::{self, ResponseSnapshot};
use super::observation::{finish_replay, Observation};
use super::query::normalize_select_results;

pub async fn replay(manifest: &Manifest) -> Result<(), String> {
    if manifest.surface != Surface::SparqlProtocol {
        return Err("Protocol replay received a non-Protocol manifest".to_owned());
    }
    let mut failures = Vec::new();
    for case in &manifest.cases {
        match execute(case).await {
            Ok(observation) => failures.extend(observation.mismatch(case)),
            Err(error) => failures.push(format!("{}: {error}", case.id)),
        }
    }
    finish_replay(failures)
}

async fn execute(case: &Case) -> Result<Observation, String> {
    match case.scenario.as_str() {
        "protocol-construct-default-turtle" => construct(None, GraphFormat::Turtle).await,
        "protocol-construct-jsonld" => {
            construct(Some("application/ld+json"), GraphFormat::JsonLd).await
        }
        "protocol-construct-ntriples" => {
            construct(Some("application/n-triples"), GraphFormat::NTriples).await
        }
        "protocol-get-query" => {
            transport(fixture::get_query(
                fixture::ASK_TRUE,
                "application/sparql-results+json",
            ))
            .await
        }
        "protocol-post-form-query" => {
            transport(fixture::form_post(
                fixture::ASK_TRUE,
                "application/sparql-results+json",
            ))
            .await
        }
        "protocol-post-raw-query" => {
            transport(fixture::raw_post(
                fixture::ASK_TRUE,
                "application/sparql-results+json",
            ))
            .await
        }
        "protocol-reject-get-dataset" => {
            problem(get_with_pairs(&[
                ("query", fixture::ASK_TRUE),
                ("default-graph-uri", "http://example.test/graph"),
            ]))
            .await
        }
        "protocol-reject-get-duplicate-query" => {
            problem(get_with_pairs(&[
                ("query", fixture::ASK_TRUE),
                ("query", fixture::ASK_TRUE),
            ]))
            .await
        }
        "protocol-reject-get-missing-query" => problem(get_with_pairs(&[("other", "value")])).await,
        "protocol-reject-post-content-parameters" => {
            problem(fixture::request(
                Method::POST,
                "/sparql",
                Some("application/sparql-query; version=1.2"),
                None,
                Body::from(fixture::ASK_TRUE),
            ))
            .await
        }
        "protocol-reject-post-form-duplicate-query" => {
            problem(form_with_pairs(&[
                ("query", fixture::ASK_TRUE),
                ("query", fixture::ASK_TRUE),
            ]))
            .await
        }
        "protocol-reject-post-form-extra-field" => {
            problem(form_with_pairs(&[
                ("query", fixture::ASK_TRUE),
                ("version", "1.2"),
            ]))
            .await
        }
        "protocol-reject-post-missing-content-type" => {
            problem(fixture::request(
                Method::POST,
                "/sparql",
                None,
                None,
                Body::from(fixture::ASK_TRUE),
            ))
            .await
        }
        "protocol-reject-post-uri-parameter" => {
            problem(fixture::request(
                Method::POST,
                "/sparql?version=1.2",
                Some("application/sparql-query"),
                None,
                Body::from(fixture::ASK_TRUE),
            ))
            .await
        }
        "protocol-reject-put-method" => reject_put().await,
        "protocol-reject-raw-non-utf8" => {
            problem(fixture::request(
                Method::POST,
                "/sparql",
                Some("application/sparql-query"),
                None,
                Body::from(vec![0xff]),
            ))
            .await
        }
        "protocol-reject-raw-oversized" => reject_oversized().await,
        "protocol-reject-service-description-accept" => {
            problem(fixture::request(
                Method::GET,
                "/sparql",
                None,
                Some("application/ld+json"),
                Body::empty(),
            ))
            .await
        }
        "protocol-select-csv" => select(Some("text/csv"), ResultFormat::Csv).await,
        "protocol-select-default-json" => select(None, ResultFormat::Json).await,
        "protocol-select-json" => {
            select(Some("application/sparql-results+json"), ResultFormat::Json).await
        }
        "protocol-select-tsv" => select(Some("text/tab-separated-values"), ResultFormat::Tsv).await,
        "protocol-select-xml" => {
            select(Some("application/sparql-results+xml"), ResultFormat::Xml).await
        }
        "protocol-service-description-get" => service_description(false).await,
        "protocol-service-description-head" => service_description(true).await,
        _ => Err("Protocol manifest contains an unknown scenario".to_owned()),
    }
}

async fn transport(request: axum::http::Request<Body>) -> Result<Observation, String> {
    let response = fixture::send(fixture::config(None), request).await?;
    let document: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|_| "transport scenario returned malformed JSON".to_owned())?;
    let observed = document["boolean"]
        .as_bool()
        .ok_or_else(|| "transport scenario omitted its ASK boolean".to_owned())?;
    if !observed {
        return Err("transport scenario did not preserve its ASK result".to_owned());
    }
    Observation::supported(&response, "transport-query", &format!("ask:{observed}"))
}

#[derive(Clone, Copy)]
enum ResultFormat {
    Json,
    Xml,
    Csv,
    Tsv,
}

async fn select(accept: Option<&str>, format: ResultFormat) -> Result<Observation, String> {
    let request = fixture::request(
        Method::POST,
        "/sparql",
        Some("application/sparql-query"),
        accept,
        Body::from(fixture::SELECT_NAMES),
    );
    let response = fixture::send(fixture::config(None), request).await?;
    let normalized = match format {
        ResultFormat::Json => normalize_select_results(
            &response,
            QueryResultsFormat::Json,
            &["name"],
            &[&["\"Alice\""], &["\"Bob\""]],
        )?,
        ResultFormat::Xml => normalize_select_results(
            &response,
            QueryResultsFormat::Xml,
            &["name"],
            &[&["\"Alice\""], &["\"Bob\""]],
        )?,
        ResultFormat::Csv => normalize_csv_results(&response)?,
        ResultFormat::Tsv => normalize_select_results(
            &response,
            QueryResultsFormat::Tsv,
            &["name"],
            &[&["\"Alice\""], &["\"Bob\""]],
        )?,
    };
    Observation::supported(&response, "representation-query-results", &normalized)
}

fn normalize_csv_results(response: &ResponseSnapshot) -> Result<String, String> {
    let body = std::str::from_utf8(&response.body)
        .map_err(|_| "query result representation is not UTF-8".to_owned())?;
    let mut lines = body.lines().map(|line| line.trim_end_matches('\r'));
    let header = lines
        .next()
        .ok_or_else(|| "CSV query result omitted its header".to_owned())?;
    if header != "name" {
        return Err("CSV query result has the wrong header".to_owned());
    }
    let mut rows = lines.map(|line| vec![line.to_owned()]).collect::<Vec<_>>();
    rows.sort_unstable();
    if rows != [vec!["Alice".to_owned()], vec!["Bob".to_owned()]] {
        return Err("CSV query result has the wrong binding bag".to_owned());
    }
    Ok(format!("vars={:?};rows={rows:?}", [header]))
}

#[derive(Clone, Copy)]
enum GraphFormat {
    Turtle,
    NTriples,
    JsonLd,
}

async fn construct(accept: Option<&str>, format: GraphFormat) -> Result<Observation, String> {
    let request = fixture::request(
        Method::POST,
        "/sparql",
        Some("application/sparql-query"),
        accept,
        Body::from(fixture::CONSTRUCT_LABELS),
    );
    let response = fixture::send(fixture::config(None), request).await?;
    let normalized = match format {
        GraphFormat::Turtle => normalize_turtle_labels(&response)?,
        GraphFormat::NTriples => normalize_ntriples_labels(&response)?,
        GraphFormat::JsonLd => normalize_jsonld_labels(&response)?,
    };
    Observation::supported(&response, "representation-rdf-graph", &normalized)
}

fn expected_label_graph() -> BTreeSet<String> {
    BTreeSet::from([
        "<http://ex/person/1> <http://ex/label> \"Alice\"".to_owned(),
        "<http://ex/person/2> <http://ex/label> \"Bob\"".to_owned(),
    ])
}

fn normalize_turtle_labels(response: &ResponseSnapshot) -> Result<String, String> {
    let actual = TurtleParser::new()
        .for_slice(&response.body)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Turtle result is malformed".to_owned())?
        .into_iter()
        .map(|triple| triple.to_string())
        .collect::<BTreeSet<_>>();
    normalize_label_graph(actual)
}

fn normalize_ntriples_labels(response: &ResponseSnapshot) -> Result<String, String> {
    let actual = NTriplesParser::new()
        .for_slice(&response.body)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "N-Triples result is malformed".to_owned())?
        .into_iter()
        .map(|triple| triple.to_string())
        .collect::<BTreeSet<_>>();
    normalize_label_graph(actual)
}

fn normalize_label_graph(actual: BTreeSet<String>) -> Result<String, String> {
    if actual != expected_label_graph() {
        return Err("graph result differs from the sealed label graph".to_owned());
    }
    Ok(actual.into_iter().collect::<Vec<_>>().join("\n"))
}

fn normalize_jsonld_labels(response: &ResponseSnapshot) -> Result<String, String> {
    let quads = JsonLdParser::new()
        .for_slice(&response.body)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "JSON-LD result is malformed".to_owned())?;
    if quads
        .iter()
        .any(|quad| quad.graph_name != GraphName::DefaultGraph)
    {
        return Err("JSON-LD result unexpectedly contains a named graph".to_owned());
    }
    let actual = quads
        .into_iter()
        .map(Triple::from)
        .map(|triple| triple.to_string())
        .collect::<BTreeSet<_>>();
    normalize_label_graph(actual)
}

async fn service_description(head: bool) -> Result<Observation, String> {
    let method = if head { Method::HEAD } else { Method::GET };
    let response = fixture::send(
        fixture::config(None),
        fixture::request(method, "/sparql", None, Some("text/turtle"), Body::empty()),
    )
    .await?;
    let normalized = if head {
        if !response.body.is_empty() || response.content_length.is_none() {
            return Err("HEAD discovery did not preserve GET metadata without a body".to_owned());
        }
        format!(
            "service-description-head:length={}",
            response.content_length.as_deref().unwrap_or_default()
        )
    } else {
        normalize_service_description(&response)?
    };
    Observation::supported(&response, "service-description", &normalized)
}

fn normalize_service_description(response: &ResponseSnapshot) -> Result<String, String> {
    let actual = TurtleParser::new()
        .with_base_iri("http://example.test/sparql")
        .map_err(|_| "fixed service-description base IRI is invalid".to_owned())?
        .for_slice(&response.body)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "service description is malformed Turtle".to_owned())?
        .into_iter()
        .map(|triple| triple.to_string())
        .collect::<BTreeSet<_>>();
    let text = std::str::from_utf8(&response.body)
        .map_err(|_| "service description is not UTF-8".to_owned())?;
    if actual.len() != 19
        || !text.contains("bounded-read-query-v1")
        || !text.contains("describe-one-target-one-hop-query-v1")
        || text.contains("SPARQL11Query")
        || text.contains("BasicFederatedQuery")
    {
        return Err("service description differs from its bounded subset".to_owned());
    }
    Ok(actual.into_iter().collect::<Vec<_>>().join("\n"))
}

async fn problem(request: axum::http::Request<Body>) -> Result<Observation, String> {
    let response = fixture::send(fixture::config(None), request).await?;
    Observation::problem(&response)
}

fn get_with_pairs(pairs: &[(&str, &str)]) -> axum::http::Request<Body> {
    fixture::request(
        Method::GET,
        format!("/sparql?{}", fixture::encoded_pairs(pairs)),
        None,
        None,
        Body::empty(),
    )
}

fn form_with_pairs(pairs: &[(&str, &str)]) -> axum::http::Request<Body> {
    fixture::request(
        Method::POST,
        "/sparql",
        Some("application/x-www-form-urlencoded"),
        None,
        Body::from(fixture::encoded_pairs(pairs)),
    )
}

async fn reject_put() -> Result<Observation, String> {
    let response = fixture::send(
        fixture::config(None),
        fixture::request(Method::PUT, "/sparql", None, None, Body::empty()),
    )
    .await?;
    if response.allow.as_deref() != Some("GET,HEAD,POST") {
        return Err("method rejection omitted the exact Allow header".to_owned());
    }
    Observation::problem(&response)
}

async fn reject_oversized() -> Result<Observation, String> {
    let oversized = format!("{} ", fixture::ASK_TRUE);
    let response = fixture::send(
        fixture::config(Some(fixture::ASK_TRUE.len())),
        fixture::request(
            Method::POST,
            "/sparql",
            Some("application/sparql-query"),
            None,
            Body::from(oversized),
        ),
    )
    .await?;
    Observation::problem(&response)
}
