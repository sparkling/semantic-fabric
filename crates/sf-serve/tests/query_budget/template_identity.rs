//! Template uniqueness must be proved over encoded RDF output, not raw tuples.
use super::*;

fn collision_config() -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE items(a TEXT NOT NULL, b TEXT NOT NULL, PRIMARY KEY(a,b)); INSERT INTO items VALUES('x-','y'),('x','-y'),('X','-y');").unwrap();
    let mapping = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> rr:logicalTable [rr:tableName "items"];
 rr:subjectMap [rr:template "http://ex/{a}-{b}"];
 rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:object "same"].
"#;
    let mut cfg = support::serve_config(Backend::sqlite(conn), mapping);
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    cfg
}

#[tokio::test]
async fn unreserved_template_separator_dedups_before_select_and_count() {
    for query in [
        "SELECT ?s WHERE { ?s <http://ex/p> ?o }",
        "SELECT DISTINCT ?s WHERE { ?s <http://ex/p> ?o }",
        "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o }",
    ] {
        let json = answer(collision_config(), query).await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        if query.contains("COUNT") {
            assert_eq!(rows[0]["n"]["value"], "2", "{query}: {json}");
        } else {
            let mut values: Vec<_> = rows
                .iter()
                .map(|row| row["s"]["value"].as_str().unwrap())
                .collect();
            values.sort();
            assert_eq!(
                values,
                ["http://ex/X--y", "http://ex/x--y"],
                "{query}: {json}"
            );
        }
    }
}

#[tokio::test]
async fn rendered_iri_retains_identity_in_later_filters_and_bgp() {
    for query in [
        "SELECT ?s WHERE { ?s <http://ex/p> ?o FILTER(?s = <http://ex/x--y>) }",
        "SELECT ?s WHERE { ?s <http://ex/p> ?o FILTER(sameTerm(<http://ex/x--y>, ?s)) }",
        "SELECT ?s WHERE { ?s <http://ex/p> ?o . ?s <http://ex/p> ?other FILTER(?s != <http://ex/X--y>) }",
    ] {
        let json = answer(collision_config(), query).await;
        assert_eq!(json["results"]["bindings"].as_array().unwrap().len(), 1, "{query}: {json}");
        assert_eq!(json["results"]["bindings"][0]["s"]["value"], "http://ex/x--y");
    }
}

fn configured(setup: &str, mapping: &str) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(setup).unwrap();
    let mut cfg = support::serve_config(Backend::sqlite(conn), mapping);
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    cfg
}

#[tokio::test]
async fn unicode_template_escaping_matches_rdf_identity_and_count() {
    let mapping = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> rr:logicalTable [rr:tableName "items"];
 rr:subjectMap [rr:template "http://ex/{a}-{b}"];
 rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:object "same"].
"#;
    let setup = "CREATE TABLE items(a TEXT,b TEXT); INSERT INTO items VALUES(char(57344),'x'),(char(57344),'x'),('%EE%80%80','x'),('你好','x');";
    let json = answer(
        configured(setup, mapping),
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
        [
            "http://ex/%25EE%2580%2580-x",
            "http://ex/%EE%80%80-x",
            "http://ex/你好-x"
        ]
    );
    let json = answer(configured(setup, mapping), "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o FILTER(?s = <http://ex/%EE%80%80-x>) }").await;
    assert_eq!(json["results"]["bindings"][0]["n"]["value"], "1");
}

#[tokio::test]
async fn hidden_object_and_graph_keys_preserve_projected_bags() {
    let mapping = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> rr:logicalTable [rr:tableName "items"];
 rr:subjectMap [rr:template "http://ex/{a}-{b}"; rr:graphMap [rr:template "http://ex/g/{g}"]];
 rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:template "http://ex/o/{v}"]].
"#;
    let setup = "CREATE TABLE items(a TEXT,b TEXT,g TEXT,v TEXT); INSERT INTO items VALUES('x-','y','left','one'),('x','-y','left','one'),('x','-y','right','one'),('x','-y','right','two');";
    for (projection, expected) in [("?s", 3), ("DISTINCT ?s", 1), ("(COUNT(*) AS ?n)", 3)] {
        let query = format!("SELECT {projection} WHERE {{ GRAPH ?g {{ ?s <http://ex/p> ?o }} }}");
        let json = answer(configured(setup, mapping), &query).await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        if projection.contains("COUNT") {
            assert_eq!(rows[0]["n"]["value"], expected.to_string(), "{json}");
        } else {
            assert_eq!(rows.len(), expected, "{query}: {json}");
            assert!(rows.iter().all(|row| row["s"]["value"] == "http://ex/x--y"));
        }
    }
}

#[tokio::test]
async fn rendered_keys_use_character_and_storage_decoders_and_suppress_nulls() {
    let mapping = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> rr:logicalTable [rr:tableName "items"];
 rr:subjectMap [rr:template "http://ex/{a}-{b}"];
 rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:object "same"].
"#;
    let setup = "CREATE TABLE items(a CHARACTER(2), b GENERATED ALWAYS AS (CASE WHEN a='a' THEN 0.0 ELSE -0.0 END) VIRTUAL); INSERT INTO items(a) VALUES('a'),('a'),('a '),(''),(NULL);";
    let json = answer(
        configured(setup, mapping),
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
        [
            "http://ex/%20%20--0",
            "http://ex/a%20--0",
            "http://ex/a%20-0"
        ]
    );
}

#[tokio::test]
async fn portable_policy_filters_original_rows_before_rendered_dedup() {
    use sf_serve::{
        PortableRowPolicy, PortableRowRule, ProvisionedBearerAdmission, ProvisionedBearerSubject,
    };
    let mapping = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> rr:logicalTable [rr:tableName "items"];
 rr:subjectMap [rr:template "http://ex/{a}-{b}"];
 rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:object "same"].
"#;
    let setup = "CREATE TABLE items(a TEXT,b TEXT,tenant TEXT COLLATE NOCASE); INSERT INTO items VALUES('x-','y','other'),('x','-y','reader'),('x-','y','READER'),('denied','secret','other');";
    for query in [
        "SELECT ?s WHERE { ?s <http://ex/p> ?o }",
        "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o }",
    ] {
        let mut cfg = configured(setup, mapping);
        let policy =
            PortableRowPolicy::new(vec![
                PortableRowRule::new(0, "items", "tenant", "reader").unwrap()
            ])
            .unwrap();
        let subject = ProvisionedBearerSubject::portable_rows("reader", TOKEN, policy).unwrap();
        cfg.set_query_admission(QueryAdmission::ProvisionedBearers(
            ProvisionedBearerAdmission::new(vec![subject]).unwrap(),
        ));
        let json = answer(cfg, query).await;
        assert_eq!(
            json["results"]["bindings"].as_array().unwrap().len(),
            1,
            "{query}: {json}"
        );
        if query.contains("COUNT") {
            assert_eq!(json["results"]["bindings"][0]["n"]["value"], "1");
        } else {
            assert_eq!(
                json["results"]["bindings"][0]["s"]["value"],
                "http://ex/x--y"
            );
        }
        assert!(!json.to_string().contains("secret"));
    }
}
