//! Actual witnesses, not the set of candidate mappings.
use super::*;

fn config() -> ServeConfig {
    config_extra("")
}
fn config_extra(extra: &str) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE a(id INTEGER, name TEXT COLLATE NOCASE); CREATE TABLE b(id INTEGER, name TEXT); CREATE TABLE unused(id INTEGER, name TEXT); INSERT INTO a VALUES(1,'Alice'),(1,'Alice'),(2,'Bob'); INSERT INTO b VALUES(1,'Alice'),(3,'Alice');").unwrap();
    conn.execute_batch(extra).unwrap();
    let mapping = ["a", "b", "unused"].map(|table| format!(r#"
<urn:map:{table}> a <http://www.w3.org/ns/r2rml#TriplesMap> ;
 <http://www.w3.org/ns/r2rml#logicalTable> [ <http://www.w3.org/ns/r2rml#tableName> "{table}" ] ;
 <http://www.w3.org/ns/r2rml#subjectMap> [ <http://www.w3.org/ns/r2rml#template> "urn:person:{{id}}" ] ;
 <http://www.w3.org/ns/r2rml#predicateObjectMap> [ <http://www.w3.org/ns/r2rml#predicate> <urn:name> ; <http://www.w3.org/ns/r2rml#objectMap> [ <http://www.w3.org/ns/r2rml#column> "name" ] ] .
"#)).join("\n");
    let mut cfg = support::serve_config(Backend::sqlite(conn), &mapping);
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    cfg
}

fn origins(row: &Value) -> Vec<&str> {
    let mut maps: Vec<_> = row["provenance"]["@graph"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|node| node["sf:mappingId"].as_str())
        .collect();
    maps.sort();
    maps
}

#[tokio::test]
async fn multiple_maps_merge_duplicate_witnesses_but_not_projected_bag_occurrences() {
    let rows = records(
        Arc::new(config()),
        "SELECT ?s ?name WHERE { ?s <urn:name> ?name }",
    )
    .await;
    let mut actual = rows
        .iter()
        .filter(|row| row["type"] == "solution")
        .map(|row| {
            (
                row["result"]["results"]["bindings"][0]["s"]["value"]
                    .as_str()
                    .unwrap(),
                origins(row),
            )
        })
        .collect::<Vec<_>>();
    actual.sort();
    assert_eq!(
        actual,
        [
            ("urn:person:1", vec!["urn:map:a", "urn:map:b"]),
            ("urn:person:2", vec!["urn:map:a"]),
            ("urn:person:3", vec!["urn:map:b"])
        ]
    );
    let projected = records(
        Arc::new(config()),
        "SELECT ?name WHERE { ?s <urn:name> ?name }",
    )
    .await;
    assert_eq!(projected.last().unwrap()["solutions"], 3);
    let distinct = records(
        Arc::new(config()),
        "SELECT DISTINCT ?name WHERE { ?s <urn:name> ?name }",
    )
    .await;
    assert_eq!(distinct.last().unwrap()["solutions"], 2);
    let alice = distinct
        .iter()
        .find(|row| row["result"]["results"]["bindings"][0]["name"]["value"] == "Alice")
        .unwrap();
    assert_eq!(origins(alice), ["urn:map:a", "urn:map:b"]);
}

#[tokio::test]
async fn multiple_maps_preserve_explicit_union_bags() {
    let rows = records(
        Arc::new(config()),
        "SELECT ?name WHERE { { ?s <urn:name> ?name } UNION { ?s <urn:name> ?name } }",
    )
    .await;
    assert_eq!(rows.last().unwrap()["solutions"], 6);
    assert_eq!(
        rows.iter()
            .filter(|row| row["type"] == "solution" && origins(row) == ["urn:map:a", "urn:map:b"])
            .count(),
        2
    );
}

#[tokio::test]
async fn bounded_rdf_join_ignores_sql_collation_and_retains_actual_origins() {
    let rows = records(
        Arc::new(config_extra("INSERT INTO b VALUES(4,'alice');")),
        "SELECT ?s ?t ?name WHERE { ?s <urn:name> ?name . ?t <urn:name> ?name }",
    )
    .await;
    assert_eq!(rows.last().unwrap()["solutions"], 6);
    let mixed = rows
        .iter()
        .find(|row| {
            row["result"]["results"]["bindings"][0]["s"]["value"] == "urn:person:1"
                && row["result"]["results"]["bindings"][0]["t"]["value"] == "urn:person:3"
        })
        .unwrap();
    assert_eq!(origins(mixed), ["urn:map:a", "urn:map:b"]);
    let rows = records(
        Arc::new(config()),
        "SELECT ?s WHERE { ?s <urn:name> \"alice\" }",
    )
    .await;
    assert_eq!(
        rows.last().unwrap()["solutions"],
        0,
        "RDF constant match does not inherit NOCASE"
    );
}

#[tokio::test]
async fn lineage_filters_null_terms_and_collects_late_origins_before_slice() {
    let rows = records(
        Arc::new(config_extra(
            "INSERT INTO a VALUES(NULL,'orphan'),(99,NULL);",
        )),
        "SELECT ?s WHERE { ?s <urn:name> ?name }",
    )
    .await;
    assert_eq!(rows.last().unwrap()["solutions"], 3);
    assert!(!serde_json::to_string(&rows).unwrap().contains("orphan"));
    let rows = records(
        Arc::new(config()),
        "SELECT DISTINCT ?name WHERE { ?s <urn:name> ?name } LIMIT 1",
    )
    .await;
    assert_eq!(rows.last().unwrap()["solutions"], 1);
    assert_eq!(origins(&rows[1]), ["urn:map:a", "urn:map:b"]);
}

#[tokio::test]
async fn multi_mapping_graph_reification_keeps_product_graph_separate() {
    let rows = records(
        Arc::new(config()),
        "CONSTRUCT { ?s <urn:name> ?name } WHERE { ?s <urn:name> ?name }",
    )
    .await;
    assert_eq!(rows[0]["profile"], "bounded-mapping-source-graph-v1");
    assert_eq!(rows.last().unwrap()["tripleOccurrences"], 3);
    let dataset = rows
        .iter()
        .filter_map(|r| r["dataset"].as_str())
        .collect::<String>();
    let quads = oxttl::NQuadsParser::new()
        .for_slice(dataset.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        quads
            .iter()
            .filter(|q| q.graph_name == oxrdf::GraphName::DefaultGraph)
            .count(),
        3
    );
    assert_eq!(quads.iter().filter(|q|q.predicate.as_str()=="http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies").count(),3);
    let maps = quads
        .iter()
        .filter(|q| q.predicate.as_str() == "urn:semantic-fabric:lineage:mappingId")
        .map(|q| q.object.to_string())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(maps.len(), 2);
    assert!(!dataset.contains("urn:map:unused"));
}

#[tokio::test]
async fn multi_origin_exceeding_witness_or_byte_cap_never_completes() {
    let extra="WITH RECURSIVE n(x) AS (VALUES(10) UNION ALL SELECT x+1 FROM n WHERE x<1040) INSERT INTO a SELECT x,'large' FROM n;";
    let response = router(Arc::new(config_extra(extra)))
        .oneshot(request(
            "SELECT ?name WHERE { ?s <urn:name> ?name } LIMIT 1",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.into_body().collect().await.is_err(),
        "LIMIT must not hide later witness overflow"
    );
    let query = "SELECT ?name WHERE { ?s <urn:name> ?name }";
    let response = router(Arc::new(config()))
        .oneshot(request(query))
        .await
        .unwrap();
    let size = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .len() as u64;
    for (limit, ok) in [(size, true), (size - 1, false)] {
        let mut cfg = config();
        cfg.query_limits = sf_core::query_control::QueryLimits::new(10000, 10000, 10000, limit);
        let response = router(Arc::new(cfg)).oneshot(request(query)).await.unwrap();
        assert_eq!(response.into_body().collect().await.is_ok(), ok);
    }
}

#[tokio::test]
async fn nested_union_join_and_hidden_blank_bindings_preserve_bags() {
    let rows=records(Arc::new(config()),"SELECT ?name WHERE { { { ?s <urn:name> ?name } UNION { { ?s <urn:name> ?name } UNION { ?s <urn:name> ?name } } } { { ?t <urn:name> ?name } UNION { ?t <urn:name> ?name } } }").await;
    assert_eq!(rows.last().unwrap()["solutions"], 30);
    let rows = records(
        Arc::new(config()),
        "SELECT ?name WHERE { _:w <urn:name> ?name }",
    )
    .await;
    assert_eq!(rows.last().unwrap()["solutions"], 3);
    let rows = records(Arc::new(config()), "SELECT ?x WHERE { ?x <urn:name> ?x }").await;
    assert_eq!(rows.last().unwrap()["solutions"], 0);
    let rows = records(
        Arc::new(config()),
        "SELECT DISTINCT ?name WHERE { { ?s <urn:name> ?name } UNION { ?s <urn:name> ?name } }",
    )
    .await;
    assert_eq!(rows.last().unwrap()["solutions"], 2);
}

#[tokio::test]
async fn multiple_origin_policy_is_applied_before_any_result_or_provenance() {
    use sf_serve::{
        PortableRowPolicy, PortableRowRule, ProvisionedBearerAdmission, ProvisionedBearerSubject,
    };
    let mut cfg = config();
    let second = "multi-lineage-test-token-two-123456789";
    let subject = |id, token, value| {
        ProvisionedBearerSubject::portable_rows(
            id,
            token,
            PortableRowPolicy::new(
                ["a", "b", "unused"]
                    .map(|table| PortableRowRule::new(0, table, "name", value).unwrap())
                    .to_vec(),
            )
            .unwrap(),
        )
        .unwrap()
    };
    cfg.set_query_admission(QueryAdmission::ProvisionedBearers(
        ProvisionedBearerAdmission::new(vec![
            subject("private-one", TOKEN, "Alice"),
            subject("private-two", second, "Bob"),
        ])
        .unwrap(),
    ));
    let app = router(Arc::new(cfg));
    for query in [
        "SELECT ?s ?name WHERE { ?s <urn:name> ?name }",
        "CONSTRUCT { ?s <urn:name> ?name } WHERE { ?s <urn:name> ?name }",
    ] {
        for (token, allowed, forbidden) in [
            (TOKEN, "Alice", "Bob"),
            (second, "Bob", "Alice"),
            (TOKEN, "Alice", "Bob"),
        ] {
            let mut req = request(query);
            req.headers_mut().insert(
                header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
            let response = app.clone().oneshot(req).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.contains(allowed));
            for value in [forbidden, TOKEN, second, "private-one", "private-two"] {
                assert!(!text.contains(value));
            }
            if token == second {
                assert!(!text.contains("\"sf:mappingId\":\"urn:map:b\""));
            }
        }
    }
}

#[tokio::test]
async fn excluded_multi_origin_shapes_reject_before_source_work() {
    for query in [
        "SELECT ?s WHERE { ?s ?p ?o }",
        "SELECT ?s WHERE { ?s a <urn:Person> }",
        "SELECT ?s WHERE { { SELECT DISTINCT ?s WHERE { ?s <urn:name> ?name } } UNION { ?s <urn:name> ?other } }",
        "SELECT ?s WHERE { ?s <urn:name> ?name FILTER(?name = \"Alice\") }",
    ] {
        let mut cfg=config();cfg.query_limits=sf_core::query_control::QueryLimits::new(10000,0,10000,100000);
        let response=router(Arc::new(cfg)).oneshot(request(query)).await.unwrap();
        assert_eq!(response.status(),StatusCode::NOT_IMPLEMENTED,"{query}");
    }
}
