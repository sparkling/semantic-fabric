use super::*;

fn configured(setup: &str, datatype: &str) -> ServeConfig {
    with_spec(
        setup,
        &format!("rr:datatype <http://www.w3.org/2001/XMLSchema#{datatype}>"),
        false,
    )
}

fn with_spec(setup: &str, spec: &str, unique_subject: bool) -> ServeConfig {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection.execute_batch(setup).unwrap();
    let mapping = format!(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
        <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/s>;
        rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "v"; {spec}]]."#
    );
    let mapping = if unique_subject {
        mapping.replace(
            "rr:subject <http://ex/s>",
            "rr:subjectMap [rr:template \"http://ex/s/{id}\"]",
        )
    } else {
        mapping
    };
    let mut cfg = support::serve_config(Backend::sqlite(connection), &mapping);
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    cfg
}

#[tokio::test]
async fn canonical_literal_distinct_does_not_follow_raw_source_uniqueness() {
    let setup = "CREATE TABLE items(id INTEGER PRIMARY KEY, v BOOLEAN UNIQUE); INSERT INTO items VALUES(1,1),(2,'true');";
    let spec = "rr:datatype <http://www.w3.org/2001/XMLSchema#boolean>";
    for (query, expected) in [
        ("SELECT ?o WHERE { ?s <http://ex/p> ?o }", 2),
        ("SELECT DISTINCT ?o WHERE { ?s <http://ex/p> ?o }", 1),
        (
            "SELECT DISTINCT ?o WHERE { ?s <http://ex/p> ?o } ORDER BY ?o LIMIT 1 OFFSET 1",
            0,
        ),
    ] {
        let json = answer(with_spec(setup, spec, true), query).await;
        assert_eq!(
            json["results"]["bindings"].as_array().unwrap().len(),
            expected,
            "{query}: {json}"
        );
    }
}

#[tokio::test]
async fn different_datatype_and_language_preserve_decoded_lexical() {
    let setup = "CREATE TABLE items(v BOOLEAN); INSERT INTO items VALUES(1),('true');";
    for (spec, literal) in [
        (
            "rr:datatype <http://www.w3.org/2001/XMLSchema#string>",
            "\"1\"",
        ),
        ("rr:language \"en\"", "\"1\"@en"),
    ] {
        let json = answer(
            with_spec(setup, spec, false),
            "SELECT DISTINCT ?o WHERE { ?s <http://ex/p> ?o }",
        )
        .await;
        assert_eq!(
            json["results"]["bindings"].as_array().unwrap().len(),
            2,
            "{json}"
        );
        for pattern in [
            format!("?s <http://ex/p> {literal}"),
            format!("?s <http://ex/p> ?o FILTER(sameTerm(?o, {literal}))"),
        ] {
            let json = answer(
                with_spec(setup, spec, false),
                &format!("SELECT ?s WHERE {{ {pattern} }}"),
            )
            .await;
            assert_eq!(
                json["results"]["bindings"].as_array().unwrap().len(),
                1,
                "{json}"
            );
        }
    }
}

#[test]
fn dynamic_natural_key_includes_datatype_in_raw_compiler_execution() {
    // Unknown natural types are not authored-serving admission authority.
    // The public raw compiler still supports dynamic SQLite construction.
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE TABLE items(v); INSERT INTO items VALUES(1),('1'),(1),('1');")
        .unwrap();
    let maps = sf_mapping::parse_r2rml(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> . <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://ex/s>; rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "v"]]."#).unwrap();
    for query in [
        "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
        "SELECT DISTINCT ?o WHERE { ?s <http://ex/p> ?o }",
    ] {
        let plan = sf_sparql::parse_and_translate(query, &maps, sf_sql::Dialect::Sqlite).unwrap();
        let result = sf_sparql::exec::select(&plan, &connection).unwrap();
        assert_eq!(result.rows.len(), 2, "{query}: {:?}", result.rows);
        assert_ne!(result.rows[0], result.rows[1]);
    }
}

#[tokio::test]
async fn explicit_natural_group_by_counts_canonical_terms() {
    let json = answer(with_spec("CREATE TABLE items(id INTEGER PRIMARY KEY, v BOOLEAN); INSERT INTO items VALUES(1,1),(2,'true');", "rr:datatype <http://www.w3.org/2001/XMLSchema#boolean>", true), "SELECT ?o (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o } GROUP BY ?o").await;
    let rows = json["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{json}");
    assert_eq!(rows[0]["o"]["value"], "true", "{json}");
    assert_eq!(rows[0]["n"]["value"], "2", "{json}");
}

#[tokio::test]
async fn mixed_iri_and_explicit_natural_roles_preserve_original_payload() {
    for (setup, datatype, expected) in [
        (
            "CREATE TABLE items(v BOOLEAN); INSERT INTO items VALUES(1),('true');",
            "boolean",
            vec!["true", "true"],
        ),
        (
            "CREATE TABLE items(v); INSERT INTO items VALUES(1),(1.0);",
            "double",
            vec!["1", "1.0E0"],
        ),
    ] {
        for modifier in ["", "DISTINCT ", "GROUP"] {
            let connection = rusqlite::Connection::open_in_memory().unwrap();
            connection.execute_batch(setup).unwrap();
            let mapping = format!(
                r#"@prefix rr: <http://www.w3.org/ns/r2rml#> . <#m> rr:logicalTable [rr:tableName "items"]; rr:subjectMap [rr:template "http://ex/s/{{v}}"]; rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "v"; rr:datatype <http://www.w3.org/2001/XMLSchema#{datatype}>]]."#
            );
            let mut cfg = support::serve_config(Backend::sqlite(connection), &mapping);
            cfg.set_query_admission(QueryAdmission::Bearer(
                BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
            ));
            let query = if modifier == "GROUP" {
                "SELECT ?s ?o (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o } GROUP BY ?s ?o"
                    .to_owned()
            } else {
                format!("SELECT {modifier}?s ?o WHERE {{ ?s <http://ex/p> ?o }}")
            };
            let json = answer(cfg, &query).await;
            let mut actual: Vec<_> = json["results"]["bindings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["o"]["value"].as_str().unwrap())
                .collect();
            actual.sort();
            assert_eq!(actual, expected, "{json}");
        }
    }
}

#[tokio::test]
async fn explicit_natural_type_is_coherent_in_public_output_and_identity() {
    for (setup, datatype, lexical, expected) in [
        ("CREATE TABLE items(v BOOLEAN); INSERT INTO items VALUES(1),('true'),(NULL);", "boolean", "true", 1),
        ("CREATE TABLE items(v TIMESTAMP); INSERT INTO items VALUES('2024-01-02 03:04:05.120000'),('2024-01-02T03:04:05.12');", "dateTime", "2024-01-02T03:04:05.12", 1),
        ("CREATE TABLE items(v); INSERT INTO items VALUES(1.0),('1');", "double", "1.0E0", 2),
    ] {
        for modifier in ["", "DISTINCT "] {
            let query = format!("SELECT {modifier}?o WHERE {{ ?s <http://ex/p> ?o }}");
            let json = answer(configured(setup, datatype), &query).await;
            let rows = json["results"]["bindings"].as_array().unwrap();
            assert_eq!(rows.len(), expected, "{query}: {json}");
            assert!(rows.iter().any(|row| row["o"]["value"] == lexical), "{json}");
        }
        let literal = format!("\"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#{datatype}>");
        for pattern in [format!("?s <http://ex/p> {literal}"), format!("?s <http://ex/p> ?o FILTER(sameTerm(?o, {literal}))")] {
            let json = answer(configured(setup, datatype), &format!("SELECT ?s WHERE {{ {pattern} }}")).await;
            assert_eq!(json["results"]["bindings"].as_array().unwrap().len(), 1, "{json}");
        }
        let json = answer(configured(setup, datatype), "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o }").await;
        assert_eq!(json["results"]["bindings"][0]["n"]["value"], expected.to_string());
    }
}
