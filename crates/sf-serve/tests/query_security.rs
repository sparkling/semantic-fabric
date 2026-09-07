//! Public query admission: credentials never come from query parameters or identity headers.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use sf_serve::{router, Backend, BearerQueryAdmission, QueryAdmission, ServeConfig};
use tower::ServiceExt;

mod support;

const TOKEN: &str = "test-only-query-credential-0123456789";
const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#people> a rr:TriplesMap; rr:logicalTable [rr:tableName "people"];
 rr:subjectMap [rr:template "http://ex/{id}"];
 rr:predicateObjectMap [rr:predicate <http://ex/name>; rr:objectMap [rr:column "name"]].
"#;

fn config(admission: QueryAdmission) -> ServeConfig {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE people(id INTEGER, name TEXT); INSERT INTO people VALUES(1,'Alice');",
    )
    .unwrap();
    let mut cfg = support::serve_config(Backend::sqlite(conn), MAPPING);
    cfg.set_query_admission(admission);
    cfg
}

fn protected() -> ServeConfig {
    config(QueryAdmission::Bearer(
        BearerQueryAdmission::for_service_principal(TOKEN).unwrap(),
    ))
}

#[tokio::test]
async fn rejects_before_polling_body_and_ignores_spoofed_identity() {
    for authorization in [None, Some("Bearer wrong"), Some("Basic dGVzdA==")] {
        let polls = Arc::new(AtomicUsize::new(0));
        let observed = polls.clone();
        let body = Body::from_stream(tokio_stream::iter(std::iter::from_fn(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            Some(Err::<String, _>(std::io::Error::other(
                "body must not be polled",
            )))
        })));
        let mut request = Request::post("/sparql")
            .header(header::CONTENT_TYPE, "application/sparql-query")
            .header("x-authenticated-subject", "admin")
            .body(body)
            .unwrap();
        if let Some(value) = authorization {
            request
                .headers_mut()
                .insert(header::AUTHORIZATION, value.parse().unwrap());
        }
        let response = router(Arc::new(protected()))
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("unauthenticated"));
        assert!(!text.contains("wrong") && !text.contains("admin") && !text.contains(TOKEN));
    }
}

#[tokio::test]
async fn credentials_admit_select_ask_and_construct_results() {
    let app = router(Arc::new(protected()));
    for (query, expected) in [
        ("SELECT ?name WHERE { ?s <http://ex/name> ?name }", "Alice"),
        ("ASK { ?s <http://ex/name> ?name }", "true"),
        ("CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }", "Alice"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post("/sparql")
                    .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/sparql-query")
                    .body(Body::from(query))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8(bytes.to_vec())
            .unwrap()
            .contains(expected));
    }
}

#[tokio::test]
async fn duplicate_authorization_is_rejected_and_control_metadata_stays_public() {
    let app = router(Arc::new(protected()));
    let response = app
        .clone()
        .oneshot(
            Request::post("/sparql")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    for path in ["/livez", "/readyz", "/sparql"] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn deny_profile_never_admits_even_a_valid_bearer() {
    let response = router(Arc::new(config(QueryAdmission::Deny)))
        .oneshot(
            Request::post("/sparql")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::from("malformed query"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn get_and_form_require_headers_not_credentials_in_query_parameters() {
    let app = router(Arc::new(protected()));
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("query", "SELECT ?name WHERE { ?s <http://ex/name> ?name }")
        .finish();
    for request in [
        Request::get(format!("/sparql?{query}"))
            .header(header::AUTHORIZATION, format!("bEaReR {TOKEN}"))
            .body(Body::empty())
            .unwrap(),
        Request::post("/sparql")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
            .body(Body::from(query.clone()))
            .unwrap(),
    ] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec()
        )
        .unwrap()
        .contains("Alice"));
    }
    let response = app
        .oneshot(
            Request::get(format!("/sparql?{query}&access_token={TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn credential_validation_is_bounded_and_diagnostics_are_redacted() {
    for token in [
        "".to_owned(),
        "short".to_owned(),
        "x".repeat(1025),
        format!("{TOKEN} bad"),
        format!("{TOKEN}\n"),
    ] {
        let error = BearerQueryAdmission::for_service_principal(&token).unwrap_err();
        assert_eq!(error.code(), "startup-configuration");
        assert!(!format!("{error:?}").contains(TOKEN));
    }
    let admission = BearerQueryAdmission::for_service_principal(TOKEN).unwrap();
    assert_eq!(format!("{admission:?}"), "BearerQueryAdmission([REDACTED])");
}

#[tokio::test]
async fn public_embedding_defaults_to_deny() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE TABLE people(id INTEGER, name TEXT)")
        .unwrap();
    let source = sf_serve::IntrospectedSource::observe_sqlite(Backend::sqlite(connection)).unwrap();
    let mapping = sf_mapping::parse_r2rml(MAPPING).unwrap();
    let cfg =
        ServeConfig::from_authored_r2rml(source, MAPPING, support::ontology_for_mapping(&mapping))
            .unwrap();
    let response = router(Arc::new(cfg))
        .oneshot(Request::post("/sparql").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
