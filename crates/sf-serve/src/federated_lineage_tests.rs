use super::*;
use serde_json::Value;

const TOKEN: &str = "test-only-federated-lineage-credential-12345";
#[path = "federated_lineage_control_tests.rs"]
mod control_tests;
fn lineage_request(query: &str) -> Request<Body> {
    let mut request = request(query);
    request
        .headers_mut()
        .insert(header::ACCEPT, crate::lineage::MEDIA_TYPE.parse().unwrap());
    request.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().unwrap(),
    );
    request
}
fn protected_config(values: [&[&str]; 2]) -> (ServeConfig, [SqlitePool; 2], [DbFile; 2]) {
    let (mut cfg, pools, files) = config(
        ["http://example.test/left", "http://example.test/right"],
        values,
    );
    cfg.set_query_admission(crate::QueryAdmission::Bearer(
        crate::BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    (cfg, pools, files)
}
async fn records(cfg: Arc<ServeConfig>, query: &str) -> Vec<Value> {
    let response = router(cfg).oneshot(lineage_request(query)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        crate::lineage::MEDIA_TYPE
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!std::str::from_utf8(&bytes).unwrap().contains(TOKEN));
    parse(&bytes)
}
fn parse(bytes: &[u8]) -> Vec<Value> {
    std::str::from_utf8(bytes)
        .unwrap()
        .split('\u{1e}')
        .skip(1)
        .map(|r| serde_json::from_str(r).unwrap())
        .collect()
}

fn custom_config(maps: [String; 2], ontology: &str) -> (ServeConfig, [SqlitePool; 2], [DbFile; 2]) {
    let files = [
        DbFile::new("custom-left", &["same"]),
        DbFile::new("custom-right", &["same"]),
    ];
    let mut pools = Vec::new();
    let sources = maps
        .into_iter()
        .enumerate()
        .map(|(index, map)| {
            let (backend, schema) =
                Backend::sqlite_pool_from_path(files[index].path().to_str().unwrap(), 1).unwrap();
            let Backend::Sqlite(pool) = &backend else {
                unreachable!()
            };
            pools.push(pool.clone());
            RuntimeSource::new(
                IntrospectedSource::unchecked(backend, schema),
                SourceMapping::new(
                    SourceId::new(index).unwrap(),
                    sf_mapping::parse_r2rml(&map).unwrap(),
                ),
            )
        })
        .collect::<Vec<_>>();
    let mut cfg = ServeConfig::new_federated(
        sources.try_into().ok().unwrap(),
        if ontology.is_empty() {
            crate::test_support::ontology(
                &[],
                &["http://example.test/left", "http://example.test/right"],
            )
        } else {
            crate::SemanticOntology::from_turtle(ontology).unwrap()
        },
    )
    .unwrap();
    cfg.set_query_admission(crate::QueryAdmission::Bearer(
        crate::BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    (cfg, pools.try_into().ok().unwrap(), files)
}

#[tokio::test]
async fn union_standardizes_source_blank_nodes_apart_in_both_media_types() {
    let maps = ["left", "right"].map(|p| {
        mapping(&format!("http://example.test/{p}")).replace(
            "rr:constant <http://example.test/item>",
            "rr:template \"item-{value}\"; rr:termType rr:BlankNode",
        )
    });
    let (mut cfg, _pools, _files) = custom_config(maps, "");
    cfg.set_query_admission(crate::QueryAdmission::UnrestrictedDevelopment);
    let cfg = Arc::new(cfg);
    let ordinary = json(cfg.clone(), UNION_SAME_VAR).await;
    let rows = &ordinary["results"]["bindings"];
    assert_eq!(rows[0]["s"]["type"], "bnode");
    assert_ne!(rows[0]["s"], rows[1]["s"]);
    let lineage = records(cfg.clone(), UNION_SAME_VAR).await;
    for index in 0..2 {
        assert_eq!(
            rows[index]["s"],
            lineage[index + 1]["result"]["results"]["bindings"][0]["s"]
        );
    }
    let reversed = records(cfg, UNION_REVERSED).await;
    assert_eq!(
        rows[0]["s"],
        reversed[2]["result"]["results"]["bindings"][0]["s"]
    );
}

#[tokio::test]
async fn lineage_affinity_uses_the_same_entailed_predicates_as_execution() {
    for inverse in [false, true] {
        let relation = if inverse {
            "http://www.w3.org/2002/07/owl#inverseOf"
        } else {
            "http://www.w3.org/2000/01/rdf-schema#subPropertyOf"
        };
        let ontology = format!("<http://example.test/left> a <http://www.w3.org/1999/02/22-rdf-syntax-ns#Property>; <{relation}> <http://example.test/inferred> . <http://example.test/right> a <http://www.w3.org/1999/02/22-rdf-syntax-ns#Property> .");
        let maps = ["left", "right"].map(|p| {
            mapping(&format!("http://example.test/{p}")).replace(
                "rr:column \"value\"",
                "rr:template \"http://example.test/value/{value}\"; rr:termType rr:IRI",
            )
        });
        let (cfg, _pools, _files) = custom_config(maps, &ontology);
        let arm = if inverse {
            "?value <http://example.test/inferred> ?s"
        } else {
            "?s <http://example.test/inferred> ?value"
        };
        let query = format!("SELECT ?s ?value WHERE {{ {{ {arm} }} UNION {{ ?s <http://example.test/right> ?value }} }}");
        let cfg = Arc::new(cfg);
        let rows = records(cfg.clone(), &query).await;
        assert_eq!(rows.last().unwrap()["solutions"], 2);
        assert_eq!([source(&rows[1]), source(&rows[2])], [0, 1]);
        let mut ordinary = lineage_request(&query);
        ordinary.headers_mut().insert(
            header::ACCEPT,
            "application/sparql-results+json".parse().unwrap(),
        );
        let response = router(cfg).oneshot(ordinary).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "unique entailed ordinary source inverse={inverse}"
        );
        let ordinary: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(
            ordinary["results"]["bindings"],
            serde_json::json!([
                rows[1]["result"]["results"]["bindings"][0],
                rows[2]["result"]["results"]["bindings"][0]
            ])
        );
        // The other source now entails the first arm too. Held pool observers
        // prove ambiguous provenance is rejected before any source admission.
        let ontology = format!(
            "{ontology} <http://example.test/right> <{relation}> <http://example.test/inferred> ."
        );
        let maps = ["left", "right"].map(|p| {
            mapping(&format!("http://example.test/{p}")).replace(
                "rr:column \"value\"",
                "rr:template \"http://example.test/value/{value}\"; rr:termType rr:IRI",
            )
        });
        let (cfg, pools, _files) = custom_config(maps, &ontology);
        let snapshot = cfg.runtime_lease().unwrap();
        let bindings = [0, 1].map(|id| {
            snapshot
                .snapshot()
                .registry()
                .binding(SourceId::new(id).unwrap())
                .unwrap()
                .compiler()
        });
        assert!(
            sf_sparql::federation::compile_source_affine_union_lineage(
                &query,
                bindings,
                &UncontrolledQueryControl
            )
            .is_err(),
            "ambiguous inverse={inverse}"
        );
        assert!(
            sf_sparql::federation::compile_source_affine_union(
                &query,
                bindings,
                &UncontrolledQueryControl
            )
            .is_err(),
            "ordinary ambiguity inverse={inverse}"
        );
        let _left = pools[0].pick_owned().acquire().await.unwrap();
        let _right = pools[1].pick_owned().acquire().await.unwrap();
        let io = Arc::new(AtomicUsize::new(0));
        for pool in pools {
            let io = io.clone();
            pool.set_admission_pending_observer(move || {
                io.fetch_add(1, Ordering::SeqCst);
            });
        }
        let cfg = Arc::new(cfg);
        for format in [
            crate::lineage::MEDIA_TYPE,
            "application/sparql-results+json",
        ] {
            let mut request = lineage_request(&query);
            request
                .headers_mut()
                .insert(header::ACCEPT, format.parse().unwrap());
            let response =
                tokio::time::timeout(Duration::from_secs(1), router(cfg.clone()).oneshot(request))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
            assert_eq!(io.load(Ordering::SeqCst), 0);
        }
    }
}
fn source(row: &Value) -> usize {
    let sources: Vec<_> = row["provenance"]["@graph"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["sf:sourceId"].as_u64())
        .collect();
    assert_eq!(sources.len(), 1);
    sources[0] as usize
}

#[tokio::test]
async fn direct_owner_with_entailed_competing_source_rejects_in_both_media_types() {
    let maps = ["left", "right"].map(|p| mapping(&format!("http://example.test/{p}")));
    let (cfg, pools, _files) = custom_config(maps, "<http://example.test/left> a <http://www.w3.org/1999/02/22-rdf-syntax-ns#Property> . <http://example.test/right> a <http://www.w3.org/1999/02/22-rdf-syntax-ns#Property>; <http://www.w3.org/2000/01/rdf-schema#subPropertyOf> <http://example.test/left> .");
    let _left = pools[0].pick_owned().acquire().await.unwrap();
    let _right = pools[1].pick_owned().acquire().await.unwrap();
    let cfg = Arc::new(cfg);
    for format in [
        crate::lineage::MEDIA_TYPE,
        "application/sparql-results+json",
    ] {
        let mut request = lineage_request(UNION_SAME_VAR);
        request
            .headers_mut()
            .insert(header::ACCEPT, format.parse().unwrap());
        let response =
            tokio::time::timeout(Duration::from_secs(1), router(cfg.clone()).oneshot(request))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }
}

#[tokio::test]
async fn public_union_lineage_preserves_bags_unbound_and_actual_branch_sources() {
    let (cfg, _pools, _files) = protected_config([&["same", "same"], &["same"]]);
    let cfg = Arc::new(cfg);
    for (query, expected) in [
        (UNION_SAME_VAR, [0, 1]),
        (UNION_DISTINCT_VARS, [0, 1]),
        (UNION_REVERSED, [1, 0]),
    ] {
        let rows = records(cfg.clone(), query).await;
        assert_eq!(rows[0]["profile"], "bounded-federated-union-lineage-v1");
        assert_eq!(rows[0]["sources"].as_array().unwrap().len(), 2);
        assert_eq!(rows.last().unwrap()["solutions"], 2);
        assert_eq!([source(&rows[1]), source(&rows[2])], expected);
        assert_ne!(rows[1]["provenance"]["@id"], rows[2]["provenance"]["@id"]);
        for (ordinal, row) in rows[1..3].iter().enumerate() {
            assert_eq!(row["ordinal"], ordinal);
            let binding = &row["result"]["results"]["bindings"][0];
            assert_eq!(binding["s"]["value"], "http://example.test/item");
            if query != UNION_SAME_VAR {
                assert!(binding.get("left").is_some() != binding.get("right").is_some());
            }
            let quads = oxjsonld::JsonLdParser::new()
                .for_slice(row["provenance"].to_string().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(!quads.is_empty());
        }
    }
}

#[tokio::test]
async fn public_union_lineage_merges_actual_maps_within_an_arm_not_between_sources() {
    let files = [
        DbFile::new("multi-left", &["same"]),
        DbFile::new("multi-right", &["same"]),
    ];
    let sources = ["http://example.test/left", "http://example.test/right"]
        .into_iter()
        .enumerate()
        .map(|(index, predicate)| {
            let (backend, schema) =
                Backend::sqlite_pool_from_path(files[index].path().to_str().unwrap(), 1).unwrap();
            let maps = ["a", "b"]
                .map(|name| mapping(predicate).replace("<#items>", &format!("<urn:map:{name}>")))
                .join("\n");
            RuntimeSource::new(
                IntrospectedSource::unchecked(backend, schema),
                SourceMapping::new(
                    SourceId::new(index).unwrap(),
                    sf_mapping::parse_r2rml(&maps).unwrap(),
                ),
            )
        })
        .collect::<Vec<_>>();
    let mut cfg = ServeConfig::new_federated(
        sources.try_into().ok().unwrap(),
        crate::test_support::ontology(
            &[],
            &["http://example.test/left", "http://example.test/right"],
        ),
    )
    .unwrap();
    cfg.set_query_admission(crate::QueryAdmission::Bearer(
        crate::BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
    let rows = records(Arc::new(cfg), UNION_SAME_VAR).await;
    assert_eq!(rows.last().unwrap()["solutions"], 2);
    for row in &rows[1..3] {
        let mut maps = row["provenance"]["@graph"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|n| n["sf:mappingId"].as_str())
            .collect::<Vec<_>>();
        maps.sort();
        assert_eq!(maps, ["urn:map:a", "urn:map:b"]);
    }
    assert_eq!([source(&rows[1]), source(&rows[2])], [0, 1]);
}
