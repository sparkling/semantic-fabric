//! Authorization precedes protected-generation shape admission and source work.
use super::*;
use crate::{BearerQueryAdmission, PortableRowPolicy, PortableRowRule, QueryAdmission};

const TOKEN: &str = "test-only-preflight-policy-0123456789";
const MAPPING: &str = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> a rr:TriplesMap; rr:logicalTable [rr:tableName "items"];
rr:subjectMap [rr:template "http://example.test/item/{id}"];
rr:predicateObjectMap [rr:predicate <http://example.test/a>; rr:objectMap [rr:column "value"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]]."#;

#[tokio::test]
async fn denied_preflight_hides_shape_errors_and_never_consumes_source_work() {
    let query = "SELECT ?value WHERE { ?s <http://example.test/a> ?value } ORDER BY ?value";
    for (table, expected) in [
        ("uncovered", StatusCode::FORBIDDEN),
        ("items", StatusCode::NOT_IMPLEMENTED),
    ] {
        let (mut cfg, _) = config_with_mapping(u64::MAX, sf_mapping::parse_r2rml(MAPPING).unwrap());
        let mutable = Arc::get_mut(&mut cfg).unwrap();
        mutable.set_max_order_rows(0);
        mutable.set_query_admission(QueryAdmission::Bearer(
            BearerQueryAdmission::for_service_principal(TOKEN)
                .unwrap()
                .with_portable_rows(
                    PortableRowPolicy::new(vec![
                        PortableRowRule::new(0, table, "tenant", "a").unwrap()
                    ])
                    .unwrap(),
                )
                .unwrap(),
        ));
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
        let mut budget = cfg.request_budget();
        budget
            .retain_authenticated(cfg.query_admission.admit(&headers).unwrap())
            .unwrap();
        let result = preflight(
            cfg.clone(),
            cfg.runtime_lease().unwrap(),
            query.into(),
            budget.clone(),
        )
        .await;
        assert_eq!(
            result
                .err()
                .expect("denial or unsupported ordering")
                .status(),
            expected
        );
        assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
        let recovered = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            cfg.compiler_permits().acquire_many_owned(4),
        )
        .await
        .unwrap()
        .unwrap();
        drop(recovered);
    }
}
