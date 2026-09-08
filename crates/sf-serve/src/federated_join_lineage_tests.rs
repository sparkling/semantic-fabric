use super::*;
use serde_json::{json, Value};

const TOKEN: &str = "test-only-join-lineage-credential-123456";
const REVERSED: &str = "SELECT ?left ?right WHERE { ?right <http://example.test/right> ?key . ?left <http://example.test/left> ?key }";

fn lineage_request(query: &str) -> Request<Body> {
    let mut req = request(query);
    req.headers_mut()
        .insert(header::ACCEPT, crate::lineage::MEDIA_TYPE.parse().unwrap());
    req.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().unwrap(),
    );
    req
}
fn protect(cfg: &mut ServeConfig) {
    cfg.set_query_admission(crate::QueryAdmission::Bearer(
        crate::BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ));
}
async fn records(cfg: Arc<ServeConfig>, query: &str) -> Vec<Value> {
    let response = router(cfg).oneshot(lineage_request(query)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        crate::lineage::MEDIA_TYPE
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(!text.contains(TOKEN));
    assert!(!text.contains("private-join-key"));
    text.split('\u{1e}')
        .skip(1)
        .map(|r| serde_json::from_str(r).unwrap())
        .collect()
}
fn assert_origins(header: &Value, row: &Value) {
    let graph = row["provenance"]["@graph"].as_array().unwrap();
    let used = graph[0]["prov:used"].as_array().unwrap();
    for source in header["sources"].as_array().unwrap() {
        assert!(used.contains(&json!({"@id":source["source"]})));
        assert!(used.contains(&json!({"@id":source["mappingDocument"]})));
        let map = graph
            .iter()
            .find(|n| {
                n["sf:mappingId"] == source["mappingCatalog"][0]
                    && n["sf:source"]["@id"] == source["source"]
                    && used.contains(&json!({"@id":n["@id"]}))
            })
            .unwrap();
        assert!(map["@id"]
            .as_str()
            .unwrap()
            .starts_with("urn:semantic-fabric:mapping-entry:"));
    }
    for (field, key) in [
        ("sf:snapshot", "snapshot"),
        ("sf:logicalPlan", "logicalPlan"),
        ("sf:policy", "policy"),
    ] {
        assert_eq!(graph[0][field]["@id"], header[key]);
    }
    assert_eq!(
        graph
            .iter()
            .filter(|n| n.get("sf:sourceId").is_some())
            .count(),
        2
    );
    assert_eq!(
        graph
            .iter()
            .filter(|n| n.get("sf:mappingId").is_some())
            .count(),
        2
    );
    assert_eq!(used.len(), 6);
}

#[tokio::test]
async fn joined_lineage_retains_bags_and_both_actual_origins_after_projection() {
    let (mut cfg, _, _files) = joined_config(
        [
            &[
                ("l1", "private-join-key"),
                ("l2", "private-join-key"),
                ("l2", "private-join-key"),
            ],
            &[
                ("r1", "private-join-key"),
                ("r2", "private-join-key"),
                ("r3", "private-join-key"),
                ("no", "different"),
            ],
        ],
        LITERAL,
        "TEXT",
    );
    protect(&mut cfg);
    let cfg = Arc::new(cfg);
    for query in [JOIN, REVERSED, "SELECT ?left ?absent WHERE { ?left <http://example.test/left> ?key . ?right <http://example.test/right> ?key }"] {
        let records = records(cfg.clone(), query).await;
        let header = &records[0];
        assert_eq!(header["profile"], "bounded-federated-join-lineage-v1");
        assert_eq!(header["rowKeys"], "not-provided");
        assert_eq!(header["sources"].as_array().unwrap().len(), 2);
        assert_eq!(records.last().unwrap(), &json!({"type":"complete","solutions":6}));
        let mut ordinary_request = lineage_request(query);
        ordinary_request.headers_mut().insert(header::ACCEPT, "application/sparql-results+json".parse().unwrap());
        let response = router(cfg.clone()).oneshot(ordinary_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let ordinary: Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let mut expected: Vec<_> = ordinary["results"]["bindings"].as_array().unwrap().iter().map(Value::to_string).collect();
        let mut actual = Vec::new();
        for (ordinal, row) in records[1..7].iter().enumerate() {
            assert_eq!(row["ordinal"], ordinal);
            assert_origins(header, row);
            actual.push(row["result"]["results"]["bindings"][0].to_string());
        }
        expected.sort(); actual.sort(); assert_eq!(actual, expected);
    }
}

#[tokio::test]
async fn empty_join_lineage_has_catalog_but_no_claimed_contribution() {
    let (mut cfg, _, _files) = joined_config([&[("l", "a")], &[("r", "b")]], LITERAL, "TEXT");
    protect(&mut cfg);
    let rows = records(Arc::new(cfg), JOIN).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1], json!({"type":"complete","solutions":0}));
}

#[tokio::test]
async fn joined_lineage_pins_both_old_sources_while_activation_changes_admission() {
    let (mut cfg, pools, _files) = joined_config(
        [&[("old-left", "same")], &[("old-right", "same")]],
        LITERAL,
        "TEXT",
    );
    protect(&mut cfg);
    let cfg = Arc::new(cfg);
    let baseline = records(cfg.clone(), JOIN).await;
    let held = pools[1].pick_owned().acquire().await.unwrap();
    let pending = Arc::new(tokio::sync::Notify::new());
    let observed = pending.clone();
    pools[1].set_admission_pending_observer(move || observed.notify_one());
    let request_cfg = cfg.clone();
    let old = tokio::spawn(async move { records(request_cfg, JOIN).await });
    tokio::time::timeout(Duration::from_secs(2), pending.notified())
        .await
        .unwrap();
    let new_files = [
        DbFile::new("new-join-left", &[]),
        DbFile::new("new-join-right", &[]),
    ];
    let sources: Vec<_> = new_files.iter().enumerate().map(|(index,file)| {
        file.execute("DROP TABLE items; CREATE TABLE items(id TEXT,value TEXT); INSERT INTO items VALUES ('new','same');");
        let (backend,schema) = Backend::sqlite_pool_from_path(file.path().to_str().unwrap(),1).unwrap();
        RuntimeSource::new(IntrospectedSource::unchecked(backend,schema),SourceMapping::new(SourceId::new(index).unwrap(),sf_mapping::parse_r2rml(&join_mapping(index,LITERAL)).unwrap()))
    }).collect();
    cfg.activate_snapshot(
        cfg.runtime_readiness().unwrap(),
        RuntimeSnapshot::new(
            Epoch(1),
            crate::test_support::ontology(
                &[],
                &["http://example.test/left", "http://example.test/right"],
            ),
            sources,
        )
        .unwrap(),
    )
    .unwrap();
    drop(held);
    let old = old.await.unwrap();
    assert_eq!(old[0]["sources"], baseline[0]["sources"]);
    assert_eq!(old[0]["snapshot"], baseline[0]["snapshot"]);
    assert_eq!(
        old[1]["result"]["results"]["bindings"][0]["right"]["value"],
        "http://example.test/item/old-right"
    );
    assert_origins(&old[0], &old[1]);
    let new = records(cfg, JOIN).await;
    assert_ne!(old[0]["sources"], new[0]["sources"]);
    assert_ne!(old[0]["snapshot"], new[0]["snapshot"]);
    assert_eq!(
        new[1]["result"]["results"]["bindings"][0]["right"]["value"],
        "http://example.test/item/new"
    );
    assert_origins(&new[0], &new[1]);
}

#[tokio::test]
async fn wider_join_lineage_rejects_before_source_admission() {
    let (mut cfg, pools, _files) =
        joined_config([&[("l", "same")], &[("r", "same")]], LITERAL, "TEXT");
    protect(&mut cfg);
    let count = Arc::new(AtomicUsize::new(0));
    for pool in pools {
        let count = count.clone();
        pool.set_admission_pending_observer(move || {
            count.fetch_add(1, Ordering::SeqCst);
        });
    }
    let cfg = Arc::new(cfg);
    for query in [
        "SELECT DISTINCT ?left WHERE { ?left <http://example.test/left> ?key . ?right <http://example.test/right> ?key }",
        "SELECT ?left WHERE { ?left <http://example.test/left> ?key OPTIONAL { ?right <http://example.test/right> ?key } }",
        "SELECT ?left WHERE { ?left <http://example.test/left> ?a . ?right <http://example.test/right> ?b }",
    ] {
        assert_eq!(router(cfg.clone()).oneshot(lineage_request(query)).await.unwrap().status(),StatusCode::NOT_IMPLEMENTED);
    }
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn joined_lineage_zero_compiler_work_rejects_before_source_admission() {
    let (mut cfg, pools, _files) =
        joined_config([&[("l", "same")], &[("r", "same")]], LITERAL, "TEXT");
    protect(&mut cfg);
    cfg.query_limits = QueryLimits::new(0, u64::MAX, u64::MAX, u64::MAX);
    let count = Arc::new(AtomicUsize::new(0));
    for pool in pools {
        let count = count.clone();
        pool.set_admission_pending_observer(move || {
            count.fetch_add(1, Ordering::SeqCst);
        });
    }
    let response = router(Arc::new(cfg))
        .oneshot(lineage_request(JOIN))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn joined_lineage_charges_exact_output_and_fails_before_headers() {
    let fixture = || {
        let (mut cfg, pools, files) = joined_config(
            [&[("l", "same")], &[("r1", "same"), ("r2", "same")]],
            LITERAL,
            "TEXT",
        );
        protect(&mut cfg);
        (cfg, pools, files)
    };
    let (cfg, _, _files) = fixture();
    let baseline = router(Arc::new(cfg))
        .oneshot(lineage_request(JOIN))
        .await
        .unwrap();
    assert_eq!(baseline.status(), StatusCode::OK);
    let size = baseline
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .len() as u64;
    for (results, bytes, retained, ok) in [
        (2, size, u64::MAX, true),
        (2, size - 1, u64::MAX, false),
        (1, u64::MAX, u64::MAX, false),
        (2, u64::MAX, 1, false),
        (2, 1, u64::MAX, false),
    ] {
        let (mut cfg, pools, _files) = fixture();
        cfg.query_limits =
            QueryLimits::new(u64::MAX, u64::MAX, results, bytes).with_max_retained_bytes(retained);
        let response = router(Arc::new(cfg))
            .oneshot(lineage_request(JOIN))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if ok {
                StatusCode::OK
            } else {
                StatusCode::TOO_MANY_REQUESTS
            }
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        if !ok {
            assert!(!body.contains(&0x1e));
        }
        for pool in pools {
            drop(
                tokio::time::timeout(Duration::from_secs(1), pool.pick_owned().acquire())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
    }
}

#[tokio::test]
async fn joined_lineage_second_admission_failure_is_pre200_and_recovers() {
    let (mut cfg, pools, _files) =
        joined_config([&[("l", "same")], &[("r", "same")]], LITERAL, "TEXT");
    protect(&mut cfg);
    cfg.timeout = Duration::from_millis(100);
    let cfg = Arc::new(cfg);
    let held = pools[1].pick_owned().acquire().await.unwrap();
    let response = router(cfg.clone())
        .oneshot(lineage_request(JOIN))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert!(!response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .contains(&0x1e));
    drop(held);
    assert_eq!(records(cfg, JOIN).await.last().unwrap()["solutions"], 1);
}

#[tokio::test]
async fn joined_lineage_retains_exact_blank_scope_and_collation() {
    let (mut cfg, _, _files) = joined_config_objects(
        [&[("same", "A")], &[("same", "A"), ("false", "a")]],
        [LITERAL, LITERAL],
        "TEXT COLLATE NOCASE",
        true,
    );
    protect(&mut cfg);
    let records = records(Arc::new(cfg), JOIN).await;
    assert_eq!(records.last().unwrap()["solutions"], 1);
    let row = &records[1]["result"]["results"]["bindings"][0];
    assert_eq!(row["left"]["type"], "bnode");
    assert_eq!(row["right"]["type"], "bnode");
    assert_ne!(row["left"]["value"], row["right"]["value"]);
    assert_origins(&records[0], &records[1]);
}

#[tokio::test]
async fn joined_lineage_policies_filter_both_contributors_and_auth_precedes_io() {
    use crate::{
        PortableRowPolicy, PortableRowRule, ProvisionedBearerAdmission, ProvisionedBearerSubject,
        QueryAdmission,
    };
    let (mut cfg, pools, _files) = joined_config(
        [&[("l-a", "a"), ("l-b", "b")], &[("r-a", "a"), ("r-b", "b")]],
        LITERAL,
        "TEXT",
    );
    let second = "test-only-second-join-lineage-credential-123456";
    let subject = |id: &str, token: &str| {
        ProvisionedBearerSubject::portable_rows(
            id,
            token,
            PortableRowPolicy::new(vec![
                PortableRowRule::new(0, "items", "value", id).unwrap(),
                PortableRowRule::new(1, "items", "value", id).unwrap(),
            ])
            .unwrap(),
        )
        .unwrap()
    };
    cfg.set_query_admission(QueryAdmission::ProvisionedBearers(
        ProvisionedBearerAdmission::new(vec![subject("a", TOKEN), subject("b", second)]).unwrap(),
    ));
    let cfg = Arc::new(cfg);
    let held = pools[0].pick_owned().acquire().await.unwrap();
    let mut req = lineage_request(JOIN);
    req.headers_mut().remove(header::AUTHORIZATION);
    assert_eq!(
        router(cfg.clone()).oneshot(req).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    drop(held);
    for (token, id, other) in [(TOKEN, "a", "b"), (second, "b", "a"), (TOKEN, "a", "b")] {
        let mut req = lineage_request(REVERSED);
        req.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        let response = router(cfg.clone()).oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            !text.contains(token)
                && !text.contains(&format!("item/l-{other}"))
                && !text.contains(&format!("item/r-{other}"))
        );
        let rows: Vec<Value> = text
            .split('\u{1e}')
            .skip(1)
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(rows.last().unwrap()["solutions"], 1);
        assert_eq!(
            rows[1]["result"]["results"]["bindings"][0]["left"]["value"],
            format!("http://example.test/item/l-{id}")
        );
        assert_origins(&rows[0], &rows[1]);
    }
}

#[tokio::test]
async fn joined_lineage_overflow_never_returns_a_complete_prefix() {
    for (build, count) in [(true, 129), (false, 4097)] {
        let values: Vec<_> = (0..count).map(|i| format!("item{i}")).collect();
        let refs: Vec<_> = values.iter().map(|s| (s.as_str(), "same")).collect();
        let small = [("same", "same")];
        let data = if build {
            [refs.as_slice(), small.as_slice()]
        } else {
            [small.as_slice(), refs.as_slice()]
        };
        let (mut cfg, pools, _files) = joined_config(data, LITERAL, "TEXT");
        protect(&mut cfg);
        cfg.query_limits = QueryLimits::new(1_000_000, 1_000_000_000, 10_000, 100_000_000)
            .with_max_retained_bytes(200_000_000);
        let response = router(Arc::new(cfg))
            .oneshot(lineage_request(JOIN))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(!response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .contains(&0x1e));
        for pool in pools {
            drop(
                tokio::time::timeout(Duration::from_secs(1), pool.pick_owned().acquire())
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
    }
}
