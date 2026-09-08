use super::*;
use oxrdf::{GraphName, Term, Triple};
use std::collections::HashSet;

const QUERY: &str = "CONSTRUCT { ?s <http://example.test/name> ?name } WHERE { ?s <http://example.test/name> ?name }";

fn dataset(records: &[Value]) -> Vec<oxrdf::Quad> {
    let fragments: String = records
        .iter()
        .filter_map(|row| row["dataset"].as_str())
        .collect();
    // All fragments form ONE RDF dataset: parsing each independently would
    // incorrectly remint shared blank nodes.
    oxttl::NQuadsParser::new()
        .for_slice(fragments.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn assert_reified_products(quads: &[oxrdf::Quad]) -> HashSet<Triple> {
    let product: HashSet<_> = quads
        .iter()
        .filter(|quad| quad.graph_name == GraphName::DefaultGraph)
        .map(|quad| {
            Triple::new(
                quad.subject.clone(),
                quad.predicate.clone(),
                quad.object.clone(),
            )
        })
        .collect();
    let reified: HashSet<_> = quads
        .iter()
        .filter(|quad| {
            quad.predicate.as_str() == "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies"
        })
        .map(|quad| {
            assert_ne!(quad.graph_name, GraphName::DefaultGraph);
            let Term::Triple(triple) = &quad.object else {
                panic!("native triple term required");
            };
            assert!(product.contains(triple.as_ref()));
            triple.as_ref().clone()
        })
        .collect();
    assert_eq!(
        product, reified,
        "every and only emitted product triple is reified"
    );
    product
}

#[tokio::test]
async fn graph_lineage_has_native_reification_outside_the_unchanged_product_graph() {
    let cfg = Arc::new(config());
    let records = records(cfg.clone(), QUERY).await;
    assert_eq!(records[0]["profile"], "constant-mapping-source-graph-v1");
    assert_eq!(records[0]["blankNodeScope"], "response");
    let quads = dataset(&records);
    assert_eq!(assert_reified_products(&quads).len(), 3);
    let mut standard = request(QUERY);
    standard
        .headers_mut()
        .insert(header::ACCEPT, "application/n-triples".parse().unwrap());
    let body = router(cfg)
        .oneshot(standard)
        .await
        .unwrap()
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let ordinary: HashSet<_> = oxttl::NTriplesParser::new()
        .for_slice(&body)
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ordinary, assert_reified_products(&quads));
    assert_eq!(
        records.last().unwrap(),
        &serde_json::json!({"type":"complete", "solutions":3,"tripleOccurrences":3})
    );
    for quad in &quads {
        if quad
            .predicate
            .as_str()
            .starts_with("http://www.w3.org/ns/prov#")
        {
            assert_ne!(quad.graph_name, GraphName::DefaultGraph);
        }
    }
}

#[tokio::test]
async fn graph_lineage_skips_empty_and_invalid_template_outputs() {
    for query in [
        "CONSTRUCT {} WHERE { ?s <http://example.test/name> ?name }",
        "CONSTRUCT { ?missing <urn:p> ?name } WHERE { ?s <http://example.test/name> ?name }",
        "CONSTRUCT { ?name <urn:p> ?s } WHERE { ?s <http://example.test/name> ?name }",
        "CONSTRUCT { ?s <urn:p> ?name } WHERE { <http://example.test/person/999> <http://example.test/name> ?name }",
    ] {
        let records = records(Arc::new(config()), query).await;
        assert_eq!(records.len(), 2, "no manufactured activity: {query}");
        assert!(dataset(&records).is_empty());
        assert_eq!(records[1]["solutions"], 0);
        assert_eq!(records[1]["tripleOccurrences"], 0);
    }
}

#[tokio::test]
async fn graph_lineage_counts_occurrences_without_changing_rdf_set_semantics() {
    let records = records(
        Arc::new(config()),
        "CONSTRUCT {
        <urn:s> <urn:p> <urn:o> . <urn:s> <urn:p> <urn:o>
    } WHERE { ?s <http://example.test/name> ?name }",
    )
    .await;
    let quads = dataset(&records);
    assert_eq!(assert_reified_products(&quads).len(), 1);
    let emitted = quads
        .iter()
        .filter(|quad| quad.graph_name == GraphName::DefaultGraph)
        .count();
    assert_eq!(records.last().unwrap()["tripleOccurrences"], emitted);
}

#[tokio::test]
async fn graph_lineage_keeps_template_blank_nodes_shared_within_each_solution_and_fresh_between_them(
) {
    let records = records(
        Arc::new(config()),
        "CONSTRUCT {
        _:new <urn:source> ?s . _:new <urn:name> ?name
    } WHERE { ?s <http://example.test/name> ?name }",
    )
    .await;
    let product = assert_reified_products(&dataset(&records));
    assert_eq!(product.len(), 6);
    let subjects: HashSet<_> = product
        .iter()
        .map(|triple| triple.subject.clone())
        .collect();
    assert_eq!(subjects.len(), 3);
    assert!(subjects.iter().all(|subject| subject.is_blank_node()));
}

#[tokio::test]
async fn graph_lineage_preserves_nested_triple_terms_in_the_reified_object() {
    let records = records(
        Arc::new(config()),
        "CONSTRUCT {
        ?s <urn:claim> <<( ?s <http://example.test/name> ?name )>>
    } WHERE { ?s <http://example.test/name> ?name }",
    )
    .await;
    let product = assert_reified_products(&dataset(&records));
    assert_eq!(product.len(), 3);
    assert!(product
        .iter()
        .all(|triple| matches!(triple.object, Term::Triple(_))));
}

fn with_source(conn: rusqlite::Connection, mapping: &str) -> ServeConfig {
    let mut cfg = support::serve_config(Backend::sqlite(conn), mapping);
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    cfg
}

#[tokio::test]
async fn graph_lineage_preserves_a_mapped_blank_node_shared_across_records() {
    let mapping = MAPPING
        .replace(
            "rr:template \"http://example.test/person/{id}\"",
            "rr:template \"{name}\" ; rr:termType rr:BlankNode",
        )
        .replace("rr:column \"name\"", "rr:column \"id\"");
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE people(id INTEGER, name TEXT); INSERT INTO people VALUES(1,'shared'),(2,'shared'),(3,'other');").unwrap();
    let records = records(Arc::new(with_source(conn, &mapping)), QUERY).await;
    assert_eq!(records.last().unwrap()["solutions"], 3);
    let product = assert_reified_products(&dataset(&records));
    assert_eq!(product.len(), 3);
    let subjects: HashSet<_> = product.iter().map(|triple| &triple.subject).collect();
    assert_eq!(
        subjects.len(),
        2,
        "shared mapped node must not be reminted per record"
    );
    assert!(subjects.iter().all(|subject| subject.is_blank_node()));
}

#[tokio::test]
async fn graph_lineage_round_trips_rdf_and_json_escaping_and_language_direction() {
    let value = "a\"\\\n\r\t\u{1e}é🦀";
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE people(id INTEGER, name TEXT);")
        .unwrap();
    conn.execute("INSERT INTO people VALUES (1, ?1)", [value])
        .unwrap();
    let cfg = Arc::new(with_source(conn, MAPPING));
    let rows = records(cfg.clone(), QUERY).await;
    let product = assert_reified_products(&dataset(&rows));
    assert_eq!(product.len(), 1);
    assert_eq!(
        product.iter().next().unwrap().object,
        oxrdf::Literal::new_simple_literal(value).into()
    );
    let rows = records(
        cfg,
        "CONSTRUCT { ?s <urn:p> \"مرحبا\"@ar--rtl } WHERE { ?s <http://example.test/name> ?name }",
    )
    .await;
    let product = assert_reified_products(&dataset(&rows));
    let Term::Literal(literal) = &product.iter().next().unwrap().object else {
        panic!("literal expected");
    };
    assert_eq!(literal.language(), Some("ar"));
    assert_eq!(literal.direction(), Some(oxrdf::BaseDirection::Rtl));
}

#[tokio::test]
async fn graph_lineage_template_admission_rejects_before_source_work() {
    let template: String = (0..257)
        .map(|i| format!("?s <urn:p{i}> ?name . "))
        .collect();
    let query =
        format!("CONSTRUCT {{ {template} }} WHERE {{ ?s <http://example.test/name> ?name }}");
    let mut cfg = config();
    cfg.query_limits = sf_core::query_control::QueryLimits::new(10000, 0, 10000, 100000);
    assert_eq!(
        router(Arc::new(cfg))
            .oneshot(request(&query))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_IMPLEMENTED
    );
}

#[tokio::test]
async fn graph_lineage_charges_reification_escaping_and_completion_bytes() {
    let cfg = Arc::new(config());
    let size = router(cfg)
        .oneshot(request(QUERY))
        .await
        .unwrap()
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .len() as u64;
    for (limit, succeeds) in [(size, true), (size - 1, false), (32, false)] {
        let mut cfg = config();
        cfg.query_limits = sf_core::query_control::QueryLimits::new(10000, 10000, 10000, limit);
        let response = router(Arc::new(cfg)).oneshot(request(QUERY)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result = response.into_body().collect().await;
        assert_eq!(result.is_ok(), succeeds, "byte limit {limit}");
        if let Err(error) = result {
            assert_eq!(error.to_string(), "result stream failed");
        }
    }
}
