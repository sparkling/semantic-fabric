//! Frozen, private PostgreSQL 16 rich-catalogue query contracts (ADR-0051 §8).
//!
//! These strings are decoder inputs only. They are not runtime SQL until the
//! typed row adapters and live qualification receipts are complete.

pub(super) const RICH_RELATIONS_SQL_V1: &str = "SELECT \
 c.oid AS relation_oid, c.relnamespace AS relation_namespace_oid, n.oid AS joined_namespace_oid, \
 CASE WHEN n.nspname IS NULL OR pg_catalog.octet_length(pg_catalog.convert_to(n.nspname::text,'UTF8')) > $2 \
      THEN NULL::text ELSE n.nspname::text END AS namespace_name, \
 CASE WHEN c.relname IS NULL OR pg_catalog.octet_length(pg_catalog.convert_to(c.relname::text,'UTF8')) > $2 \
      THEN NULL::text ELSE c.relname::text END AS relation_name, \
 (n.nspname IS NULL OR c.relname IS NULL OR pg_catalog.octet_length(pg_catalog.convert_to(n.nspname::text,'UTF8')) > $2 OR pg_catalog.octet_length(pg_catalog.convert_to(c.relname::text,'UTF8')) > $2) AS sf_text_overflow, \
 c.relkind, c.relpersistence, c.relispartition, c.relrowsecurity, c.relforcerowsecurity, c.reloftype, c.relrewrite, c.relam, am.oid AS joined_access_method_oid, amn.nspname AS access_method_namespace, am.amname AS access_method_name, am.amtype AS access_method_type, c.relnatts \
 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace LEFT JOIN pg_catalog.pg_am am ON am.oid=c.relam LEFT JOIN pg_catalog.pg_namespace amn ON amn.oid=am.amnamespace \
 WHERE n.nspname=$1 AND c.relkind IN ('r','p','f') ORDER BY c.relname LIMIT $3";

pub(super) const RICH_ATTRIBUTES_SQL_V1: &str = "SELECT \
 a.attrelid AS relation_oid, a.attnum, CASE WHEN a.attname IS NULL OR pg_catalog.octet_length(pg_catalog.convert_to(a.attname::text,'UTF8')) > $2 THEN NULL::text ELSE a.attname::text END AS attribute_name, \
 (a.attname IS NULL OR pg_catalog.octet_length(pg_catalog.convert_to(a.attname::text,'UTF8')) > $2) AS sf_text_overflow, \
 a.attisdropped, a.attislocal, a.attinhcount, a.attnotnull, a.atttypid, a.attndims, a.atttypmod, a.attcollation \
 FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
 WHERE n.nspname=$1 AND c.relkind IN ('r','p','f') AND a.attnum > 0 ORDER BY a.attrelid, a.attnum LIMIT $3";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_queries_are_scoped_bounded_and_overflow_aware() {
        for query in [RICH_RELATIONS_SQL_V1, RICH_ATTRIBUTES_SQL_V1] {
            assert!(query.contains("LIMIT $3"));
            assert!(query.contains("sf_text_overflow"));
            assert!(query.contains("n.nspname=$1"));
            assert!(query.contains("c.relkind IN ('r','p','f')"));
        }
    }

    #[test]
    fn attribute_query_retains_positive_dropped_slots_and_pg_coordinates() {
        for field in [
            "a.attnum",
            "a.attisdropped",
            "a.atttypid",
            "a.attcollation",
            "a.attndims",
        ] {
            assert!(RICH_ATTRIBUTES_SQL_V1.contains(field));
        }
        assert!(RICH_ATTRIBUTES_SQL_V1.contains("a.attnum > 0"));
    }
}
