//! Real authenticated SQLite HTTP path equality and correlation checks.
use super::*;

const PATH_MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#edges> rr:logicalTable [ rr:sqlQuery "SELECT parent, child, 1  +  2 FROM edge" ];
 rr:subjectMap [ rr:template "http://ex/n/{parent}" ];
 rr:predicateObjectMap [ rr:predicate <http://ex/reaches>; rr:objectMap [ rr:template "http://ex/n/{child}"; rr:termType rr:IRI ] ].
<#outer> rr:logicalTable [ rr:tableName "outer_nodes" ];
 rr:subjectMap [ rr:template "http://ex/n/{name}" ];
 rr:predicateObjectMap [ rr:predicate <http://ex/mark>; rr:object "outer" ].
"#;

fn config(edges: &str) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE edge(parent TEXT COLLATE NOCASE, child TEXT COLLATE NOCASE); CREATE TABLE outer_nodes(name TEXT COLLATE NOCASE); INSERT INTO outer_nodes VALUES ('a');").unwrap();
    conn.execute_batch(edges).unwrap();
    let mut config = support::serve_config(Backend::sqlite(conn), PATH_MAPPING);
    config.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    config
}

async fn result(config: ServeConfig, query: &str) -> Vec<serde_json::Value> {
    let response = router(Arc::new(config))
        .oneshot(authenticated(query))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{query}");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice::<serde_json::Value>(&body).unwrap()["results"]["bindings"]
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn authenticated_path_preserves_case_distinct_nodes() {
    let result = result(
        config("INSERT INTO edge VALUES('a','B'),('b','c'),('A','B');"),
        "SELECT ?s ?o WHERE { ?s <http://ex/reaches>+ ?o }",
    )
    .await;
    let actual: std::collections::BTreeSet<_> = result
        .iter()
        .map(|r| {
            (
                r["s"]["value"].as_str().unwrap(),
                r["o"]["value"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        actual,
        [
            ("http://ex/n/a", "http://ex/n/B"),
            ("http://ex/n/b", "http://ex/n/c"),
            ("http://ex/n/A", "http://ex/n/B")
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(result.len(), 3);
}

#[tokio::test]
async fn outer_nocase_column_cannot_create_a_path_correlation() {
    for (pattern, expected) in [
        ("?s <http://ex/mark> ?m . ?s <http://ex/reaches>+ ?o", 0),
        (
            "?s <http://ex/mark> ?m OPTIONAL { ?s <http://ex/reaches>+ ?o }",
            1,
        ),
        (
            "?s <http://ex/mark> ?m FILTER EXISTS { ?s <http://ex/reaches>+ ?o }",
            0,
        ),
        (
            "?s <http://ex/mark> ?m FILTER NOT EXISTS { ?s <http://ex/reaches>+ ?o }",
            1,
        ),
        (
            "?s <http://ex/mark> ?m MINUS { ?s <http://ex/reaches>+ ?o }",
            1,
        ),
    ] {
        let query = format!("SELECT ?s ?o WHERE {{ {pattern} }}");
        let rows = result(config("INSERT INTO edge VALUES('A','z');"), &query).await;
        assert_eq!(rows.len(), expected, "{query}");
        for row in rows {
            assert_eq!(row["s"]["value"], "http://ex/n/a");
            assert!(row.get("o").is_none(), "false correlation: {query}");
        }
    }
}
