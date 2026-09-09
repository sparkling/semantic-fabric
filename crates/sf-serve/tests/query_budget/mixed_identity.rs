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
async fn literal_iri_filter_mismatch_preserves_kind_and_unbound_error() {
    let mapping = format!(
        r#"{}
<#iris> rr:logicalTable [rr:tableName "iris"]; rr:subject <http://ex/right>;
 rr:predicateObjectMap [rr:predicate <http://ex/q>; rr:objectMap [rr:column "o"; rr:termType rr:IRI]]."#,
        literal_mapping().replace("#double>", "#hexBinary>")
    );
    let setup = "CREATE TABLE edges(s TEXT,o BLOB); CREATE TABLE outer_nodes(s TEXT); CREATE TABLE iris(o TEXT); INSERT INTO edges VALUES('a',X'AB'); INSERT INTO iris VALUES('AB');";
    for expression in ["?literal = ?iri", "sameTerm(?literal, ?iri)"] {
        for (optional, negate, expected) in [
            (false, false, 0),
            (false, true, 1),
            (true, false, 0),
            (true, true, 0),
        ] {
            let right = if optional {
                "OPTIONAL { <http://ex/right> <http://ex/q> ?iri FILTER(?iri = <http://ex/missing>) }"
            } else {
                "<http://ex/right> <http://ex/q> ?iri"
            };
            let filter = if negate {
                format!("!({expression})")
            } else {
                expression.into()
            };
            let query = format!(
                "SELECT ?literal WHERE {{ ?s <http://ex/p> ?literal . {right} FILTER({filter}) }}"
            );
            let json = answer(configured_mixed(setup, &mapping), &query).await;
            assert_eq!(
                json["results"]["bindings"].as_array().unwrap().len(),
                expected,
                "{query}: {json}"
            );
        }
    }
}

fn literal_mapping() -> String {
    MAP.replace(
        "rr:template \"http://ex/n/{o}\"; rr:termType rr:IRI",
        "rr:column \"o\"; rr:datatype <http://www.w3.org/2001/XMLSchema#double>",
    )
}

fn column_iri_mapping() -> String {
    // parse_r2rml currently assigns its document fallback base to column maps.
    MAP.replace(
        "rr:template \"http://ex/n/{o}\"; rr:termType rr:IRI",
        "rr:column \"o\"; rr:termType rr:IRI",
    )
}

#[tokio::test]
async fn column_iri_base_resolution_precedes_triple_dedup_and_count() {
    let setup = "CREATE TABLE edges(s TEXT,o TEXT); CREATE TABLE outer_nodes(s TEXT); INSERT INTO edges VALUES('a','AB'),('a','http://example.com/base/AB');";
    for query in [
        "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
        "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o }",
    ] {
        let json = answer(configured_mixed(setup, &column_iri_mapping()), query).await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{query}: {json}");
        if query.contains("COUNT") {
            assert_eq!(rows[0]["n"]["value"], "1", "{json}");
        } else {
            assert_eq!(
                rows[0]["o"]["value"], "http://example.com/base/AB",
                "{json}"
            );
        }
    }
}

#[tokio::test]
async fn column_iri_base_resolution_precedes_constant_and_filter_matching() {
    let setup = "CREATE TABLE edges(s TEXT,o TEXT); CREATE TABLE outer_nodes(s TEXT); INSERT INTO edges VALUES('a','AB');";
    for pattern in [
        "?s <http://ex/p> <http://example.com/base/AB>",
        "?s <http://ex/p> ?o FILTER(?o = <http://example.com/base/AB>)",
        "?s <http://ex/p> ?o FILTER(sameTerm(?o, <http://example.com/base/AB>))",
    ] {
        let query = format!("SELECT ?s WHERE {{ {pattern} }}");
        let json = answer(configured_mixed(setup, &column_iri_mapping()), &query).await;
        assert_eq!(
            json["results"]["bindings"].as_array().unwrap().len(),
            1,
            "{query}: {json}"
        );
    }
}

#[tokio::test]
async fn column_iri_base_resolution_precedes_bgp_and_variable_filter_matching() {
    let mapping = format!(
        r#"{}
<#right> rr:logicalTable [rr:tableName "right_values"]; rr:subject <http://ex/right>;
 rr:predicateObjectMap [rr:predicate <http://ex/q>; rr:objectMap [rr:column "o"; rr:termType rr:IRI]]."#,
        column_iri_mapping()
    );
    let setup = "CREATE TABLE edges(s TEXT,o TEXT); CREATE TABLE outer_nodes(s TEXT); CREATE TABLE right_values(o TEXT); INSERT INTO edges VALUES('a','AB'); INSERT INTO right_values VALUES('http://example.com/base/AB');";
    for pattern in [
        "?s <http://ex/p> ?o . <http://ex/right> <http://ex/q> ?o",
        "?s <http://ex/p> ?o . <http://ex/right> <http://ex/q> ?other FILTER(?o = ?other)",
        "?s <http://ex/p> ?o . <http://ex/right> <http://ex/q> ?other FILTER(sameTerm(?o, ?other))",
    ] {
        let query = format!("SELECT ?s WHERE {{ {pattern} }}");
        let json = answer(configured_mixed(setup, &mapping), &query).await;
        assert_eq!(
            json["results"]["bindings"].as_array().unwrap().len(),
            1,
            "{query}: {json}"
        );
    }
}

#[tokio::test]
async fn column_iri_unique_keys_cannot_prove_rdf_uniqueness() {
    let setup = "CREATE TABLE edges(s TEXT,o TEXT PRIMARY KEY NOT NULL); CREATE TABLE outer_nodes(s TEXT); INSERT INTO edges VALUES('a','AB'),('a','http://example.com/base/AB');";
    let json = answer(
        configured_mixed(setup, &column_iri_mapping()),
        "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o }",
    )
    .await;
    assert_eq!(json["results"]["bindings"][0]["n"]["value"], "1", "{json}");
}

#[tokio::test]
async fn column_iri_reference_atom_resolves_after_native_join_and_keeps_blank_decoding() {
    let mapping = REF_WITNESS
        .replace(
            "rr:template \"http://ex/{s}\"",
            "rr:column \"s\"; rr:termType rr:IRI",
        )
        .replace(
            "rr:template \"http://ex/{label}\"",
            "rr:column \"label\"; rr:termType rr:BlankNode",
        );
    let setup = "CREATE TABLE child(s TEXT,fk TEXT COLLATE NOCASE); CREATE TABLE parent(k TEXT,label); INSERT INTO child VALUES('row','a'),('http://example.com/base/row','A'); INSERT INTO parent VALUES('a','AB'),('A',X'AB');";
    for query in [
        "SELECT ?s ?o WHERE { ?s <http://ex/ref> ?o }",
        "SELECT ?o WHERE { <http://example.com/base/row> <http://ex/ref> ?o }",
        "SELECT ?s ?o WHERE { ?s <http://ex/ref> ?o FILTER(?s = <http://example.com/base/row>) }",
        "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/ref> ?o }",
    ] {
        let json = answer(configured_mixed(setup, &mapping), query).await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{query}: {json}");
        if query.contains("COUNT") {
            assert_eq!(rows[0]["n"]["value"], "1", "{json}");
        } else {
            assert_eq!(rows[0]["o"]["type"], "bnode", "{json}");
        }
    }
}

#[tokio::test]
async fn column_iri_filters_keep_unbound_and_ordering_errors_under_negation() {
    let setup = "CREATE TABLE edges(s TEXT,o TEXT); CREATE TABLE outer_nodes(s TEXT); INSERT INTO edges VALUES('a','AB');";
    for expression in [
        "?o < <http://ex/z>",
        "!(?o < <http://ex/z>)",
        "!sameTerm(?missing, <http://ex/z>)",
    ] {
        let query = format!("SELECT ?s WHERE {{ ?s <http://ex/p> ?o OPTIONAL {{ ?s <http://ex/p> ?missing FILTER(?missing = <http://ex/absent>) }} FILTER({expression}) }}");
        let json = answer(configured_mixed(setup, &column_iri_mapping()), &query).await;
        assert!(
            json["results"]["bindings"].as_array().unwrap().is_empty(),
            "{query}: {json}"
        );
    }
}

#[tokio::test]
async fn literal_natural_constant_uses_declared_datatype() {
    let mapping = MAP.replace(
        "rr:template \"http://ex/n/{o}\"; rr:termType rr:IRI",
        "rr:column \"o\"",
    );
    for kind in ["INTEGER", "BIGINT"] {
        let setup = format!("CREATE TABLE edges(s TEXT,o {kind}); CREATE TABLE outer_nodes(s TEXT); INSERT INTO edges VALUES('a',1);");
        for (literal, count) in [
            ("1", 1),
            ("\"1\"", 0),
            ("\"1\"^^<http://www.w3.org/2001/XMLSchema#double>", 0),
        ] {
            let query = format!("SELECT ?s WHERE {{ ?s <http://ex/p> {literal} }}");
            let json = answer(configured_mixed(&setup, &mapping), &query).await;
            assert_eq!(
                json["results"]["bindings"].as_array().unwrap().len(),
                count,
                "{kind}: {query}: {json}"
            );
        }
    }
}

#[tokio::test]
async fn explicit_literal_identity_preserves_signed_zero_terms_before_count() {
    for query in [
        "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
        "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://ex/p> ?o }",
    ] {
        let json = answer(configured_mixed(ZERO, &literal_mapping()), query).await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        if query.contains("COUNT") {
            assert_eq!(rows[0]["n"]["value"], "2", "{json}");
        } else {
            let mut values: Vec<_> = rows
                .iter()
                .map(|r| r["o"]["value"].as_str().unwrap())
                .collect();
            values.sort();
            assert_eq!(values, vec!["-0", "0"], "{json}");
        }
    }
}

#[tokio::test]
async fn literal_bgp_identity_is_not_numeric_filter_equality() {
    let mapping = format!(
        r#"{}
<#right> rr:logicalTable [rr:tableName "right_values"];
 rr:subject <http://ex/right>;
 rr:predicateObjectMap [rr:predicate <http://ex/q>;
 rr:objectMap [rr:column "o"; rr:datatype <http://www.w3.org/2001/XMLSchema#double>]]."#,
        literal_mapping()
    );
    let setup =
        format!("{ZERO} CREATE TABLE right_values(o); INSERT INTO right_values VALUES (0.0);");
    for (pattern, expected) in [
        ("?s <http://ex/p> ?o . <http://ex/right> <http://ex/q> ?o", vec!["0"]),
        ("?s <http://ex/p> ?o FILTER(?o = 0)", vec!["-0", "0"]),
        ("?s <http://ex/p> ?o FILTER(0 = ?o)", vec!["-0", "0"]),
        ("?s <http://ex/p> ?o . <http://ex/right> <http://ex/q> ?x FILTER(?o = ?x)", vec!["-0", "0"]),
        ("?s <http://ex/p> ?o . <http://ex/right> <http://ex/q> ?x FILTER(?x = ?o)", vec!["-0", "0"]),
        ("?s <http://ex/p> ?o FILTER(sameTerm(?o, \"-0\"^^<http://www.w3.org/2001/XMLSchema#double>))", vec!["-0"]),
    ] {
        let query = format!("SELECT ?o WHERE {{ {pattern} }}");
        let json = answer(configured_mixed(&setup, &mapping), &query).await;
        let mut values: Vec<_> = json["results"]["bindings"].as_array().unwrap().iter().map(|r| r["o"]["value"].as_str().unwrap()).collect();
        values.sort();
        assert_eq!(values, expected, "{query}: {json}");
    }
}

#[tokio::test]
async fn literal_constant_constraints_survive_dedup_without_a_projected_object() {
    for value in ["0", "-0"] {
        let query = format!("SELECT ?s WHERE {{ ?s <http://ex/p> \"{value}\"^^<http://www.w3.org/2001/XMLSchema#double> }}");
        let json = answer(configured_mixed(ZERO, &literal_mapping()), &query).await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{query}: {json}");
        assert_eq!(rows[0]["s"]["value"], "http://ex/n/a%20");
    }
}

#[tokio::test]
async fn literal_identity_checks_datatype_language_and_unbound_error() {
    for (spec, matching, different) in [
        (
            "rr:datatype <http://www.w3.org/2001/XMLSchema#integer>",
            "\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>",
            "\"1\"^^<http://www.w3.org/2001/XMLSchema#double>",
        ),
        ("rr:language \"en\"", "\"1\"@en", "\"1\"@fr"),
    ] {
        let mapping = MAP.replace(
            "rr:template \"http://ex/n/{o}\"; rr:termType rr:IRI",
            &format!("rr:column \"o\"; {spec}"),
        );
        let setup = "CREATE TABLE edges(s TEXT,o TEXT); CREATE TABLE outer_nodes(s TEXT); INSERT INTO edges VALUES('a','1');";
        for (pattern, count) in [
            (format!("?s <http://ex/p> {matching}"), 1),
            (format!("?s <http://ex/p> {different}"), 0),
            (format!("?s <http://ex/p> ?o FILTER(sameTerm(?o, {matching}))"), 1),
            (format!("?s <http://ex/p> ?o FILTER(sameTerm(?o, {different}))"), 0),
            (format!("?s <http://ex/p> ?o OPTIONAL {{ ?s <http://ex/p> ?missing FILTER(sameTerm(?missing, {different})) }} FILTER(!sameTerm(?missing, {different}))"), 0),
        ] {
            let query = format!("SELECT ?s WHERE {{ {pattern} }}");
            let json = answer(configured_mixed(setup, &mapping), &query).await;
            assert_eq!(json["results"]["bindings"].as_array().unwrap().len(), count, "{query}: {json}");
        }
    }
}

#[tokio::test]
async fn literal_numeric_lexicals_compare_values_without_losing_iri_dual_use() {
    for (datatype, value, expression, count) in [
        ("integer", "9007199254740993", "?o > 9007199254740992", 1),
        (
            "decimal",
            "1.000000000000000002",
            "?o > 1.000000000000000001",
            1,
        ),
        ("double", "NaN", "?o = ?o", 0),
        ("double", "NaN", "?o != 0", 1),
        ("double", "invalid", "?o > 0", 0),
        (
            "integer",
            "10",
            "?o > \"9\"^^<http://www.w3.org/2001/XMLSchema#int>",
            1,
        ),
    ] {
        let mapping = literal_mapping().replace("#double>", &format!("#{datatype}>"));
        let setup = format!("CREATE TABLE edges(s TEXT,o TEXT); CREATE TABLE outer_nodes(s TEXT); INSERT INTO edges VALUES('a','{value}');");
        let query = format!("SELECT ?o WHERE {{ ?s <http://ex/p> ?o FILTER({expression}) }}");
        // The derived-integer fallback's prior SQL affinity is numeric, not TEXT.
        let setup = if expression.contains("#int>") {
            setup.replace("o TEXT", "o INTEGER")
        } else {
            setup
        };
        let json = answer(configured_mixed(&setup, &mapping), &query).await;
        assert_eq!(
            json["results"]["bindings"].as_array().unwrap().len(),
            count,
            "{query}: {json}"
        );
    }
    let mapping = literal_mapping().replace("http://ex/n/{s}", "http://ex/n/{o}");
    for query in [
        "SELECT ?o WHERE { ?s <http://ex/p> ?o . ?s <http://ex/p> ?other }",
        "SELECT ?o WHERE { ?s <http://ex/p> ?o FILTER(?o = 0) }",
    ] {
        let json = answer(configured_mixed(ZERO, &mapping), query).await;
        let rows = json["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "{query}: {json}");
        assert_ne!(rows[0]["o"]["value"], rows[1]["o"]["value"]);
    }
}

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
