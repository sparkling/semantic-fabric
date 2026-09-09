use super::*;
#[path = "federated_join_lineage_tests.rs"]
mod lineage_tests;

const JOIN: &str = "SELECT ?left ?right WHERE { ?left <http://example.test/left> ?key . ?right <http://example.test/right> ?key }";

#[tokio::test]
async fn public_bag_equals_independently_materialized_graph_oracle() {
    use oxrdf::{Dataset, GraphName, Literal, NamedNode, Quad};
    use spareval::{QueryEvaluator, QueryResults};
    let data = [
        vec![("l-A", "A"), ("l-a", "a"), ("l-A", "A"), ("l-other", "A")],
        vec![("r", "A"), ("r", "a"), ("r", "A"), ("r-other", "absent")],
    ];
    let (cfg, _, _files) = joined_config([&data[0], &data[1]], LITERAL, "TEXT COLLATE NOCASE");
    let cfg = Arc::new(cfg);
    let mut graph = Dataset::new();
    for (index, rows) in data.iter().enumerate() {
        let predicate = NamedNode::new(format!(
            "http://example.test/{}",
            if index == 0 { "left" } else { "right" }
        ))
        .unwrap();
        for (id, key) in rows {
            graph.insert(&Quad::new(
                NamedNode::new(format!("http://example.test/item/{id}")).unwrap(),
                predicate.clone(),
                Literal::new_simple_literal(*key),
                GraphName::DefaultGraph,
            ));
        }
    }
    for query in [JOIN, "SELECT ?left WHERE { ?right <http://example.test/right> ?key . ?left <http://example.test/left> ?key }"] {
        let parsed = spargebra::SparqlParser::new().parse_query(query).unwrap();
        let QueryResults::Solutions(rows) = QueryEvaluator::new().prepare(&parsed).execute(&graph).unwrap() else { panic!("SELECT oracle"); };
        let mut expected: Vec<Vec<String>> = rows.map(|row| row.unwrap().iter().map(|(_, term)| term.to_string()).collect()).collect();
        let answer = json(cfg.clone(), query).await;
        let names = answer["head"]["vars"].as_array().unwrap();
        let mut actual: Vec<Vec<String>> = answer["results"]["bindings"].as_array().unwrap().iter().map(|row| names.iter().map(|name| format!("<{}>", row[name.as_str().unwrap()]["value"].as_str().unwrap())).collect()).collect();
        expected.sort(); actual.sort();
        assert_eq!(actual, expected, "{query}");
    }
}

#[tokio::test]
async fn duplicate_source_triples_and_nulls_do_not_invent_join_multiplicity() {
    let (cfg, _, files) = joined_config(
        [&[("l", "same"), ("l", "same")], &[("r", "same")]],
        LITERAL,
        "TEXT",
    );
    files[0].execute("INSERT INTO items VALUES ('null-object',NULL),(NULL,'same')");
    files[1].execute("INSERT INTO items VALUES ('null-object',NULL),(NULL,'same')");
    let cfg = Arc::new(cfg);
    let answer = json(cfg.clone(), JOIN).await;
    assert_eq!(answer["results"]["bindings"].as_array().unwrap().len(), 1);
    assert!(json(cfg,
        "SELECT ?left ?right ?absent WHERE { ?left <http://example.test/left> ?key . ?right <http://example.test/right> ?key }").await["results"]["bindings"][0].get("absent").is_none());
}

#[tokio::test]
async fn distinct_rdf_keys_are_not_collapsed_by_source_collation() {
    let (cfg, _, _files) = joined_config(
        [&[("l-A", "A"), ("l-a", "a")], &[("r", "A"), ("r", "a")]],
        LITERAL,
        "TEXT COLLATE NOCASE",
    );
    assert_eq!(
        json(Arc::new(cfg), JOIN).await["results"]["bindings"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn canonical_literal_reducer_does_not_compare_raw_source_spelling() {
    let object = "rr:column \"value\"; rr:datatype <http://www.w3.org/2001/XMLSchema#boolean>";
    let (cfg, _, _files) = joined_config(
        [&[("l", "true")], &[("r", "1"), ("r", "true"), ("no", "0")]],
        object,
        "BOOLEAN",
    );
    let cfg = Arc::new(cfg);
    for query in [JOIN, "SELECT ?left ?right WHERE { ?right <http://example.test/right> ?key . ?left <http://example.test/left> ?key }"] {
        let answer = json(cfg.clone(), query).await;
        let rows = answer["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{query}: {answer}");
        assert_eq!(rows[0]["right"]["value"], "http://example.test/item/r");
    }
}

#[tokio::test]
async fn unsupported_join_shapes_reject_before_source_admission() {
    for query in [
        "SELECT DISTINCT ?left WHERE { ?left <http://example.test/left> ?key . ?right <http://example.test/right> ?key }",
        "SELECT ?left WHERE { ?left <http://example.test/left> ?key OPTIONAL { ?right <http://example.test/right> ?key } }",
        "SELECT ?left WHERE { ?left <http://example.test/left> ?a . ?right <http://example.test/right> ?b }",
        "SELECT ?right WHERE { <http://example.test/item/L> <http://example.test/left> ?key . ?right <http://example.test/right> ?key }",
        "SELECT ?left WHERE { ?left <http://example.test/left> \"same\" . ?left <http://example.test/right> ?key }",
        "SELECT ?key WHERE { ?key <http://example.test/left> ?key . ?right <http://example.test/right> ?key }",
    ] {
        let (cfg,pools,_files) = joined_config([&[("l","same")],&[("r","same")]],LITERAL,"TEXT");
        let io = Arc::new(AtomicUsize::new(0));
        for pool in &pools {
            let io = io.clone();
            pool.set_admission_pending_observer(move || { io.fetch_add(1,Ordering::SeqCst); });
        }
        let response = router(Arc::new(cfg)).oneshot(request(query)).await.unwrap();
        assert_eq!(response.status(),StatusCode::NOT_IMPLEMENTED);
        assert_eq!(io.load(Ordering::SeqCst),0);
    }
}

#[tokio::test]
async fn second_source_timeout_and_mid_execution_work_limit_have_no_success_prefix() {
    let (mut cfg, pools, _files) =
        joined_config([&[("l", "same")], &[("r", "same")]], LITERAL, "TEXT");
    cfg.timeout = Duration::from_millis(100);
    let cfg = Arc::new(cfg);
    let lease = pools[1].pick_owned().acquire().await.unwrap();
    let response = router(cfg.clone()).oneshot(request(JOIN)).await.unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    drop(lease);
    assert_eq!(
        json(cfg, JOIN).await["results"]["bindings"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let (mut cfg, _, _files) = joined_config(
        [&[("l", "same")], &[("r1", "same"), ("r2", "same")]],
        LITERAL,
        "TEXT",
    );
    cfg.query_limits = QueryLimits::new(1_000_000, 10, 100, 1_000_000);
    let response = router(Arc::new(cfg)).oneshot(request(JOIN)).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

fn join_mapping(index: usize, object: &str) -> String {
    let predicate = if index == 0 { "left" } else { "right" };
    format!(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> a rr:TriplesMap ; rr:logicalTable [ rr:tableName "items" ] ;
rr:subjectMap [ rr:template "http://example.test/item/{{id}}" ] ;
rr:predicateObjectMap [ rr:predicate <http://example.test/{predicate}> ; rr:objectMap [ {object} ] ] ."#
    )
}

fn joined_config(
    values: [&[(&str, &str)]; 2],
    object: &str,
    right_type: &str,
) -> (ServeConfig, [SqlitePool; 2], [DbFile; 2]) {
    joined_config_objects(values, [object, object], right_type, false)
}

fn joined_config_objects(
    values: [&[(&str, &str)]; 2],
    objects: [&str; 2],
    right_type: &str,
    blank: bool,
) -> (ServeConfig, [SqlitePool; 2], [DbFile; 2]) {
    let mut sources = Vec::new();
    let mut pools = Vec::new();
    let mut files = Vec::new();
    for (index, values) in values.iter().enumerate() {
        let file = DbFile::new("join", &[]);
        let conn = rusqlite::Connection::open(file.path()).unwrap();
        conn.execute_batch(&format!(
            "DROP TABLE items; CREATE TABLE items(id TEXT, value {});",
            if index == 1 { right_type } else { "TEXT" }
        ))
        .unwrap();
        for (id, value) in *values {
            conn.execute("INSERT INTO items VALUES (?1,?2)", [id, value])
                .unwrap();
        }
        drop(conn);
        let (backend, schema) =
            Backend::sqlite_pool_from_path(file.path().to_str().unwrap(), 1).unwrap();
        let Backend::Sqlite(pool) = &backend else {
            unreachable!()
        };
        pools.push(pool.clone());
        let mut mapping = join_mapping(index, objects[index]);
        if blank {
            mapping = mapping.replace(
                "rr:subjectMap [",
                "rr:subjectMap [ rr:termType rr:BlankNode ;",
            );
        }
        sources.push(RuntimeSource::new(
            IntrospectedSource::unchecked(backend, schema),
            SourceMapping::new(
                SourceId::new(index).unwrap(),
                sf_mapping::parse_r2rml(&mapping).unwrap(),
            ),
        ));
        files.push(file);
    }
    let mut config = ServeConfig::new_federated(
        sources.try_into().unwrap(),
        crate::test_support::ontology(
            &[],
            &["http://example.test/left", "http://example.test/right"],
        ),
    )
    .unwrap();
    config.set_query_admission(crate::QueryAdmission::UnrestrictedDevelopment);
    (
        config,
        pools.try_into().ok().unwrap(),
        files.try_into().ok().unwrap(),
    )
}

#[tokio::test]
async fn language_case_is_equal_but_different_language_is_not() {
    for (language, expected) in [("en", 1), ("fr", 0)] {
        let right = format!(r#"rr:column "value" ; rr:language "{language}""#);
        let (cfg, _, _files) = joined_config_objects(
            [&[("l", "same")], &[("r", "same")]],
            [r#"rr:column "value" ; rr:language "EN""#, &right],
            "TEXT",
            false,
        );
        assert_eq!(
            json(Arc::new(cfg), JOIN).await["results"]["bindings"]
                .as_array()
                .unwrap()
                .len(),
            expected
        );
    }
}

#[tokio::test]
async fn output_blank_nodes_are_scoped_to_their_source() {
    let (cfg, _, _files) = joined_config_objects(
        [&[("same", "key")], &[("same", "key")]],
        [LITERAL, LITERAL],
        "TEXT",
        true,
    );
    let result = json(Arc::new(cfg), JOIN).await;
    let row = &result["results"]["bindings"][0];
    assert_eq!(row["left"]["type"], "bnode");
    assert_eq!(row["right"]["type"], "bnode");
    assert_ne!(row["left"]["value"], row["right"]["value"]);
}

#[tokio::test]
async fn nonmatching_probe_payload_still_consumes_retained_capacity() {
    let large = "z".repeat(8192);
    let (mut cfg, _, _files) =
        joined_config([&[("l", "same")], &[(&large, "same")]], LITERAL, "TEXT");
    cfg.query_limits =
        QueryLimits::new(1_000_000, 1_000_000, 100, 1_000_000).with_max_retained_bytes(4096);
    // Two shared variables: key matches the reducer, subject does not. Even
    // an empty final bag must not let the large right subject bypass admission.
    let query = "SELECT ?s WHERE { ?s <http://example.test/left> ?key . ?s <http://example.test/right> ?key }";
    let response = router(Arc::new(cfg)).oneshot(request(query)).await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}
const LITERAL: &str =
    r#"rr:column "value" ; rr:datatype <http://www.w3.org/2001/XMLSchema#string>"#;

#[tokio::test]
async fn exact_source_sets_normalize_padding_without_deduplicating_results() {
    let (cfg, _, _files) = joined_config(
        [
            &[("l1", "a  "), ("l2", "a  ")],
            &[("r", "a"), ("r", "a "), ("r", "a  ")],
        ],
        LITERAL,
        "CHAR(3)",
    );
    let query = "SELECT ?key WHERE { ?left <http://example.test/left> ?key . ?right <http://example.test/right> ?key }";
    let answer = json(Arc::new(cfg), query).await;
    let rows = answer["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], rows[1]);
}

#[tokio::test]
async fn probe_triple_cap_is_inclusive_and_overflow_never_returns_partial_success() {
    for count in [4096, 4097] {
        let values: Vec<_> = (0..count).map(|i| format!("r{i}")).collect();
        let refs: Vec<_> = values.iter().map(|id| (id.as_str(), "same")).collect();
        let (mut cfg, _, _files) = joined_config([&[("l", "same")], &refs], LITERAL, "TEXT");
        cfg.query_limits = QueryLimits::new(1_000_000, 1_000_000_000, 10_000, 10_000_000)
            .with_max_retained_bytes(100_000_000);
        let response = router(Arc::new(cfg)).oneshot(request(JOIN)).await.unwrap();
        assert_eq!(
            response.status(),
            if count == 4096 {
                StatusCode::OK
            } else {
                StatusCode::TOO_MANY_REQUESTS
            }
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if count == 4096 {
            assert_eq!(
                result["results"]["bindings"].as_array().unwrap().len(),
                count
            );
        } else {
            assert!(result.get("results").is_none());
        }
    }
}

#[tokio::test]
async fn public_join_keeps_hidden_keys_and_three_by_four_bag_multiplicity() {
    let (cfg, _, _files) = joined_config(
        [
            &[("l1", "same"), ("l2", "same"), ("l3", "same")],
            &[
                ("r1", "same"),
                ("r2", "same"),
                ("r3", "same"),
                ("r4", "same"),
            ],
        ],
        LITERAL,
        "TEXT",
    );
    let result = json(Arc::new(cfg), JOIN).await;
    let rows = result["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 12);
    for l in 1..=3 {
        for r in 1..=4 {
            assert_eq!(
                rows.iter()
                    .filter(
                        |row| row["left"]["value"] == format!("http://example.test/item/l{l}")
                            && row["right"]["value"] == format!("http://example.test/item/r{r}")
                    )
                    .count(),
                1
            );
        }
    }
    assert!(rows.iter().all(|row| row.get("key").is_none()));
}

#[tokio::test]
async fn template_keys_decode_escapes_and_sqlite_padding_does_not_drop_matches() {
    for (object, left, right, ty) in [
        (
            r#"rr:template "http://example.test/key/{value}" ; rr:termType rr:IRI"#,
            "a /%雪",
            "a /%雪",
            "TEXT",
        ),
        (LITERAL, "a  ", "a", "CHAR(3)"),
    ] {
        let (cfg, _, _files) = joined_config([&[("l", left)], &[("r", right)]], object, ty);
        assert_eq!(
            json(Arc::new(cfg), JOIN).await["results"]["bindings"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}

#[tokio::test]
async fn sql_collation_false_positives_are_removed_by_exact_rdf_comparison() {
    let (cfg, _, _files) = joined_config(
        [&[("l", "A")], &[("r", "a"), ("r2", "A")]],
        LITERAL,
        "TEXT COLLATE NOCASE",
    );
    let answer = json(Arc::new(cfg), JOIN).await;
    let rows = answer["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["right"]["value"], "http://example.test/item/r2");
}

#[tokio::test]
async fn result_and_serialized_and_retained_limits_fail_before_success_headers() {
    for limits in [
        QueryLimits::new(1_000_000, 1_000_000, 0, 1_000_000),
        QueryLimits::new(1_000_000, 1_000_000, 1, 1),
        QueryLimits::new(1_000_000, 1_000_000, 1, 1_000_000).with_max_retained_bytes(1),
    ] {
        let (mut cfg, _, _files) =
            joined_config([&[("l", "same")], &[("r", "same")]], LITERAL, "TEXT");
        cfg.query_limits = limits;
        let response = router(Arc::new(cfg)).oneshot(request(JOIN)).await.unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(!String::from_utf8_lossy(&body).contains("bindings"));
    }
}

#[tokio::test]
async fn driving_batch_boundary_is_inclusive_and_never_truncates() {
    for count in [0, 128, 129] {
        let values: Vec<_> = (0..count)
            .map(|i| (format!("l{i}"), "same".to_owned()))
            .collect();
        let refs: Vec<_> = values
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let (cfg, _, _files) = joined_config([&refs, &[("r", "same")]], LITERAL, "TEXT");
        let response = router(Arc::new(cfg)).oneshot(request(JOIN)).await.unwrap();
        assert_eq!(
            response.status(),
            if count > 128 {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::OK
            }
        );
        if count <= 128 {
            let data = response.into_body().collect().await.unwrap().to_bytes();
            let json: serde_json::Value = serde_json::from_slice(&data).unwrap();
            assert_eq!(json["results"]["bindings"].as_array().unwrap().len(), count);
        }
    }
}

#[tokio::test]
async fn join_policies_cover_both_sources_and_authentication_precedes_io() {
    use crate::{
        PortableRowPolicy, PortableRowRule, ProvisionedBearerAdmission, ProvisionedBearerSubject,
        QueryAdmission,
    };
    let (mut cfg, pools, _files) = joined_config(
        [&[("l-a", "a"), ("l-b", "b")], &[("r-a", "a"), ("r-b", "b")]],
        LITERAL,
        "TEXT",
    );
    let token = "test-only-join-principal-0123456789";
    cfg.set_query_admission(QueryAdmission::ProvisionedBearers(
        ProvisionedBearerAdmission::new(vec![ProvisionedBearerSubject::portable_rows(
            "a",
            token,
            PortableRowPolicy::new(vec![
                PortableRowRule::new(0, "items", "value", "a").unwrap(),
                PortableRowRule::new(1, "items", "value", "a").unwrap(),
            ])
            .unwrap(),
        )
        .unwrap()])
        .unwrap(),
    ));
    let app = router(Arc::new(cfg));
    let held = pools[0].pick_owned().acquire().await.unwrap();
    assert_eq!(
        app.clone().oneshot(request(JOIN)).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    drop(held);
    let mut req = request(JOIN);
    req.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let data = response.into_body().collect().await.unwrap().to_bytes();
    let result: serde_json::Value = serde_json::from_slice(&data).unwrap();
    assert_eq!(result["results"]["bindings"].as_array().unwrap().len(), 1);
    assert_eq!(
        result["results"]["bindings"][0]["left"]["value"],
        "http://example.test/item/l-a"
    );
}
