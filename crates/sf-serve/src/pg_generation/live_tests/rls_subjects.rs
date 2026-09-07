//! One server, one policy snapshot, multiple authenticated source identities.
use super::*;
use crate::{ProvisionedBearerAdmission, ProvisionedBearerSubject};

const ALICE: &str = "test-only-alice-credential-0123456789";
const BOB: &str = "test-only-bob-credential-9876543210";

fn registry() -> QueryAdmission {
    QueryAdmission::ProvisionedBearers(
        ProvisionedBearerAdmission::new(
            [
                ("opaque-a", ALICE, "tenant-a"),
                ("opaque-b", BOB, "tenant-b"),
            ]
            .into_iter()
            .map(|(id, token, tenant)| {
                ProvisionedBearerSubject::postgres_rls(
                    id,
                    token,
                    PostgresRlsClaims::new(BTreeMap::from([(
                        "app.tenant_id".into(),
                        tenant.into(),
                    )]))
                    .unwrap(),
                )
                .unwrap()
            })
            .collect(),
        )
        .unwrap(),
    )
}

async fn request_as(cfg: Arc<ServeConfig>, token: &str, query: &str) -> Response {
    crate::router(cfg)
        .oneshot(
            Request::post("/sparql")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/sparql-query")
                .header("x-subject-id", "opaque-b")
                .header("x-tenant-id", "tenant-b")
                .header("x-postgres-rls-context", r#"{"app.tenant_id":"tenant-b"}"#)
                .body(Body::from(query.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn text_as(cfg: Arc<ServeConfig>, token: &str, query: &str) -> String {
    let response = request_as(cfg, token, query).await;
    assert_eq!(response.status(), StatusCode::OK);
    String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}

async fn configured(pool: deadpool_postgres::Pool) -> Arc<ServeConfig> {
    let mut cfg = config(pool, "unused", &mapping("parent", "http://ex/name")).await;
    cfg.set_query_admission(registry());
    Arc::new(cfg)
}

pub(super) async fn exercise(f: &Fixture) {
    let cfg = configured(f.pool.clone()).await;
    let pid = clean(&f.pool).await;
    for (token, allowed, denied) in [
        (ALICE, "Alice", "Bob"),
        (BOB, "Bob", "Alice"),
        (ALICE, "Alice", "Bob"),
    ] {
        for query in [
            SELECT,
            "CONSTRUCT { ?s <http://ex/name> ?name } WHERE { ?s <http://ex/name> ?name }",
        ] {
            let body = text_as(cfg.clone(), token, query).await;
            assert!(body.contains(allowed) && !body.contains(denied), "{body}");
            assert_eq!(
                clean(&f.pool).await,
                pid,
                "clean pool member must be reused"
            );
        }
        for (name, expected) in [(allowed, true), (denied, false)] {
            let body = text_as(
                cfg.clone(),
                token,
                &format!("ASK {{ ?s <http://ex/name> \"{name}\" }}"),
            )
            .await;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&body).unwrap()["boolean"],
                expected
            );
            assert_eq!(clean(&f.pool).await, pid);
        }
    }
    // Invalid credentials never fall back to a default registry member.
    assert_eq!(
        request_as(cfg, "invalid", SELECT).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(clean(&f.pool).await, pid);

    let concurrent_pool = pool(f.reader_config(), 2);
    let concurrent = configured(concurrent_pool.clone()).await;
    for _ in 0..4 {
        let (alice, bob) = tokio::join!(
            text_as(concurrent.clone(), ALICE, SELECT),
            text_as(concurrent.clone(), BOB, SELECT),
        );
        assert!(alice.contains("Alice") && !alice.contains("Bob"));
        assert!(bob.contains("Bob") && !bob.contains("Alice"));
    }
    let c1 = concurrent_pool.get().await.unwrap();
    let c2 = concurrent_pool.get().await.unwrap();
    for conn in [&c1, &c2] {
        let value: Option<String> = conn
            .query_one("SELECT current_setting('app.tenant_id',true)", &[])
            .await
            .unwrap()
            .get(0);
        assert!(value.is_none_or(|v| v.is_empty()));
    }
    drop((c1, c2, concurrent, concurrent_pool));
    exercise_union(f).await;
}

async fn exercise_union(f: &Fixture) {
    use crate::semantic_admission::{MappingOrigin, ValidatedMapping};
    let ontology = crate::test_support::ontology(&[], &["http://ex/left", "http://ex/right"]);
    let second_pool = pool(f.reader_config(), 1);
    let mut sources = Vec::new();
    for (id, table, predicate, pool) in [
        (0, "parent", "http://ex/left", f.pool.clone()),
        (1, "child", "http://ex/right", second_pool.clone()),
    ] {
        let source = IntrospectedSource::observe_postgres(pool).await.unwrap();
        let mapping = sf_core::SourceMapping::new(
            SourceId::new(id).unwrap(),
            sf_mapping::parse_r2rml(&mapping(table, predicate)).unwrap(),
        );
        let mapping =
            ValidatedMapping::validate(mapping, MappingOrigin::Authored, &ontology, &source)
                .unwrap();
        sources.push(crate::RuntimeSource::admitted(source, mapping).unwrap());
    }
    let mut cfg = ServeConfig::new_federated(sources.try_into().ok().unwrap(), ontology).unwrap();
    cfg.set_query_admission(registry());
    let cfg = Arc::new(cfg);
    let query =
        "SELECT ?name WHERE { { ?s <http://ex/left> ?name } UNION { ?s <http://ex/right> ?name } }";
    for (token, allowed, denied) in [
        (ALICE, "Alice", "Bob"),
        (BOB, "Bob", "Alice"),
        (ALICE, "Alice", "Bob"),
    ] {
        let body = text_as(cfg.clone(), token, query).await;
        let result: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(result["results"]["bindings"].as_array().unwrap().len(), 2);
        assert!(body.contains(allowed) && !body.contains(denied));
        clean(&f.pool).await;
        clean(&second_pool).await;
    }
}
