use super::*;
use sf_serve::{
    PortableRowPolicy, PortableRowRule, ProvisionedBearerAdmission, ProvisionedBearerSubject,
};

#[tokio::test]
async fn unsupported_origins_and_shape_bounds_reject_without_querying_source() {
    let query = "SELECT ?name WHERE { ?s <http://example.test/name> ?name }";
    let second = MAPPING.replace("http://example.test/People", "http://example.test/Second");
    let multiple = format!("{MAPPING}\n{second}");
    let referenced = MAPPING.replace("rr:column \"name\"", "rr:parentTriplesMap <http://example.test/People> ; rr:joinCondition [ rr:child \"id\" ; rr:parent \"id\" ]");
    for mapping in [multiple, referenced] {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE people(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO people VALUES(1,'Alice');").unwrap();
        let mut cfg = support::serve_config(Backend::sqlite(conn), &mapping);
        cfg.set_query_admission(QueryAdmission::Bearer(
            BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
        ));
        // Source work would instead produce a policy failure. Eligibility must
        // reject first without spending even one source-work unit.
        cfg.query_limits = sf_core::query_control::QueryLimits::new(10000, 0, 10000, 100000);
        let response = router(Arc::new(cfg)).oneshot(request(query)).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }
    let query = format!(
        "SELECT ?name WHERE {{ {} }}",
        "?s <http://example.test/name> ?name . ".repeat(257)
    );
    assert_eq!(
        router(Arc::new(config()))
            .oneshot(request(&query))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_IMPLEMENTED
    );
    let mut cfg = config();
    cfg.query_limits = sf_core::query_control::QueryLimits::new(0, 10000, 10000, 100000);
    assert_eq!(
        router(Arc::new(cfg))
            .oneshot(request(
                "SELECT ?n WHERE { ?s <http://example.test/name> ?n }"
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn union_preserves_distinct_bag_occurrences_and_same_origin() {
    let rows = records(
        Arc::new(config()),
        "SELECT ?name WHERE {
        { ?s <http://example.test/name> ?name } UNION { ?s <http://example.test/name> ?name }
    }",
    )
    .await;
    assert_eq!(rows.last().unwrap()["solutions"], 6);
    let identifiers: std::collections::BTreeSet<_> = rows[1..7]
        .iter()
        .map(|row| row["provenance"]["@id"].as_str().unwrap())
        .collect();
    assert_eq!(identifiers.len(), 6);
}

#[tokio::test]
async fn saturation_keeps_the_authored_mapping_identity() {
    let mapping = MAPPING.replace(
        "rr:subjectMap [",
        "rr:subjectMap [ rr:class <http://example.test/Child> ;",
    );
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE people(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO people VALUES(1,'Alice');").unwrap();
    let ontology = sf_serve::SemanticOntology::from_turtle(
        r#"
        @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        @prefix owl: <http://www.w3.org/2002/07/owl#> .
        <http://example.test/Child> a owl:Class ; rdfs:subClassOf <http://example.test/Parent> .
        <http://example.test/Parent> a owl:Class .
        <http://example.test/name> a rdf:Property .
    "#,
    )
    .unwrap();
    let source = sf_serve::IntrospectedSource::observe_sqlite(Backend::sqlite(conn)).unwrap();
    let mut cfg = ServeConfig::from_authored_r2rml(source, &mapping, ontology).unwrap();
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    let rows = records(
        Arc::new(cfg),
        "SELECT ?s WHERE { ?s a <http://example.test/Parent> }",
    )
    .await;
    assert_eq!(rows[0]["mappingId"], "http://example.test/People");
    assert_eq!(rows.last().unwrap()["solutions"], 1);
}

#[tokio::test]
async fn provenance_does_not_bypass_portable_row_policy_or_expose_identity() {
    let second = "test-only-lineage-other-credential-987654321";
    let mut cfg = config();
    let subject = |id: &str, token: &str, value: &str| {
        ProvisionedBearerSubject::portable_rows(
            id,
            token,
            PortableRowPolicy::new(vec![
                PortableRowRule::new(0, "people", "name", value).unwrap()
            ])
            .unwrap(),
        )
        .unwrap()
    };
    cfg.set_query_admission(QueryAdmission::ProvisionedBearers(
        ProvisionedBearerAdmission::new(vec![
            subject("private-subject-one", TOKEN, "Alice"),
            subject("private-subject-two", second, "Bob"),
        ])
        .unwrap(),
    ));
    let app = router(Arc::new(cfg));
    let mut policies = vec![];
    for (token, present, absent) in [
        (TOKEN, "Alice", "Bob"),
        (second, "Bob", "Alice"),
        (TOKEN, "Alice", "Bob"),
    ] {
        let mut req = request("SELECT ?name WHERE { ?s <http://example.test/name> ?name }");
        req.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        let response = app.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains(present));
        for forbidden in [
            absent,
            TOKEN,
            second,
            "private-subject-one",
            "private-subject-two",
        ] {
            assert!(!text.contains(forbidden));
        }
        let header: Value = serde_json::from_str(text.split('\u{1e}').nth(1).unwrap()).unwrap();
        policies.push(header["policy"].clone());
    }
    // One configured policy registry, not a public identifier of each subject.
    assert!(policies.windows(2).all(|pair| pair[0] == pair[1]));
}
