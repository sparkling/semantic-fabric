//! Ordinary public queries use RDF identity, not source SQL collation.
use super::*;

const MAP: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#edges> rr:logicalTable [ rr:tableName "edges" ];
 rr:subjectMap [ rr:template "http://ex/n/{s}" ];
 rr:predicateObjectMap [ rr:predicate <http://ex/p>;
   rr:objectMap [ rr:template "http://ex/n/{o}"; rr:termType rr:IRI ] ].
<#outer> rr:logicalTable [ rr:tableName "outer_nodes" ];
 rr:subjectMap [ rr:template "http://ex/n/{s}" ];
 rr:predicateObjectMap [ rr:predicate <http://ex/mark>; rr:object "outer" ].
"#;

fn configured(kind: &str, rows: &str) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!("CREATE TABLE edges(s {kind}, o {kind}); CREATE TABLE outer_nodes(s {kind}); INSERT INTO outer_nodes VALUES('a'); INSERT INTO edges VALUES {rows};")).unwrap();
    let mut cfg = support::serve_config(Backend::sqlite(conn), MAP);
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    cfg
}

async fn answer(cfg: ServeConfig, query: &str) -> serde_json::Value {
    let response = router(Arc::new(cfg))
        .oneshot(authenticated(query))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{query}");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn case_distinct_triples_survive_ordinary_select_and_count() {
    for query in [
        "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }",
        "SELECT DISTINCT ?s ?o WHERE { ?s <http://ex/p> ?o }",
        "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o }",
    ] {
        let json = answer(
            configured(
                "TEXT COLLATE NOCASE",
                "('a','B'),('A','B'),('a','B'),('s','a '),('s','a')",
            ),
            query,
        )
        .await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        if query.contains("COUNT") {
            assert_eq!(rows[0]["n"]["value"], "4", "{query}");
        } else {
            let mut pairs: Vec<_> = rows
                .iter()
                .map(|r| {
                    (
                        r["s"]["value"].as_str().unwrap(),
                        r["o"]["value"].as_str().unwrap(),
                    )
                })
                .collect();
            pairs.sort();
            assert_eq!(
                pairs,
                vec![
                    ("http://ex/n/A", "http://ex/n/B"),
                    ("http://ex/n/a", "http://ex/n/B"),
                    ("http://ex/n/s", "http://ex/n/a"),
                    ("http://ex/n/s", "http://ex/n/a%20"),
                ],
                "{query}"
            );
        }
    }
}

#[tokio::test]
async fn constants_cannot_match_only_because_sql_is_case_insensitive() {
    for query in [
        "ASK { <http://ex/n/a> <http://ex/p> ?o }",
        "ASK { ?s <http://ex/p> <http://ex/n/Z> }",
    ] {
        let json = answer(configured("TEXT COLLATE NOCASE", "('A','z')"), query).await;
        assert_eq!(json["boolean"], false, "{query}");
    }
}

#[tokio::test]
async fn ordinary_correlations_do_not_inherit_source_collation() {
    for (pattern, expected) in [
        ("?s <http://ex/mark> ?m . ?s <http://ex/p> ?o", 0),
        ("?s <http://ex/mark> ?m OPTIONAL { ?s <http://ex/p> ?o }", 1),
        (
            "?s <http://ex/mark> ?m FILTER EXISTS { ?s <http://ex/p> ?o }",
            0,
        ),
        (
            "?s <http://ex/mark> ?m FILTER NOT EXISTS { ?s <http://ex/p> ?o }",
            1,
        ),
        ("?s <http://ex/mark> ?m MINUS { ?s <http://ex/p> ?o }", 1),
    ] {
        let query = format!("SELECT ?s ?o WHERE {{ {pattern} }}");
        let json = answer(configured("TEXT COLLATE NOCASE", "('A','z')"), &query).await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), expected, "{query}");
        for row in rows {
            assert_eq!(row["s"]["value"], "http://ex/n/a");
            assert!(row.get("o").is_none(), "{query}");
        }
    }
}

#[tokio::test]
async fn character_padding_does_not_duplicate_a_generated_triple() {
    let json = answer(
        configured("CHARACTER(4)", "('a','b'),('a ','b ')"),
        "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }",
    )
    .await;
    let rows = json["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["s"]["value"], "http://ex/n/a%20%20%20");
    assert_eq!(rows[0]["o"]["value"], "http://ex/n/b%20%20%20");
}

#[tokio::test]
async fn native_ref_joins_preserve_source_equality_and_character_keys() {
    const REF: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#parent> rr:logicalTable [rr:tableName "parent"];
 rr:subjectMap [rr:template "http://ex/{label}"].
<#child> rr:logicalTable [rr:tableName "child"];
 rr:subjectMap [rr:template "http://ex/{s}"];
 rr:predicateObjectMap [rr:predicate <http://ex/ref>;
 rr:objectMap [rr:parentTriplesMap <#parent>;
 rr:joinCondition [rr:child "fk"; rr:parent "k"]]].
"#;
    for (kind, children, parents, expected) in [
        (
            "TEXT COLLATE NOCASE",
            "('child','A')",
            "('a','first')",
            vec!["http://ex/first"],
        ),
        (
            "CHARACTER(4)",
            "('child','a'),('child','a ')",
            "('a','first'),('a ','second')",
            vec!["http://ex/first", "http://ex/second"],
        ),
    ] {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!("CREATE TABLE child(s TEXT, fk {kind}); CREATE TABLE parent(k TEXT, label TEXT); INSERT INTO child VALUES {children}; INSERT INTO parent VALUES {parents};")).unwrap();
        let mut cfg = support::serve_config(Backend::sqlite(conn), REF);
        cfg.set_query_admission(QueryAdmission::Bearer(
            BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
        ));
        let json = answer(cfg, "SELECT ?s ?o WHERE { ?s <http://ex/ref> ?o }").await;
        let mut objects: Vec<_> = json["results"]["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["o"]["value"].as_str().unwrap())
            .collect();
        objects.sort();
        assert_eq!(objects, expected, "{kind}");
    }
}

#[tokio::test]
async fn portable_policy_keeps_native_equality_before_rdf_reconstruction() {
    use sf_serve::{
        PortableRowPolicy, PortableRowRule, ProvisionedBearerAdmission, ProvisionedBearerSubject,
    };
    for (kind, data, value, expected) in [
        ("TEXT COLLATE NOCASE", "('A','B')", "a", "http://ex/n/A"),
        (
            "CHARACTER(4)",
            "('a','B'),('a ','B')",
            "a ",
            "http://ex/n/a%20%20%20",
        ),
    ] {
        let mut cfg = configured(kind, data);
        let policy =
            PortableRowPolicy::new(vec![PortableRowRule::new(0, "edges", "s", value).unwrap()])
                .unwrap();
        let subject = ProvisionedBearerSubject::portable_rows("reader", TOKEN, policy).unwrap();
        cfg.set_query_admission(QueryAdmission::ProvisionedBearers(
            ProvisionedBearerAdmission::new(vec![subject]).unwrap(),
        ));
        let json = answer(cfg, "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }").await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{kind}");
        assert_eq!(rows[0]["s"]["value"], expected);
    }
}

#[tokio::test]
#[ignore = "known pre-existing Ref witness identity gap; required before native Ref exactness qualification"]
async fn reference_atom_dedup_preserves_conflicting_collations_and_projection_bags() {
    const REF: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#parent> rr:logicalTable [rr:tableName "parent"];
 rr:subjectMap [rr:template "http://ex/{label}"].
<#child> rr:logicalTable [rr:tableName "child"];
 rr:subjectMap [rr:template "http://ex/{s}"];
 rr:predicateObjectMap [rr:predicate <http://ex/mark>; rr:object "marked"];
 rr:predicateObjectMap [rr:predicate <http://ex/ref>;
 rr:objectMap [rr:parentTriplesMap <#parent>;
 rr:joinCondition [rr:child "fk"; rr:parent "k"]]].
"#;
    for (child_collation, parent_collation) in [
        ("BINARY", "NOCASE"),
        ("NOCASE", "BINARY"),
        ("NOCASE", "NOCASE"),
    ] {
        for (query, object) in [
            ("SELECT ?o WHERE { ?s <http://ex/ref> ?o }", true),
            ("SELECT ?s ?o WHERE { ?s <http://ex/mark> ?m OPTIONAL { ?s <http://ex/ref> ?o } }", true),
            ("SELECT ?s ?o WHERE { ?s <http://ex/mark> ?m OPTIONAL { ?s <http://ex/ref> ?o FILTER(?o = ?o) } }", true),
            ("SELECT ?s ?o WHERE { ?s <http://ex/mark> ?m OPTIONAL { ?s <http://ex/ref> ?o FILTER(?o = ?s) } }", false),
        ] {
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            conn.execute_batch(&format!("CREATE TABLE child(s TEXT, fk TEXT COLLATE {child_collation}); CREATE TABLE parent(k TEXT COLLATE {parent_collation}, label TEXT); INSERT INTO child VALUES('one','a'),('two','a'); INSERT INTO parent VALUES('A','target'),('a','target');")).unwrap();
            let mut cfg = support::serve_config(Backend::sqlite(conn), REF);
            cfg.set_query_admission(QueryAdmission::Bearer(BearerQueryAdmission::for_service_principal(TOKEN).unwrap()));
            let maps = sf_mapping::parse_r2rml(REF).unwrap();
            sf_sparql::parse_and_translate(query, &maps, sf_sql::Dialect::Sqlite).expect("raw compiler preserves Ref OPTIONAL");
            let json = answer(cfg, query).await;
            let rows = json["results"]["bindings"].as_array().unwrap();
            assert_eq!(rows.len(), 2, "{child_collation}/{parent_collation}: {query}");
            for row in rows {
                if object { assert_eq!(row["o"]["value"], "http://ex/target", "{query}"); }
                else { assert!(row.get("o").is_none(), "{query}"); }
            }
        }
    }
}
