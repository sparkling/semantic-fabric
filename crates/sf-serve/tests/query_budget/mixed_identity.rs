use super::*;

fn configured_mixed(setup: &str, mapping: &str) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(setup).unwrap();
    let mut cfg = support::serve_config(Backend::sqlite(conn), mapping);
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    cfg
}

const ZERO: &str = "CREATE TABLE edges(s CHARACTER(2), o GENERATED ALWAYS AS (CASE WHEN s='a' THEN 0.0 ELSE -0.0 END) VIRTUAL); CREATE TABLE outer_nodes(s TEXT); INSERT INTO edges(s) VALUES('a'),('a ');";

#[tokio::test]
async fn decoded_mixed_template_keys_preserve_join_and_union_bags() {
    for (pattern, expected) in [
        ("?s <http://ex/p> ?o . ?other <http://ex/p> ?o", 2),
        (
            "?s <http://ex/p> ?o OPTIONAL { ?other <http://ex/p> ?o }",
            2,
        ),
        ("{ ?s <http://ex/p> ?o } UNION { ?s <http://ex/p> ?o }", 4),
    ] {
        let query = format!("SELECT ?o WHERE {{ {pattern} }}");
        let json = answer(configured_mixed(ZERO, MAP), &query).await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), expected, "{query}: {json}");
    }
}

#[tokio::test]
async fn mixed_storage_classes_deduplicate_lexical_templates_not_projection_bags() {
    for query in [
        "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
        "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o }",
    ] {
        let setup = "CREATE TABLE edges(s CHARACTER(2),o); CREATE TABLE outer_nodes(s TEXT); INSERT INTO edges VALUES('a',1),('a ',1.0),('A',1.0),('A',1);";
        let json = answer(configured_mixed(setup, MAP), query).await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        if query.contains("COUNT") {
            assert_eq!(rows[0]["n"]["value"], "2");
        } else {
            assert_eq!(rows.len(), 2);
            assert!(rows.iter().all(|r| r["o"]["value"] == "http://ex/n/1"));
        }
    }
}

#[tokio::test]
async fn mixed_lexical_keys_use_blob_and_declared_date_decoders_without_casts() {
    for (kind, data, object) in [
        ("", "('a',X'abff'),('a ',X'abff')", "ABFF"),
        (
            "DATE",
            "('a','2026-09-08'),('a ','2026-09-08')",
            "2026-09-08",
        ),
    ] {
        let setup = format!("CREATE TABLE edges(s CHARACTER(2),o {kind}); CREATE TABLE outer_nodes(s TEXT); INSERT INTO edges VALUES {data};");
        let json = answer(
            configured_mixed(&setup, MAP),
            "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }",
        )
        .await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{kind}: {json}");
        assert_eq!(rows[0]["s"]["value"], "http://ex/n/a%20");
        assert_eq!(rows[0]["o"]["value"], format!("http://ex/n/{object}"));
    }
}

#[tokio::test]
async fn explicit_numeric_literal_keeps_value_comparison_separate_from_identity() {
    let mapping = MAP.replace(
        "rr:template \"http://ex/n/{o}\"; rr:termType rr:IRI",
        "rr:column \"o\"; rr:datatype <http://www.w3.org/2001/XMLSchema#integer>",
    );
    let setup = "CREATE TABLE edges(s TEXT,o INTEGER); CREATE TABLE outer_nodes(s TEXT); INSERT INTO edges VALUES('a',1),('a',10);";
    let json = answer(
        configured_mixed(setup, &mapping),
        "SELECT ?o WHERE { ?s <http://ex/p> ?o FILTER(?o > 9) }",
    )
    .await;
    let rows = json["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{json}");
    assert_eq!(rows[0]["o"]["value"], "10");
}

#[tokio::test]
async fn unsupported_template_constant_filters_keep_their_pre_source_rejection() {
    let query = "SELECT ?o WHERE { ?s <http://ex/p> ?o FILTER(?o = <http://ex/n/-0>) }";
    let maps = sf_mapping::parse_r2rml(MAP).unwrap();
    let error = sf_sparql::parse_and_translate(query, &maps, sf_sql::Dialect::Sqlite)
        .expect_err("existing template-vs-constant FILTER is outside the admitted profile");
    assert!(
        error.to_string().contains("needs a plain column binding"),
        "{error}"
    );
    let response = router(Arc::new(configured_mixed(ZERO, MAP)))
        .oneshot(authenticated(query))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
}
