use super::*;
use crate::{
    PortableRowPolicy, PortableRowRule, ProvisionedBearerAdmission, ProvisionedBearerSubject,
    QueryAdmission,
};

#[tokio::test]
async fn reload_keeps_row_policy_and_denies_new_uncovered_tables() {
    let mut fixture = Fixture::new();
    let db = rusqlite::Connection::open(fixture.root.join("source.db")).unwrap();
    db.execute_batch(
        "ALTER TABLE items ADD COLUMN tenant TEXT;
        ALTER TABLE items ADD COLUMN refreshed TEXT;
        UPDATE items SET tenant='allowed', refreshed='new-allowed';
        INSERT INTO items VALUES ('secret-old','denied','secret-new');
        CREATE TABLE private(value TEXT, tenant TEXT, refreshed TEXT);
        INSERT INTO private VALUES ('secret-private','allowed','secret-private-new');",
    )
    .unwrap();
    Arc::get_mut(&mut fixture.opts).unwrap().query_admission = QueryAdmission::ProvisionedBearers(
        ProvisionedBearerAdmission::new(vec![ProvisionedBearerSubject::portable_rows(
            "fixture-subject",
            TOKEN,
            PortableRowPolicy::new(vec![
                PortableRowRule::new(0, "items", "tenant", "allowed").unwrap()
            ])
            .unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let config = fixture.config().await;
    assert_eq!(
        value(
            crate::router(Arc::clone(&config))
                .oneshot(request(Body::from(QUERY), true))
                .await
                .unwrap()
        )
        .await,
        "old"
    );
    fixture.replace_mapping("rr:column \"refreshed\"");
    fixture.refresh(&config).await;
    assert_eq!(
        value(
            crate::router(Arc::clone(&config))
                .oneshot(request(Body::from(QUERY), true))
                .await
                .unwrap()
        )
        .await,
        "new-allowed"
    );
    let changed = mapping("rr:column \"refreshed\"")
        .replace("rr:tableName \"items\"", "rr:tableName \"private\"");
    std::fs::write(fixture.root.join("mapping.ttl"), changed).unwrap();
    fixture.refresh(&config).await;
    assert!(matches!(
        config.runtime_readiness().unwrap(),
        RuntimeReadiness::Ready { .. }
    ));
    assert_eq!(
        crate::router(config)
            .oneshot(request(Body::from(QUERY), true))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
}
