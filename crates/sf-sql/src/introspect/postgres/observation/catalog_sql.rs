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
 c.relkind, c.relpersistence, c.relispartition, c.relisshared, c.relrowsecurity, c.relforcerowsecurity, c.reloftype, c.relrewrite, c.relam, am.oid AS joined_access_method_oid, amn.nspname AS access_method_namespace, am.amname AS access_method_name, am.amtype AS access_method_type, (EXISTS (SELECT 1 FROM pg_catalog.pg_inherits h WHERE h.inhrelid=c.oid)) AS inherits_as_child, (EXISTS (SELECT 1 FROM pg_catalog.pg_inherits h WHERE h.inhparent=c.oid)) AS inherits_as_parent, c.relnatts \
 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace LEFT JOIN pg_catalog.pg_am am ON am.oid=c.relam LEFT JOIN pg_catalog.pg_namespace amn ON amn.oid=am.amnamespace \
 WHERE n.nspname=$1 AND c.relkind IN ('r','p','f') ORDER BY c.relname LIMIT $3";

pub(super) const RICH_GUARD_SQL_V1: &str = "SELECT \
 current_setting('server_version_num')::int4 AS server_version_num, \
 current_setting('server_encoding') AS server_encoding, current_setting('client_encoding') AS client_encoding, \
 current_setting('max_identifier_length')::int4 AS max_identifier_length, current_setting('max_index_keys')::int4 AS max_index_keys, \
 current_setting('integer_datetimes') AS integer_datetimes, current_setting('session_replication_role') AS session_replication_role, \
 current_setting('search_path') AS search_path, \
 d.oid AS database_oid, d.datlocprovider AS database_provider, d.datcollate AS database_collate, d.datctype AS database_ctype, d.daticulocale AS database_icu_locale, d.daticurules AS database_icu_rules, d.datcollversion AS database_recorded_version, pg_catalog.pg_database_collation_actual_version(d.oid) AS database_actual_version, \
 (SELECT count(*)::int8 FROM pg_catalog.pg_namespace WHERE nspname='public') AS public_namespace_count, \
 (SELECT count(*)::int8 FROM pg_catalog.pg_database WHERE datname=current_database()) AS current_database_count FROM pg_catalog.pg_database d WHERE d.datname=current_database()";

pub(super) const RICH_ATTRIBUTES_SQL_V1: &str = "WITH bounded AS (SELECT \
 a.attrelid AS relation_oid, a.attnum, CASE WHEN a.attname IS NULL OR pg_catalog.octet_length(pg_catalog.convert_to(a.attname::text,'UTF8')) > $2 THEN NULL::text ELSE a.attname::text END AS attribute_name, \
 (a.attname IS NULL OR pg_catalog.octet_length(pg_catalog.convert_to(a.attname::text,'UTF8')) > $2) AS sf_text_overflow, \
 a.attisdropped, a.attislocal, a.attinhcount, a.attnotnull, a.atttypid, a.attndims, a.atttypmod, a.attcollation, \
 t.oid AS joined_type_oid, t.typname AS joined_type_name, t.typnamespace AS joined_type_namespace_oid, t.typtype AS joined_type_kind, t.typisdefined AS joined_type_is_defined, t.typbasetype AS joined_type_base_oid, t.typelem AS joined_type_element_oid, t.typrelid AS joined_type_relation_oid, t.typcollation AS joined_type_collation_oid, \
 coll.oid AS joined_collation_oid, coll.collname AS joined_collation_name, coll.collnamespace AS joined_collation_namespace_oid, coll.collprovider AS joined_collation_provider, coll.collencoding AS joined_collation_encoding, coll.collisdeterministic AS joined_collation_is_deterministic, coll.collversion AS joined_collation_version, \
 pg_catalog.row_number() OVER (PARTITION BY a.attrelid ORDER BY a.attnum) AS sf_physical_ordinal \
 FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace LEFT JOIN pg_catalog.pg_type t ON t.oid=a.atttypid LEFT JOIN pg_catalog.pg_collation coll ON coll.oid=a.attcollation \
 WHERE n.nspname=$1 AND c.relkind IN ('r','p','f') AND a.attnum > 0) \
 SELECT bounded.*, (bounded.sf_physical_ordinal=1601) AS sf_physical_overflow FROM bounded WHERE bounded.sf_physical_ordinal <= 1601 ORDER BY bounded.relation_oid, bounded.attnum LIMIT $3";

pub(super) const RICH_CONSTRAINTS_SQL_V1: &str = "SELECT \
 con.oid AS constraint_oid, con.contype, con.conrelid AS child_oid, con.confrelid AS parent_oid, \
 con.convalidated, con.condeferrable, con.condeferred, CASE WHEN pg_catalog.cardinality(con.conkey) <= 32 THEN con.conkey ELSE NULL::int2[] END AS conkey, CASE WHEN pg_catalog.cardinality(con.confkey) <= 32 THEN con.confkey ELSE NULL::int2[] END AS confkey, con.conindid, con.confmatchtype, con.confupdtype, con.confdeltype, \
 i.indisprimary, i.indisunique, i.indisvalid, i.indisready, i.indislive, i.indimmediate, i.indnkeyatts, i.indnatts, i.indkey, i.indclass, i.indcollation, i.indnullsnotdistinct, am.amname AS index_access_method, COALESCE((SELECT pg_catalog.bool_and(opc.opcdefault AND opc.opcnamespace=(SELECT oid FROM pg_catalog.pg_namespace WHERE nspname='pg_catalog') AND opc.opcmethod=am.oid) FROM pg_catalog.unnest(i.indclass) AS cls(opclass_oid) JOIN pg_catalog.pg_opclass opc ON opc.oid=cls.opclass_oid), false) AS index_opclass_default, \
 (pg_catalog.cardinality(con.conkey) IS NULL OR pg_catalog.cardinality(con.conkey)=0 OR pg_catalog.cardinality(con.conkey)>32 OR pg_catalog.cardinality(con.confkey)>32) AS sf_array_overflow, \
 pg_catalog.cardinality(tr.trigger_oids) > 4 AS sf_trigger_overflow, tr.trigger_oids, tr.trigger_shape_valid, tr.trigger_all_enabled, op.operator_oids, op.search_operator_oids \
 FROM pg_catalog.pg_constraint con LEFT JOIN pg_catalog.pg_index i ON i.indexrelid=con.conindid LEFT JOIN pg_catalog.pg_class idx ON idx.oid=con.conindid LEFT JOIN pg_catalog.pg_am am ON am.oid=idx.relam \
 LEFT JOIN LATERAL (SELECT pg_catalog.array_agg(t.oid ORDER BY t.oid) AS trigger_oids, COALESCE(count(*)=4 AND bool_and(t.tgconstrindid=con.conindid AND t.tgparentid=0 AND t.tgisinternal AND NOT t.tgdeferrable AND NOT t.tginitdeferred AND t.tgnargs=0 AND t.tgattr IS NULL AND t.tgqual IS NULL AND t.tgoldtable IS NULL AND t.tgnewtable IS NULL), false) AS trigger_shape_valid, COALESCE(count(*)=4 AND bool_and(t.tgenabled IN ('O','A')), false) AS trigger_all_enabled FROM (SELECT t.* FROM pg_catalog.pg_trigger t WHERE t.tgconstraint=con.oid ORDER BY t.oid LIMIT 5) t) tr ON true \
 LEFT JOIN LATERAL (SELECT pg_catalog.array_agg(o.oid ORDER BY o.oid) AS operator_oids, pg_catalog.array_agg(o.amopopr ORDER BY o.oid) AS search_operator_oids FROM pg_catalog.pg_operator o WHERE false) op ON true \
 WHERE con.contype IN ('p','u','f') ORDER BY con.conrelid, con.oid LIMIT $1";

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
    fn guard_query_contains_every_profile_gate_and_no_unbounded_relation_scan() {
        for field in [
            "server_version_num",
            "server_encoding",
            "client_encoding",
            "max_identifier_length",
            "max_index_keys",
            "integer_datetimes",
            "session_replication_role",
            "search_path",
            "public_namespace_count",
            "current_database_count",
            "database_oid",
            "database_provider",
            "database_collate",
            "database_ctype",
            "database_icu_locale",
            "database_icu_rules",
            "database_recorded_version",
            "database_actual_version",
        ] {
            assert!(RICH_GUARD_SQL_V1.contains(field));
        }
        assert!(!RICH_GUARD_SQL_V1.contains("FROM pg_class"));
    }

    #[test]
    fn constraint_query_bounds_arrays_and_trigger_aggregates() {
        for field in [
            "conkey",
            "confkey",
            "sf_array_overflow",
            "sf_trigger_overflow",
            "trigger_oids",
            "operator_oids",
            "search_operator_oids",
            "index_access_method",
            "index_opclass_default",
            "trigger_shape_valid",
            "trigger_all_enabled",
            "LIMIT $1",
        ] {
            assert!(RICH_CONSTRAINTS_SQL_V1.contains(field));
        }
        assert!(RICH_CONSTRAINTS_SQL_V1.contains("cardinality(tr.trigger_oids) > 4"));
        assert!(RICH_CONSTRAINTS_SQL_V1.contains("ORDER BY t.oid LIMIT 5"));
        assert!(RICH_CONSTRAINTS_SQL_V1.contains("cardinality(con.conkey) <= 32"));
    }

    #[test]
    fn attribute_query_retains_positive_dropped_slots_and_pg_coordinates() {
        for field in [
            "a.attnum",
            "a.attisdropped",
            "a.atttypid",
            "a.attcollation",
            "a.attndims",
            "joined_type_oid",
            "joined_type_kind",
            "joined_type_is_defined",
            "joined_type_base_oid",
            "joined_type_element_oid",
            "joined_type_relation_oid",
            "joined_type_collation_oid",
            "joined_collation_oid",
            "joined_collation_encoding",
            "joined_collation_is_deterministic",
        ] {
            assert!(RICH_ATTRIBUTES_SQL_V1.contains(field));
        }
        assert!(RICH_ATTRIBUTES_SQL_V1.contains("a.attnum > 0"));
    }
}
