//! Public profile boundaries: database identity is trusted configuration, not a header.
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use sf_serve::{BearerQueryAdmission, PostgresRlsClaims, QueryAdmission};
use std::collections::BTreeMap;
use std::sync::Arc;
use tower::ServiceExt;

mod support;
const TOKEN: &str = "test-only-rls-credential-0123456789";

#[test]
fn source_claims_are_bounded_custom_settings_with_redacted_diagnostics() {
    let claims = PostgresRlsClaims::new(BTreeMap::from([(
        "app.tenant_id".into(),
        "tenant-secret".into(),
    )]))
    .unwrap();
    assert!(!format!("{claims:?}").contains("tenant-secret"));
    for key in [
        "role",
        "search_path",
        "row_security",
        "app.bad-name",
        "app.tenant; RESET ALL",
        "app.",
        "App.tenant_id",
        "pg_reserved.identity",
    ] {
        assert!(PostgresRlsClaims::new(BTreeMap::from([(key.into(), "x".into())])).is_err());
    }
    assert!(PostgresRlsClaims::new(BTreeMap::new()).is_err());
    assert!(PostgresRlsClaims::new(
        (0..17)
            .map(|n| (format!("app.claim_{n}"), "x".into()))
            .collect()
    )
    .is_err());
    assert!(
        PostgresRlsClaims::new(BTreeMap::from([("app.tenant_id".into(), "x\0y".into())])).is_err()
    );
    assert!(
        PostgresRlsClaims::new(BTreeMap::from([("app.tenant_id".into(), "x".repeat(1025))]))
            .is_err()
    );
}

#[tokio::test]
async fn postgres_row_security_never_falls_back_to_unfiltered_sqlite() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE people(id INTEGER); INSERT INTO people VALUES(1)")
        .unwrap();
    let mapping = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#p> a rr:TriplesMap; rr:logicalTable [rr:tableName "people"];
      rr:subjectMap [rr:template "http://ex/{id}"; rr:class <http://ex/Person>]."#;
    let mut cfg = support::serve_config(sf_serve::Backend::sqlite(conn), mapping);
    let claims = PostgresRlsClaims::new(BTreeMap::from([(
        "app.tenant_id".into(),
        "tenant-a".into(),
    )]))
    .unwrap();
    cfg.set_query_admission(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN)
            .unwrap()
            .with_postgres_rls(claims)
            .unwrap(),
    ));
    let response = sf_serve::router(Arc::new(cfg))
        .oneshot(
            Request::post("/sparql")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header(header::CONTENT_TYPE, "application/sparql-query")
                .body(Body::from("SELECT ?s WHERE { ?s a <http://ex/Person> }"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
