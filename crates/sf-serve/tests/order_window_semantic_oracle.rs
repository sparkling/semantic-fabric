//! Independent materialized-oracle qualification for ADR-0054's admitted
//! finite root variable-key ORDER window.
//!
//! Expected order comes only from `spareval` over an independently constructed
//! RDF dataset. This test never imports the engine comparator or `sorted_indices`.
//! Cases where SPARQL/XSD leaves relative order undefined are deliberately
//! checked only for result-bag preservation and repeatability, never sequence
//! equality against the evaluator.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use oxrdf::{Dataset, Term};
use sf_serve::{router, ServeConfig};
use spareval::{QueryEvaluator, QueryResults};
use spargebra::SparqlParser;
use tower::ServiceExt;

#[path = "order_window_semantic_oracle/fixture.rs"]
mod fixture_data;

use fixture_data::{fixture, Fixture, EX};

fn scalar_query(predicate: &str, order: &str, slice: &str) -> String {
    format!("SELECT ?s ?value WHERE {{ ?s <{EX}{predicate}> ?value }} ORDER BY {order} {slice}")
}

fn multiple_query(order: &str, slice: &str) -> String {
    format!(
        "SELECT ?s ?first ?second WHERE {{ ?s <{EX}first> ?first ; <{EX}second> ?second }} \
         ORDER BY {order} {slice}"
    )
}

async fn engine_subjects(config: &Arc<ServeConfig>, query: &str) -> Vec<String> {
    let request = Request::builder()
        .method("POST")
        .uri("/sparql")
        .header(header::CONTENT_TYPE, "application/sparql-query")
        .header(header::ACCEPT, "application/sparql-results+json")
        .body(Body::from(query.to_owned()))
        .expect("build query request");
    let response = router(config.clone())
        .oneshot(request)
        .await
        .expect("serve query");
    assert_eq!(response.status(), StatusCode::OK, "query={query}");
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect query response")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).expect("parse result JSON");
    json["results"]["bindings"]
        .as_array()
        .expect("bindings array")
        .iter()
        .map(|row| {
            row["s"]["value"]
                .as_str()
                .expect("bound subject")
                .to_owned()
        })
        .collect()
}

fn oracle_subjects(graph: &Dataset, query: &str) -> Vec<String> {
    let query = SparqlParser::new()
        .parse_query(query)
        .expect("spareval query parse");
    let QueryResults::Solutions(rows) = QueryEvaluator::new()
        .prepare(&query)
        .execute(graph)
        .expect("spareval query evaluation")
    else {
        panic!("expected SELECT oracle answer")
    };
    rows.map(|row| {
        let row = row.expect("spareval solution");
        match row.get("s").expect("oracle subject binding") {
            Term::NamedNode(subject) => subject.as_str().to_owned(),
            other => panic!("expected named-node subject, got {other}"),
        }
    })
    .collect()
}

async fn assert_defined_sequence(fixture: &Fixture, query: &str) {
    assert_eq!(
        engine_subjects(&fixture.config, query).await,
        oracle_subjects(&fixture.graph, query),
        "defined ORDER sequence diverged for {query}"
    );
}

async fn assert_undefined_sequence_is_not_oracle_claimed(fixture: &Fixture, query: &str) {
    let first = engine_subjects(&fixture.config, query).await;
    let second = engine_subjects(&fixture.config, query).await;
    assert_eq!(first, second, "engine extension must remain deterministic");
    let mut engine_bag = first;
    let mut oracle_bag = oracle_subjects(&fixture.graph, query);
    engine_bag.sort_unstable();
    oracle_bag.sort_unstable();
    assert_eq!(
        engine_bag, oracle_bag,
        "undefined ORDER must preserve the result bag"
    );
}

fn expected_subjects(table: &str, ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| format!("{EX}{table}/{id}")).collect()
}

#[tokio::test]
async fn mixed_numeric_defined_orders_precision_subnormals_and_infinities_match_spareval() {
    let fixture = fixture();
    for predicate in ["numeric-mixed", "numeric-precision", "numeric-infinity"] {
        assert_defined_sequence(&fixture, &scalar_query(predicate, "?value", "LIMIT 100")).await;
        assert_defined_sequence(
            &fixture,
            &scalar_query(predicate, "DESC(?value)", "LIMIT 100"),
        )
        .await;
    }
    assert_defined_sequence(
        &fixture,
        &scalar_query("numeric-mixed", "?value", "OFFSET 3 LIMIT 5"),
    )
    .await;
}

#[tokio::test]
async fn rounding_signed_zero_nan_and_stable_ties_are_covered_without_false_oracle_authority() {
    let fixture = fixture();
    for (predicate, order) in [
        ("numeric-float-ties", "?value ?s"),
        ("numeric-float-ties", "DESC(?value) ?s"),
        ("numeric-double-ties", "?value ?s"),
        ("numeric-double-ties", "DESC(?value) ?s"),
    ] {
        assert_defined_sequence(&fixture, &scalar_query(predicate, order, "LIMIT 100")).await;
    }
    let float_asc = scalar_query("numeric-float-ties", "?value", "LIMIT 100");
    assert_undefined_sequence_is_not_oracle_claimed(&fixture, &float_asc).await;
    assert_eq!(
        engine_subjects(&fixture.config, &float_asc).await,
        expected_subjects("numeric_float_ties", &["01", "02", "03", "04", "05"])
    );
    let float_desc = scalar_query("numeric-float-ties", "DESC(?value)", "LIMIT 100");
    assert_undefined_sequence_is_not_oracle_claimed(&fixture, &float_desc).await;
    assert_eq!(
        engine_subjects(&fixture.config, &float_desc).await,
        expected_subjects("numeric_float_ties", &["05", "03", "04", "01", "02"])
    );
    let double_asc = scalar_query("numeric-double-ties", "?value", "LIMIT 100");
    assert_undefined_sequence_is_not_oracle_claimed(&fixture, &double_asc).await;
    assert_eq!(
        engine_subjects(&fixture.config, &double_asc).await,
        expected_subjects("numeric_double_ties", &["01", "02", "03"])
    );
    let double_desc = scalar_query("numeric-double-ties", "DESC(?value)", "LIMIT 100");
    assert_undefined_sequence_is_not_oracle_claimed(&fixture, &double_desc).await;
    assert_eq!(
        engine_subjects(&fixture.config, &double_desc).await,
        expected_subjects("numeric_double_ties", &["03", "01", "02"])
    );
    for order in ["?value", "DESC(?value)"] {
        assert_undefined_sequence_is_not_oracle_claimed(
            &fixture,
            &scalar_query("numeric-nan", order, "LIMIT 100"),
        )
        .await;
    }
}

#[tokio::test]
async fn boolean_aliases_keep_defined_value_groups_and_source_stable_ties() {
    let fixture = fixture();
    assert_defined_sequence(
        &fixture,
        &scalar_query("boolean-alias", "?value ?s", "LIMIT 100"),
    )
    .await;
    assert_defined_sequence(
        &fixture,
        &scalar_query("boolean-alias", "DESC(?value) ?s", "LIMIT 100"),
    )
    .await;
    let asc = scalar_query("boolean-alias", "?value", "LIMIT 100");
    assert_undefined_sequence_is_not_oracle_claimed(&fixture, &asc).await;
    assert_eq!(
        engine_subjects(&fixture.config, &asc).await,
        expected_subjects("boolean_aliases", &["01", "02", "03", "04"])
    );
    let desc = scalar_query("boolean-alias", "DESC(?value)", "LIMIT 100");
    assert_undefined_sequence_is_not_oracle_claimed(&fixture, &desc).await;
    assert_eq!(
        engine_subjects(&fixture.config, &desc).await,
        expected_subjects("boolean_aliases", &["03", "04", "01", "02"])
    );
}

#[tokio::test]
async fn zoned_and_local_calendar_domains_match_where_comparison_is_defined() {
    let fixture = fixture();
    for predicate in [
        "datetime-zoned",
        "datetime-local",
        "date-zoned",
        "date-local",
        "time-zoned",
        "time-local",
    ] {
        assert_defined_sequence(&fixture, &scalar_query(predicate, "?value", "LIMIT 100")).await;
        assert_defined_sequence(
            &fixture,
            &scalar_query(predicate, "DESC(?value)", "LIMIT 100"),
        )
        .await;
    }
    for order in ["?value", "DESC(?value)"] {
        assert_undefined_sequence_is_not_oracle_claimed(
            &fixture,
            &scalar_query("datetime-partial", order, "LIMIT 100"),
        )
        .await;
    }
}

#[tokio::test]
async fn duration_subtypes_match_defined_orders_but_partial_duration_order_is_not_claimed() {
    let fixture = fixture();
    for predicate in [
        "duration-day-time",
        "duration-year-month",
        "duration-defined",
    ] {
        assert_defined_sequence(&fixture, &scalar_query(predicate, "?value", "LIMIT 100")).await;
        assert_defined_sequence(
            &fixture,
            &scalar_query(predicate, "DESC(?value)", "LIMIT 100"),
        )
        .await;
    }
    for order in ["?value", "DESC(?value)"] {
        assert_undefined_sequence_is_not_oracle_claimed(
            &fixture,
            &scalar_query("duration-partial", order, "LIMIT 100"),
        )
        .await;
    }
}

#[tokio::test]
async fn unbound_multiple_keys_stable_ties_and_offset_limit_match_their_exact_boundaries() {
    let fixture = fixture();
    let optional_asc = format!(
        "SELECT ?s ?value WHERE {{ ?s <{EX}row> ?marker OPTIONAL {{ ?s <{EX}optional> ?value }} }} \
         ORDER BY ?value ?s LIMIT 100"
    );
    let optional_desc = format!(
        "SELECT ?s ?value WHERE {{ ?s <{EX}row> ?marker OPTIONAL {{ ?s <{EX}optional> ?value }} }} \
         ORDER BY DESC(?value) ?s LIMIT 100"
    );
    assert_defined_sequence(&fixture, &optional_asc).await;
    assert_defined_sequence(&fixture, &optional_desc).await;

    assert_defined_sequence(
        &fixture,
        &multiple_query("?first DESC(?second) ?s", "LIMIT 100"),
    )
    .await;
    assert_defined_sequence(
        &fixture,
        &multiple_query("?first DESC(?second) ?s", "OFFSET 1 LIMIT 3"),
    )
    .await;
    assert_defined_sequence(
        &fixture,
        &multiple_query("DESC(?first) ?second ?s", "LIMIT 100"),
    )
    .await;

    let tied = multiple_query("?first DESC(?second)", "LIMIT 100");
    assert_undefined_sequence_is_not_oracle_claimed(&fixture, &tied).await;
    assert_eq!(
        engine_subjects(&fixture.config, &tied).await,
        expected_subjects("multiple_keys", &["01", "02", "03", "04", "05"])
    );
    for (slice, expected_id) in [("OFFSET 3 LIMIT 1", "04"), ("OFFSET 4 LIMIT 1", "05")] {
        let query = multiple_query("?first DESC(?second)", slice);
        assert_eq!(
            engine_subjects(&fixture.config, &query).await,
            expected_subjects("multiple_keys", &[expected_id]),
            "stable source tie must cross the slice boundary in arrival order"
        );
    }
}

#[tokio::test]
async fn heterogeneous_literal_domain_extension_is_deterministic_but_not_semantic_authority() {
    let fixture = fixture();
    for order in ["?value", "DESC(?value)"] {
        assert_undefined_sequence_is_not_oracle_claimed(
            &fixture,
            &scalar_query("cross-domain", order, "LIMIT 100"),
        )
        .await;
    }
}
