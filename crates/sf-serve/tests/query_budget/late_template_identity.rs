//! Public authenticated requests compare the finalized template IRI, not its slots.
use super::*;

fn config(recipe: &str, setup: &str) -> ServeConfig {
    let mapping = format!(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#m> rr:logicalTable [rr:tableName "items"]; rr:subjectMap [rr:template "{recipe}"];
rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:object "same"]."#
    );
    mapping_config(&mapping, setup)
}

fn mapping_config(mapping: &str, setup: &str) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(setup).unwrap();
    let mut cfg = support::serve_config(Backend::sqlite(conn), mapping);
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    cfg
}

#[tokio::test]
async fn mixed_static_late_joins_and_filters_compare_whole_iris() {
    let mapping = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#m> rr:logicalTable [rr:tableName "items"]; rr:subjectMap [rr:template "{v}://host/x"];
rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:object "same"].
<#n> rr:logicalTable [rr:tableName "other"]; rr:subjectMap [rr:template "http://host/{x}"];
rr:predicateObjectMap [rr:predicate <http://ex/q>; rr:object "other"]."#;
    let setup = format!("{SETUP} CREATE TABLE other(x TEXT); INSERT INTO other VALUES('x');");
    for pattern in [
        "?s <http://ex/p> ?o . ?s <http://ex/q> ?v",
        "?s <http://ex/p> ?o . ?t <http://ex/q> ?v FILTER(?s = ?t)",
        "?s <http://ex/p> ?o . ?t <http://ex/q> ?v FILTER(sameTerm(?s, ?t))",
    ] {
        let query = format!("SELECT ?s WHERE {{ {pattern} }}");
        let json = answer(mapping_config(mapping, &setup), &query).await;
        assert_eq!(
            json["results"]["bindings"].as_array().unwrap().len(),
            1,
            "{query}: {json}"
        );
        assert_eq!(
            json["results"]["bindings"][0]["s"]["value"],
            "http://host/x"
        );
    }
    for (pattern, expected) in [
        ("?s <http://ex/p> ?o OPTIONAL { ?s <http://ex/q> ?v }", 2),
        (
            "?s <http://ex/p> ?o FILTER EXISTS { ?s <http://ex/q> ?v }",
            1,
        ),
        (
            "?s <http://ex/p> ?o FILTER NOT EXISTS { ?s <http://ex/q> ?v }",
            1,
        ),
        ("?s <http://ex/p> ?o MINUS { ?s <http://ex/q> ?v }", 1),
        (
            "?s <http://ex/p> ?o OPTIONAL { ?s <http://ex/q> ?v } FILTER(?v != <urn:none>)",
            1,
        ),
    ] {
        let json = answer(
            mapping_config(mapping, &setup),
            &format!("SELECT ?s WHERE {{ {pattern} }}"),
        )
        .await;
        assert_eq!(
            json["results"]["bindings"].as_array().unwrap().len(),
            expected,
            "{pattern}: {json}"
        );
    }
}

#[tokio::test]
async fn late_atom_policy_precedes_dedup_even_for_fully_bound_queries() {
    use sf_serve::{
        PortableRowPolicy, PortableRowRule, ProvisionedBearerAdmission, ProvisionedBearerSubject,
    };
    let setup = "CREATE TABLE items(v TEXT,a TEXT,b TEXT,tenant TEXT); INSERT INTO items VALUES('http','x-','y','denied'),('http','x','-y','reader'),('http','x','-y','reader'),('http','secret','x','denied');";
    for query in [
        "SELECT ?s WHERE { ?s <http://ex/p> ?o }",
        "SELECT (COUNT(*) AS ?n) WHERE { <http://ex/x--y> <http://ex/p> ?o }",
    ] {
        let mut cfg = config("{v}://ex/{a}-{b}", setup);
        let policy =
            PortableRowPolicy::new(vec![
                PortableRowRule::new(0, "items", "tenant", "reader").unwrap()
            ])
            .unwrap();
        cfg.set_query_admission(QueryAdmission::ProvisionedBearers(
            ProvisionedBearerAdmission::new(vec![ProvisionedBearerSubject::portable_rows(
                "reader", TOKEN, policy,
            )
            .unwrap()])
            .unwrap(),
        ));
        let json = answer(cfg, query).await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        if query.contains("COUNT") {
            assert_eq!(rows[0]["n"]["value"], "1");
        } else {
            assert_eq!(rows[0]["s"]["value"], "http://ex/x--y");
        }
    }
}

#[tokio::test]
async fn hidden_late_graph_and_object_preserve_bags() {
    let mapping = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#m> rr:logicalTable [rr:tableName "items"];
rr:subjectMap [rr:constant <http://ex/s>; rr:graphMap [rr:template "{g}:graph"]];
rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:template "{v}:object"]]."#;
    let setup = "CREATE TABLE items(g TEXT, v TEXT); INSERT INTO items VALUES('urn','urn'),('urn','urn'),('1','urn'),('1','1'),(NULL,'urn'),('urn',NULL);";
    for projection in ["?s", "(COUNT(*) AS ?n)"] {
        for query_source in [false, true] {
            let mapping = if query_source {
                mapping.replace(
                    "rr:tableName \"items\"",
                    "rr:sqlQuery \"SELECT g,v FROM items\"",
                )
            } else {
                mapping.to_owned()
            };
            let query =
                format!("SELECT {projection} WHERE {{ GRAPH ?g {{ ?s <http://ex/p> ?o }} }}");
            let json = answer(mapping_config(&mapping, setup), &query).await;
            let rows = json["results"]["bindings"].as_array().unwrap();
            if projection.contains("COUNT") {
                assert_eq!(rows[0]["n"]["value"], "3", "{json}");
            } else {
                assert_eq!(rows.len(), 3, "{json}");
            }
        }
    }
}

#[tokio::test]
async fn invalid_template_is_not_hidden_by_count_and_zero_slot_uses_callback() {
    for query in [
        "SELECT ?s WHERE { ?s <http://ex/p> ?o }",
        "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o }",
    ] {
        let cfg = config(
            "http://host/%{v}",
            "CREATE TABLE items(v TEXT PRIMARY KEY); INSERT INTO items VALUES('zz');",
        );
        let response = router(Arc::new(cfg))
            .oneshot(authenticated(query))
            .await
            .unwrap();
        if response.status().is_success() {
            assert!(
                response.into_body().collect().await.is_err(),
                "invalid template returned a successful body: {query}"
            );
        } else {
            assert!(response.status().is_server_error());
        }
    }
    let json = answer(config("1:x", "CREATE TABLE items(v TEXT); INSERT INTO items VALUES('a'),('b');"), "SELECT (COUNT(*) AS ?n) WHERE { <http://example.com/base/1:x> <http://ex/p> ?o FILTER(<urn:a> = <urn:a>) }").await;
    assert_eq!(json["results"]["bindings"][0]["n"]["value"], "1");
}

const SETUP: &str =
    "CREATE TABLE items(v TEXT); INSERT INTO items VALUES('http'),('1'),('http'),(NULL);";

#[tokio::test]
async fn constant_bound_subject_keeps_rendered_atom_cardinality() {
    let setup = "CREATE TABLE items(v TEXT,a TEXT,b TEXT); INSERT INTO items VALUES('http','x-','y'),('http','x','-y');";
    for query in [
        "SELECT ?o WHERE { <http://ex/x--y> <http://ex/p> ?o }",
        "SELECT (COUNT(*) AS ?n) WHERE { <http://ex/x--y> <http://ex/p> ?o }",
    ] {
        let json = answer(config("{v}://ex/{a}-{b}", setup), query).await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{json}");
        if query.contains("COUNT") {
            assert_eq!(rows[0]["n"]["value"], "1", "{json}");
        }
    }
}

#[tokio::test]
async fn dynamic_scheme_select_count_and_constant_patterns_use_finalized_identity() {
    let json = answer(
        config("{v}://host/x", SETUP),
        "SELECT ?s WHERE { ?s <http://ex/p> ?o }",
    )
    .await;
    let mut values: Vec<_> = json["results"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["s"]["value"].as_str().unwrap())
        .collect();
    values.sort();
    assert_eq!(
        values,
        ["http://example.com/base/1://host/x", "http://host/x"]
    );
    for (pattern, expected) in [
        ("?s <http://ex/p> ?o", "2"),
        ("<http://host/x> <http://ex/p> ?o", "1"),
        ("<http://example.com/base/1://host/x> <http://ex/p> ?o", "1"),
        ("?s <http://ex/p> ?o FILTER(?s = <http://host/x>)", "1"),
        (
            "?s <http://ex/p> ?o FILTER(sameTerm(?s, <http://example.com/base/1://host/x>))",
            "1",
        ),
        ("?s <http://ex/p> ?o FILTER(?s != <http://host/x>)", "1"),
    ] {
        let query = format!("SELECT (COUNT(*) AS ?n) WHERE {{ {pattern} }}");
        let json = answer(config("{v}://host/x", SETUP), &query).await;
        assert_eq!(
            json["results"]["bindings"][0]["n"]["value"], expected,
            "{query}: {json}"
        );
    }
}

#[tokio::test]
async fn dynamic_port_and_percent_escape_boundary_finalize_after_encoding() {
    for (recipe, setup, expected) in [
        (
            "http://host:{v}/x",
            "CREATE TABLE items(v TEXT); INSERT INTO items VALUES('80'),('abc');",
            vec![
                "http://example.com/base/http://host:abc/x",
                "http://host:80/x",
            ],
        ),
        (
            "http://host/%{v}",
            "CREATE TABLE items(v TEXT); INSERT INTO items VALUES('20');",
            vec!["http://host/%20"],
        ),
    ] {
        let json = answer(
            config(recipe, setup),
            "SELECT ?s WHERE { ?s <http://ex/p> ?o }",
        )
        .await;
        let mut values: Vec<_> = json["results"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["s"]["value"].as_str().unwrap())
            .collect();
        values.sort();
        assert_eq!(values, expected);
    }
}
