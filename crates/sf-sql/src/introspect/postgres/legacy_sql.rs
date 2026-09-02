pub(super) const TABLES_SQL: &str = "SELECT \
     (table_name IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(table_name::pg_catalog.text, 'UTF8')) > $2) \
       AS sf_text_rejected, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(table_name::pg_catalog.text, 'UTF8')) <= $2 \
       THEN table_name::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_0 \
     FROM information_schema.tables \
     WHERE table_schema::pg_catalog.text = $1 AND table_type = 'BASE TABLE' \
     ORDER BY table_name LIMIT $3";

// Generated base-table SQL is intentionally unqualified under the runtime's
// exact `pg_catalog,public,pg_temp` search path. A `public` base table that has
// the same name as any `pg_catalog` relation would therefore be introspected
// from `public` but executed from the earlier catalogue namespace. Until the IR
// carries qualified relation identity, reject that database at introspection.
pub(super) const EARLIER_RELATION_COLLISIONS_SQL: &str = "SELECT \
     (c.relname IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(c.relname::pg_catalog.text, 'UTF8')) > $3) \
       AS sf_text_rejected, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(c.relname::pg_catalog.text, 'UTF8')) <= $3 \
       THEN c.relname::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_0 \
     FROM pg_catalog.pg_class c \
     JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
     WHERE c.relname::pg_catalog.text = ANY($1) \
       AND n.nspname::pg_catalog.text = $2 ORDER BY c.relname LIMIT $4";

pub(super) const COLUMNS_SQL: &str = "SELECT \
     (table_name IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(table_name::pg_catalog.text, 'UTF8')) > $3 OR \
      column_name IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(column_name::pg_catalog.text, 'UTF8')) > $3 OR \
      data_type IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(data_type::pg_catalog.text, 'UTF8')) > $3 OR \
      is_nullable IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(is_nullable::pg_catalog.text, 'UTF8')) > $3) \
       AS sf_text_rejected, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(table_name::pg_catalog.text, 'UTF8')) <= $3 \
       THEN table_name::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_0, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(column_name::pg_catalog.text, 'UTF8')) <= $3 \
       THEN column_name::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_1, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(data_type::pg_catalog.text, 'UTF8')) <= $3 \
       THEN data_type::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_2, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(is_nullable::pg_catalog.text, 'UTF8')) <= $3 \
       THEN is_nullable::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_3 \
     FROM information_schema.columns \
     WHERE table_name::pg_catalog.text = ANY($1) \
       AND table_schema::pg_catalog.text = $2 \
     ORDER BY table_name, ordinal_position LIMIT $4";

pub(super) const KEYS_SQL: &str = "SELECT \
     (tc.table_name IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(tc.table_name::pg_catalog.text, 'UTF8')) > $3 OR \
      tc.constraint_type IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(tc.constraint_type::pg_catalog.text, 'UTF8')) > $3 OR \
      tc.constraint_name IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(tc.constraint_name::pg_catalog.text, 'UTF8')) > $3 OR \
      kcu.column_name IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(kcu.column_name::pg_catalog.text, 'UTF8')) > $3) \
       AS sf_text_rejected, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(tc.table_name::pg_catalog.text, 'UTF8')) <= $3 \
       THEN tc.table_name::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_0, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(tc.constraint_type::pg_catalog.text, 'UTF8')) <= $3 \
       THEN tc.constraint_type::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_1, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(tc.constraint_name::pg_catalog.text, 'UTF8')) <= $3 \
       THEN tc.constraint_name::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_2, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(kcu.column_name::pg_catalog.text, 'UTF8')) <= $3 \
       THEN kcu.column_name::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_3 \
     FROM information_schema.table_constraints tc \
     JOIN information_schema.key_column_usage kcu \
       ON tc.constraint_catalog = kcu.constraint_catalog \
      AND tc.constraint_schema = kcu.constraint_schema \
      AND tc.constraint_name = kcu.constraint_name \
      AND tc.table_catalog = kcu.table_catalog \
      AND tc.table_schema = kcu.table_schema \
      AND tc.table_name = kcu.table_name \
     WHERE tc.table_name::pg_catalog.text = ANY($1) \
       AND tc.table_schema::pg_catalog.text = $2 \
       AND tc.constraint_type IN ('PRIMARY KEY', 'UNIQUE') \
     ORDER BY tc.table_name, tc.constraint_name, kcu.ordinal_position LIMIT $4";

// The paired attnum arrays preserve composite-FK column alignment. Parent
// namespace is selected explicitly because the current DTO cannot represent a
// schema-qualified parent and must reject that case rather than misbind it.
pub(super) const FOREIGN_KEYS_SQL: &str = "SELECT \
     (child.relname IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(child.relname::pg_catalog.text, 'UTF8')) > $3 OR \
      con.conname IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(con.conname::pg_catalog.text, 'UTF8')) > $3 OR \
      ca.attname IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(ca.attname::pg_catalog.text, 'UTF8')) > $3 OR \
      parent.relname IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(parent.relname::pg_catalog.text, 'UTF8')) > $3 OR \
      parent_ns.nspname IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(parent_ns.nspname::pg_catalog.text, 'UTF8')) > $3 OR \
      pa.attname IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(pa.attname::pg_catalog.text, 'UTF8')) > $3) \
       AS sf_text_rejected, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(child.relname::pg_catalog.text, 'UTF8')) <= $3 \
       THEN child.relname::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_0, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(con.conname::pg_catalog.text, 'UTF8')) <= $3 \
       THEN con.conname::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_1, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(ca.attname::pg_catalog.text, 'UTF8')) <= $3 \
       THEN ca.attname::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_2, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(parent.relname::pg_catalog.text, 'UTF8')) <= $3 \
       THEN parent.relname::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_3, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(parent_ns.nspname::pg_catalog.text, 'UTF8')) <= $3 \
       THEN parent_ns.nspname::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_4, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(pa.attname::pg_catalog.text, 'UTF8')) <= $3 \
       THEN pa.attname::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_5 \
     FROM pg_catalog.pg_constraint con \
     JOIN pg_catalog.pg_class child ON child.oid = con.conrelid \
     JOIN pg_catalog.pg_namespace child_ns ON child_ns.oid = child.relnamespace \
     JOIN pg_catalog.pg_class parent ON parent.oid = con.confrelid \
     JOIN pg_catalog.pg_namespace parent_ns ON parent_ns.oid = parent.relnamespace \
     JOIN LATERAL ROWS FROM ( \
          pg_catalog.unnest(con.conkey), pg_catalog.unnest(con.confkey) \
     ) WITH ORDINALITY AS k(child_attnum, parent_attnum, ord) ON true \
     JOIN pg_catalog.pg_attribute ca \
       ON ca.attrelid = con.conrelid AND ca.attnum = k.child_attnum \
     JOIN pg_catalog.pg_attribute pa \
       ON pa.attrelid = con.confrelid AND pa.attnum = k.parent_attnum \
     WHERE con.contype = 'f' AND child.relname::pg_catalog.text = ANY($1) \
       AND child_ns.nspname::pg_catalog.text = $2 \
     ORDER BY child.relname, con.conname, k.ord LIMIT $4";

pub(super) const RELTUPLES_SQL: &str = "SELECT \
     (c.relname IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(c.relname::pg_catalog.text, 'UTF8')) > $3) \
       AS sf_text_rejected, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(c.relname::pg_catalog.text, 'UTF8')) <= $3 \
       THEN c.relname::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_0, \
     GREATEST(c.reltuples, 0)::bigint AS row_estimate \
     FROM pg_catalog.pg_class c \
     JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
     WHERE c.relname::pg_catalog.text = ANY($1) \
       AND n.nspname::pg_catalog.text = $2 \
       AND c.relkind IN ('r', 'p', 'm', 'v') LIMIT $4";

pub(super) const NDISTINCT_SQL: &str = "SELECT \
     (tablename IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(tablename::pg_catalog.text, 'UTF8')) > $3 OR \
      attname IS NULL OR \
      pg_catalog.octet_length(pg_catalog.convert_to(attname::pg_catalog.text, 'UTF8')) > $3) \
       AS sf_text_rejected, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(tablename::pg_catalog.text, 'UTF8')) <= $3 \
       THEN tablename::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_0, \
     CASE WHEN pg_catalog.octet_length( \
          pg_catalog.convert_to(attname::pg_catalog.text, 'UTF8')) <= $3 \
       THEN attname::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_1, \
     n_distinct AS distinct_estimate \
     FROM pg_catalog.pg_stats \
     WHERE tablename::pg_catalog.text = ANY($1) \
       AND schemaname::pg_catalog.text = $2 LIMIT $4";
