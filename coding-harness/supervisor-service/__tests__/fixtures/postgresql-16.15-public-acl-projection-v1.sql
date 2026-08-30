-- SPDX-License-Identifier: MIT
-- ADR-0047 test-only projection body. The capture harness supplies the fixed
-- transaction/GUC preamble and same-snapshot profile guard. Each output line
-- is one JSON object; the caller validates and canonicalizes the array.

WITH raw_records AS (
  SELECT 'schema'::text AS object_class, NULL::text AS schema_name,
    n.nspname::text AS object_name, NULL::text AS subobject_name,
    'schema'::text AS object_kind, NULL::text AS routine_identity_arguments,
    a.privilege_type::text AS privilege, a.is_grantable AS grantable
  FROM pg_catalog.pg_namespace AS n
  CROSS JOIN LATERAL pg_catalog.aclexplode(
    CASE WHEN n.nspacl IS NULL THEN pg_catalog.acldefault('n', n.nspowner)
      WHEN pg_catalog.cardinality(n.nspacl) = 0 THEN NULL::aclitem[]
      ELSE n.nspacl END
  ) AS a
  WHERE a.grantee = 0 AND n.nspname NOT IN ('public', 'sf_supervisor_v1')

  UNION ALL
  SELECT 'relation', n.nspname::text, c.relname::text, NULL::text,
    CASE c.relkind WHEN 'r' THEN 'table' WHEN 'p' THEN 'partitioned-table'
      WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized-view'
      WHEN 'f' THEN 'foreign-table' WHEN 'S' THEN 'sequence' END::text,
    NULL::text, a.privilege_type::text, a.is_grantable
  FROM pg_catalog.pg_class AS c
  JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
  CROSS JOIN LATERAL pg_catalog.aclexplode(
    CASE WHEN c.relacl IS NULL THEN pg_catalog.acldefault(
        (CASE WHEN c.relkind = 'S' THEN 's' ELSE 'r' END)::"char", c.relowner)
      WHEN pg_catalog.cardinality(c.relacl) = 0 THEN NULL::aclitem[]
      ELSE c.relacl END
  ) AS a
  WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S') AND a.grantee = 0
    AND n.nspname NOT IN ('public', 'sf_supervisor_v1')

  UNION ALL
  SELECT 'column', n.nspname::text, c.relname::text, att.attname::text,
    CASE c.relkind WHEN 'r' THEN 'table' WHEN 'p' THEN 'partitioned-table'
      WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized-view'
      WHEN 'f' THEN 'foreign-table' END::text,
    NULL::text, a.privilege_type::text, a.is_grantable
  FROM pg_catalog.pg_attribute AS att
  JOIN pg_catalog.pg_class AS c ON c.oid = att.attrelid
  JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
  CROSS JOIN LATERAL pg_catalog.aclexplode(
    CASE WHEN att.attacl IS NULL THEN pg_catalog.acldefault('c', c.relowner)
      WHEN pg_catalog.cardinality(att.attacl) = 0 THEN NULL::aclitem[]
      ELSE att.attacl END
  ) AS a
  WHERE att.attnum > 0 AND NOT att.attisdropped
    AND c.relkind IN ('r', 'p', 'v', 'm', 'f') AND a.grantee = 0
    AND n.nspname NOT IN ('public', 'sf_supervisor_v1')

  UNION ALL
  SELECT 'routine', n.nspname::text, p.proname::text, NULL::text,
    CASE p.prokind WHEN 'f' THEN 'function' WHEN 'p' THEN 'procedure'
      WHEN 'a' THEN 'aggregate' WHEN 'w' THEN 'window-function' END::text,
    pg_catalog.pg_get_function_identity_arguments(p.oid)::text,
    a.privilege_type::text, a.is_grantable
  FROM pg_catalog.pg_proc AS p
  JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
  CROSS JOIN LATERAL pg_catalog.aclexplode(
    CASE WHEN p.proacl IS NULL THEN pg_catalog.acldefault('f', p.proowner)
      WHEN pg_catalog.cardinality(p.proacl) = 0 THEN NULL::aclitem[]
      ELSE p.proacl END
  ) AS a
  WHERE p.prokind IN ('f', 'p', 'a', 'w') AND a.grantee = 0
    AND n.nspname NOT IN ('public', 'sf_supervisor_v1')

  UNION ALL
  SELECT 'type', n.nspname::text, t.typname::text, NULL::text,
    CASE WHEN t.typelem <> 0
          AND t.typsubscript = 'pg_catalog.array_subscript_handler'::regproc
        THEN 'array'
      WHEN t.typtype = 'b' THEN 'base' WHEN t.typtype = 'c' THEN 'composite'
      WHEN t.typtype = 'd' THEN 'domain' WHEN t.typtype = 'e' THEN 'enum'
      WHEN t.typtype = 'p' THEN 'pseudo' WHEN t.typtype = 'r' THEN 'range'
      WHEN t.typtype = 'm' THEN 'multirange' END::text,
    NULL::text, a.privilege_type::text, a.is_grantable
  FROM pg_catalog.pg_type AS t
  JOIN pg_catalog.pg_namespace AS n ON n.oid = t.typnamespace
  LEFT JOIN pg_catalog.pg_type AS element
    ON t.typelem <> 0
   AND t.typsubscript = 'pg_catalog.array_subscript_handler'::regproc
   AND element.oid = t.typelem
  CROSS JOIN LATERAL pg_catalog.aclexplode(
    CASE WHEN t.typelem <> 0
          AND t.typsubscript = 'pg_catalog.array_subscript_handler'::regproc
      THEN CASE WHEN element.typacl IS NULL
          THEN pg_catalog.acldefault('T', element.typowner)
        WHEN pg_catalog.cardinality(element.typacl) = 0 THEN NULL::aclitem[]
        ELSE element.typacl END
      ELSE CASE WHEN t.typacl IS NULL THEN pg_catalog.acldefault('T', t.typowner)
        WHEN pg_catalog.cardinality(t.typacl) = 0 THEN NULL::aclitem[]
        ELSE t.typacl END END
  ) AS a
  WHERE t.typtype IN ('b', 'c', 'd', 'e', 'p', 'r', 'm') AND a.grantee = 0
    AND n.nspname NOT IN ('public', 'sf_supervisor_v1')

  UNION ALL
  SELECT 'language', NULL::text, l.lanname::text, NULL::text, 'language',
    NULL::text, a.privilege_type::text, a.is_grantable
  FROM pg_catalog.pg_language AS l
  CROSS JOIN LATERAL pg_catalog.aclexplode(
    CASE WHEN l.lanacl IS NULL THEN pg_catalog.acldefault('l', l.lanowner)
      WHEN pg_catalog.cardinality(l.lanacl) = 0 THEN NULL::aclitem[]
      ELSE l.lanacl END
  ) AS a WHERE a.grantee = 0

  UNION ALL
  SELECT 'foreign-data-wrapper', NULL::text, f.fdwname::text, NULL::text,
    'foreign-data-wrapper', NULL::text, a.privilege_type::text, a.is_grantable
  FROM pg_catalog.pg_foreign_data_wrapper AS f
  CROSS JOIN LATERAL pg_catalog.aclexplode(
    CASE WHEN f.fdwacl IS NULL THEN pg_catalog.acldefault('F', f.fdwowner)
      WHEN pg_catalog.cardinality(f.fdwacl) = 0 THEN NULL::aclitem[]
      ELSE f.fdwacl END
  ) AS a WHERE a.grantee = 0

  UNION ALL
  SELECT 'foreign-server', NULL::text, s.srvname::text, NULL::text,
    'foreign-server', NULL::text, a.privilege_type::text, a.is_grantable
  FROM pg_catalog.pg_foreign_server AS s
  CROSS JOIN LATERAL pg_catalog.aclexplode(
    CASE WHEN s.srvacl IS NULL THEN pg_catalog.acldefault('S', s.srvowner)
      WHEN pg_catalog.cardinality(s.srvacl) = 0 THEN NULL::aclitem[]
      ELSE s.srvacl END
  ) AS a WHERE a.grantee = 0
)
SELECT pg_catalog.json_build_object(
  'objectClass', object_class, 'schemaName', schema_name,
  'objectName', object_name, 'subobjectName', subobject_name,
  'objectKind', object_kind,
  'routineIdentityArguments', routine_identity_arguments,
  'privilege', privilege, 'grantable', grantable
)::text
FROM raw_records
ORDER BY object_class COLLATE pg_catalog."C" ASC,
  schema_name COLLATE pg_catalog."C" ASC NULLS FIRST,
  object_name COLLATE pg_catalog."C" ASC,
  subobject_name COLLATE pg_catalog."C" ASC NULLS FIRST,
  object_kind COLLATE pg_catalog."C" ASC,
  routine_identity_arguments COLLATE pg_catalog."C" ASC NULLS FIRST,
  privilege COLLATE pg_catalog."C" ASC, grantable ASC;
