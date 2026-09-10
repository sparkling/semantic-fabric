use super::*;

const QUERY: &str = "SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s <http://example.test/a>+ ?o } }";

#[tokio::test]
async fn empty_named_graph_inventory_still_requires_mapping_work() {
    let response = router(Arc::new(path_config(QUERY.len() as u64 + key_work(QUERY))))
        .oneshot(authenticated(QUERY))
        .await
        .unwrap();
    assert_budget_problem(response).await;
}

#[tokio::test]
async fn paid_empty_named_graph_inventory_returns_a_complete_empty_answer() {
    let response = router(Arc::new(path_config(100_000)))
        .oneshot(authenticated(QUERY))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(result["results"]["bindings"], serde_json::json!([]));
}

#[tokio::test]
async fn named_graph_inventory_and_reflexive_checks_preserve_exact_path_results() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE edges(s INTEGER, o INTEGER); INSERT INTO edges VALUES (1,2),(2,3);",
    )
    .unwrap();
    let mapping = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
        @prefix ex: <http://example.test/> .
        <#Edges> a rr:TriplesMap; rr:logicalTable [rr:tableName "edges"];
          rr:subjectMap [rr:template "http://example.test/node/{s}"; rr:graph ex:z, ex:a, rr:defaultGraph];
          rr:predicateObjectMap [rr:predicate ex:a; rr:graph ex:z;
            rr:objectMap [rr:template "http://example.test/node/{o}"]]."#;
    let mut config = support::serve_config(Backend::sqlite(conn), mapping);
    config.query_limits = QueryLimits::new(100_000, u64::MAX, u64::MAX, u64::MAX);
    config.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    let app = router(Arc::new(config));
    for (operator, pairs) in [
        ("+", vec![(1, 2), (1, 3), (2, 3)]),
        ("*", vec![(1, 1), (1, 2), (1, 3), (2, 2), (2, 3), (3, 3)]),
        ("?", vec![(1, 1), (1, 2), (2, 2), (2, 3), (3, 3)]),
    ] {
        let query = QUERY.replace("a>+", &format!("a>{operator}"));
        let response = app.clone().oneshot(authenticated(&query)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let mut actual: Vec<_> = result["results"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                ["g", "s", "o"].map(|v| {
                    assert_eq!(row[v]["type"], "uri");
                    row[v]["value"].as_str().unwrap().to_owned()
                })
            })
            .collect();
        let mut expected: Vec<_> = ["a", "z"]
            .into_iter()
            .flat_map(|g| {
                pairs.iter().map(move |(s, o)| {
                    [
                        format!("http://example.test/{g}"),
                        format!("http://example.test/node/{s}"),
                        format!("http://example.test/node/{o}"),
                    ]
                })
            })
            .collect();
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected, "operator {operator}");
    }
}

#[tokio::test]
async fn public_collated_sql_projection_preserves_padding_and_date_type() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE edges(s CHARACTER(4), o DATE); INSERT INTO edges VALUES ('a','2026-09-08');",
    )
    .unwrap();
    let mapping = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
        <#Edges> a rr:TriplesMap; rr:logicalTable [rr:sqlQuery "SELECT s COLLATE BINARY AS s, o COLLATE BINARY AS o FROM edges"];
          rr:subjectMap [rr:template "http://example.test/node/{s}"];
          rr:predicateObjectMap [rr:predicate <http://example.test/a>;
            rr:objectMap [rr:column "o"; rr:datatype <http://www.w3.org/2001/XMLSchema#date>]]."#;
    let mut config = support::serve_config(Backend::sqlite(conn), mapping);
    config.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    let query = "SELECT ?s ?o WHERE { ?s <http://example.test/a> ?o }";
    let response = router(Arc::new(config))
        .oneshot(authenticated(query))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        document["results"]["bindings"],
        serde_json::json!([{
            "s":{"type":"uri","value":"http://example.test/node/a%20%20%20"},
            "o":{"type":"literal","value":"2026-09-08","datatype":"http://www.w3.org/2001/XMLSchema#date"}
        }])
    );
}
