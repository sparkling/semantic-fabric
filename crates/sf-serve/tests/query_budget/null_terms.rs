//! Authenticated public queries must not observe NULL-generated non-triples.
use super::*;

fn config(class: bool) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE items(id TEXT,value TEXT);
        INSERT INTO items VALUES(NULL,'value');",
    )
    .unwrap();
    let mapping = format!(
        r#"
      @prefix rr: <http://www.w3.org/ns/r2rml#> .
      <http://ex/map> rr:logicalTable [ rr:tableName "items" ];
        rr:subjectMap [ rr:template "http://ex/{{id}}" {} ];
        rr:predicateObjectMap [ rr:predicate <http://ex/p>; rr:objectMap [ rr:column "value" ] ]."#,
        if class {
            "; rr:class <http://ex/C>"
        } else {
            ""
        }
    );
    let mut config = support::serve_config(Backend::sqlite(conn), &mapping);
    config.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    config
}

async fn result(config: ServeConfig, query: &str) -> serde_json::Value {
    let response = router(Arc::new(config))
        .oneshot(authenticated(query))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{query}");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn authenticated_select_ask_and_count_exclude_absent_terms() {
    for (class, pattern) in [(false, "?s <http://ex/p> ?o"), (true, "?s a ?o")] {
        let json = result(config(class), &format!("SELECT ?o WHERE {{ {pattern} }}")).await;
        assert_eq!(json["results"]["bindings"], serde_json::json!([]));
        let json = result(config(class), &format!("ASK {{ {pattern} }}")).await;
        assert_eq!(json["boolean"], false);
        let json = result(
            config(class),
            &format!("SELECT (COUNT(*) AS ?n) WHERE {{ {pattern} }}"),
        )
        .await;
        assert_eq!(json["results"]["bindings"][0]["n"]["value"], "0");
    }
}

#[tokio::test]
async fn authenticated_optional_and_anti_join_preserve_the_left_solution() {
    for (condition, expected) in [
        ("OPTIONAL", 1),
        ("FILTER EXISTS", 0),
        ("FILTER NOT EXISTS", 1),
        ("MINUS", 1),
    ] {
        let query = format!("SELECT ?o ?s WHERE {{ VALUES ?o {{ \"value\" }} {condition} {{ ?s <http://ex/p> ?o }} }}");
        let json = result(config(false), &query).await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), expected, "{query}");
        for row in rows {
            assert_eq!(row["o"]["value"], "value");
            assert!(row.get("s").is_none(), "false right binding: {query}");
        }
    }
}
