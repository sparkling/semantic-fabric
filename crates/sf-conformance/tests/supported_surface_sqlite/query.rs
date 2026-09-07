use std::collections::BTreeSet;

use oxttl::TurtleParser;
use sf_conformance::supported_surface::{Case, Manifest, Surface};
use sparesults::{QueryResultsFormat, QueryResultsParser, SliceQueryResultsParserOutput};

use super::fixture::{self, ResponseSnapshot};
use super::observation::{finish_replay, Observation};

pub async fn replay(manifest: &Manifest) -> Result<(), String> {
    if manifest.surface != Surface::SparqlQuery {
        return Err("Query replay received a non-Query manifest".to_owned());
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
        "query-ask-false" => ask("ASK { ?s <http://ex/name> \"Nobody\" }", false).await,
        "query-ask-true" => ask(fixture::ASK_TRUE, true).await,
        "query-construct-basic" => construct().await,
        "query-describe-constant-one-hop" => {
            describe("DESCRIBE <http://ex/person/1>", person_one_graph()).await
        }
        "query-describe-multiple-targets-unsupported" => {
            problem("DESCRIBE <http://ex/person/1> <http://ex/person/2>").await
        }
        "query-describe-unbound-target-unsupported" => {
            problem("DESCRIBE ?s WHERE { ?x <http://ex/name> \"Alice\" }").await
        }
        "query-describe-variable-one-hop" => {
            describe(
                "DESCRIBE ?s WHERE { ?s <http://ex/name> \"Alice\" }",
                person_one_graph(),
            )
            .await
        }
        "query-select-bgp" => {
            select(
                fixture::SELECT_NAMES,
                &["name"],
                &[&["\"Alice\""], &["\"Bob\""]],
            )
            .await
        }
        "query-select-filter" => {
            select(
                "SELECT ?name WHERE { ?s <http://ex/age> ?age ; <http://ex/name> ?name . FILTER(?age > 25) }",
                &["name"],
                &[&["\"Alice\""]],
            )
            .await
        }
        "query-select-optional-unbound" => {
            select(
                "SELECT ?name ?email WHERE { ?s <http://ex/name> ?name OPTIONAL { ?s <http://ex/email> ?email } } ORDER BY ?name LIMIT 2",
                &["name", "email"],
                &[
                    &["\"Alice\"", "\"alice@example.test\""],
                    &["\"Bob\"", "UNBOUND"],
                ],
            )
            .await
        }
        "query-select-order-window" => {
            select(
                "SELECT ?name WHERE { ?s <http://ex/name> ?name } ORDER BY ?name LIMIT 1 OFFSET 1",
                &["name"],
                &[&["\"Bob\""]],
            )
            .await
        }
        "query-service-unsupported" => {
            problem("SELECT * WHERE { SERVICE <http://example.invalid/sparql> { ?s ?p ?o } }")
                .await
        }
        _ => Err("Query manifest contains an unknown scenario".to_owned()),
    }
}

async fn ask(query: &str, expected: bool) -> Result<Observation, String> {
    let response = send_query(query, "application/sparql-results+json").await?;
    let document: serde_json::Value = serde_json::from_slice(&response.body)
        .map_err(|_| "ASK returned malformed JSON".to_owned())?;
    let observed = document["boolean"]
        .as_bool()
        .ok_or_else(|| "ASK JSON omitted its boolean".to_owned())?;
    if observed != expected {
        return Err("ASK returned the wrong boolean".to_owned());
    }
    Observation::supported(&response, "ask-boolean", &format!("ask:{observed}"))
}

async fn select(
    query: &str,
    expected_variables: &[&str],
    expected_rows: &[&[&str]],
) -> Result<Observation, String> {
    let response = send_query(query, "application/sparql-results+json").await?;
    let normalized = normalize_select_results(
        &response,
        QueryResultsFormat::Json,
        expected_variables,
        expected_rows,
    )?;
    Observation::supported(&response, "select-bindings", &normalized)
}

async fn construct() -> Result<Observation, String> {
    let response = send_query(fixture::CONSTRUCT_LABELS, "text/turtle").await?;
    let expected = BTreeSet::from([
        "<http://ex/person/1> <http://ex/label> \"Alice\"".to_owned(),
        "<http://ex/person/2> <http://ex/label> \"Bob\"".to_owned(),
    ]);
    let normalized = normalize_turtle_graph(&response, &expected)?;
    Observation::supported(&response, "construct-graph", &normalized)
}

async fn describe(query: &str, expected: BTreeSet<String>) -> Result<Observation, String> {
    let response = send_query(query, "text/turtle").await?;
    let normalized = normalize_turtle_graph(&response, &expected)?;
    Observation::supported(&response, "describe-graph", &normalized)
}

async fn problem(query: &str) -> Result<Observation, String> {
    let response = send_query(query, "application/sparql-results+json").await?;
    Observation::problem(&response)
}

async fn send_query(query: &str, accept: &str) -> Result<ResponseSnapshot, String> {
    fixture::send(fixture::config(None), fixture::raw_post(query, accept)).await
}

pub fn normalize_select_results(
    response: &ResponseSnapshot,
    format: QueryResultsFormat,
    expected_variables: &[&str],
    expected_rows: &[&[&str]],
) -> Result<String, String> {
    let parsed = QueryResultsParser::from_format(format)
        .for_slice(&response.body)
        .map_err(|_| "SELECT returned malformed query results".to_owned())?;
    let SliceQueryResultsParserOutput::Solutions(solutions) = parsed else {
        return Err("SELECT result unexpectedly contains a boolean".to_owned());
    };
    let variables = solutions.variables().to_vec();
    let variable_names = variables
        .iter()
        .map(|variable| variable.as_str().to_owned())
        .collect::<Vec<_>>();
    let expected_variables = expected_variables
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    if variable_names != expected_variables {
        return Err("SELECT returned the wrong variable sequence".to_owned());
    }
    let mut rows = solutions
        .map(|solution| {
            let solution = solution.map_err(|_| "SELECT returned a malformed row".to_owned())?;
            Ok(variables
                .iter()
                .map(|variable| {
                    solution
                        .get(variable)
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "UNBOUND".to_owned())
                })
                .collect::<Vec<_>>())
        })
        .collect::<Result<Vec<_>, String>>()?;
    rows.sort();
    let mut expected = expected_rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    expected.sort();
    if rows != expected {
        return Err("SELECT returned the wrong binding bag".to_owned());
    }
    Ok(format!("vars={variable_names:?};rows={rows:?}"))
}

pub fn normalize_turtle_graph(
    response: &ResponseSnapshot,
    expected: &BTreeSet<String>,
) -> Result<String, String> {
    let triples = TurtleParser::new()
        .for_slice(&response.body)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "RDF response is not valid Turtle".to_owned())?;
    let actual = triples
        .into_iter()
        .map(|triple| triple.to_string())
        .collect::<BTreeSet<_>>();
    if &actual != expected {
        return Err("RDF response graph differs from the sealed fixture graph".to_owned());
    }
    Ok(actual.into_iter().collect::<Vec<_>>().join("\n"))
}

fn person_one_graph() -> BTreeSet<String> {
    BTreeSet::from([
        "<http://ex/person/1> <http://ex/age> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer>"
            .to_owned(),
        "<http://ex/person/1> <http://ex/email> \"alice@example.test\"".to_owned(),
        "<http://ex/person/1> <http://ex/name> \"Alice\"".to_owned(),
        "<http://ex/person/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Person>"
            .to_owned(),
    ])
}
