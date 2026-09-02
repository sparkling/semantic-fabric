use super::*;

#[test]
fn relation_scope_query_cannot_dispatch_through_a_search_path_operator() {
    assert_eq!(
        POSTGRES_RELATION_SCOPE_QUERY,
        "SELECT pg_catalog.current_setting('search_path') AS search_path"
    );
}

#[test]
fn relation_scope_comparison_is_exact_and_local() {
    assert!(relation_scope_matches("pg_catalog,public,pg_temp"));
    for mismatch in [
        "public,pg_catalog,pg_temp",
        "pg_catalog, public, pg_temp",
        "PG_CATALOG,public,pg_temp",
        "pg_catalog,public",
        "pg_catalog,public,pg_temp,hostile",
    ] {
        assert!(!relation_scope_matches(mismatch), "accepted {mismatch:?}");
    }
}
