use std::collections::HashMap;

use sf_core::ir::Segment;

use super::*;

#[test]
fn template_concat_emits_parseable_duckdb_iri_sql() {
    let segments = vec![
        Segment::Literal("urn:person/".into()),
        Segment::Column("slug".into()),
    ];
    let mut params = Vec::new();
    let mut pidx = 0;

    let expression = render_template_concat(
        &segments,
        0,
        true,
        Dialect::DuckDb,
        &HashMap::new(),
        &mut params,
        &mut pidx,
    )
    .unwrap();

    assert_eq!(params, vec!["urn:person/"]);
    assert_eq!(pidx, 1);
    assert_eq!(expression.matches('?').count(), 1, "{expression}");
    assert!(expression.contains("string_split(t0.\"slug\"::text, '')"));
    assert!(expression.contains("WITH ORDINALITY"));
    assert!(expression.contains("ORDER BY ord"));
    assert!(
        expression.contains("CASE WHEN t0.\"slug\"::text IS NULL THEN NULL"),
        "{expression}"
    );

    let query = format!("SELECT {expression} FROM people t0");
    Dialect::DuckDb.parse(&query).unwrap();
}

#[test]
fn composed_duckdb_fragments_keep_text_order_placeholders() {
    let fragment = "SELECT ? AS first, ? AS second";

    let rebased = rebase_placeholders(fragment, Dialect::DuckDb, 7).unwrap();

    assert_eq!(rebased, fragment);
    assert_eq!(rebased.matches('?').count(), 2);
    assert!(!rebased.contains('$'));
    assert_eq!(Dialect::DuckDb.placeholder(8), "?");
}
