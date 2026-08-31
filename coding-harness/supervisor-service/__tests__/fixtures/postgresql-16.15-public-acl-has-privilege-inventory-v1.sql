-- SPDX-License-Identifier: MIT
-- ADR-0047 independent raw-catalogue candidate matrix. This query contains no
-- effective-privilege call and no expected ACL result.

BEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY DEFERRABLE;
SET LOCAL search_path TO pg_catalog;
SET LOCAL row_security TO on;
SET LOCAL quote_all_identifiers TO off;
SET LOCAL client_encoding TO 'UTF8';

\echo @@ADR0047-HAS-INVENTORY-V1/SCHEMA/BEGIN@@
COPY (
  WITH privilege(name) AS (VALUES ('CREATE'::text), ('USAGE'::text))
  SELECT n.oid, pg_catalog.encode(pg_catalog.convert_to(n.nspname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(p.name, 'UTF8'), 'hex')
  FROM pg_catalog.pg_namespace AS n CROSS JOIN privilege AS p
  WHERE n.nspname NOT IN ('public', 'sf_supervisor_v1')
  ORDER BY n.nspname::text COLLATE pg_catalog."C", p.name COLLATE pg_catalog."C", n.oid
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-INVENTORY-V1/SCHEMA/END@@

\echo @@ADR0047-HAS-INVENTORY-V1/RELATION/BEGIN@@
COPY (
  WITH privilege(name) AS (VALUES ('DELETE'::text), ('INSERT'::text),
    ('REFERENCES'::text), ('SELECT'::text), ('TRIGGER'::text), ('TRUNCATE'::text),
    ('UPDATE'::text), ('USAGE'::text))
  SELECT c.oid, pg_catalog.encode(pg_catalog.convert_to(n.nspname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(c.relname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(CASE c.relkind
      WHEN 'r' THEN 'table' WHEN 'p' THEN 'partitioned-table' WHEN 'v' THEN 'view'
      WHEN 'm' THEN 'materialized-view' WHEN 'f' THEN 'foreign-table'
      WHEN 'S' THEN 'sequence' END, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(p.name, 'UTF8'), 'hex')
  FROM pg_catalog.pg_class AS c
  JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
  CROSS JOIN privilege AS p
  WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S')
    AND n.nspname NOT IN ('public', 'sf_supervisor_v1')
    AND (CASE WHEN c.relkind = 'S' THEN p.name IN ('SELECT', 'UPDATE', 'USAGE')
      ELSE p.name <> 'USAGE' END)
  ORDER BY n.nspname::text COLLATE pg_catalog."C", c.relname::text COLLATE pg_catalog."C",
    (CASE c.relkind WHEN 'r' THEN 'table' WHEN 'p' THEN 'partitioned-table'
      WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized-view'
      WHEN 'f' THEN 'foreign-table' WHEN 'S' THEN 'sequence' END)
      COLLATE pg_catalog."C", p.name COLLATE pg_catalog."C", c.oid
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-INVENTORY-V1/RELATION/END@@

\echo @@ADR0047-HAS-INVENTORY-V1/COLUMN/BEGIN@@
COPY (
  WITH privilege(name) AS (VALUES
    ('INSERT'::text), ('REFERENCES'::text), ('SELECT'::text), ('UPDATE'::text))
  SELECT c.oid, a.attnum,
    pg_catalog.encode(pg_catalog.convert_to(n.nspname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(c.relname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(a.attname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(CASE c.relkind
      WHEN 'r' THEN 'table' WHEN 'p' THEN 'partitioned-table' WHEN 'v' THEN 'view'
      WHEN 'm' THEN 'materialized-view' WHEN 'f' THEN 'foreign-table' END, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(p.name, 'UTF8'), 'hex')
  FROM pg_catalog.pg_attribute AS a
  JOIN pg_catalog.pg_class AS c ON c.oid = a.attrelid
  JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
  CROSS JOIN privilege AS p
  WHERE a.attnum > 0 AND NOT a.attisdropped AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
    AND n.nspname NOT IN ('public', 'sf_supervisor_v1')
  ORDER BY n.nspname::text COLLATE pg_catalog."C", c.relname::text COLLATE pg_catalog."C",
    a.attname::text COLLATE pg_catalog."C",
    (CASE c.relkind WHEN 'r' THEN 'table' WHEN 'p' THEN 'partitioned-table'
      WHEN 'v' THEN 'view' WHEN 'm' THEN 'materialized-view'
      WHEN 'f' THEN 'foreign-table' END) COLLATE pg_catalog."C",
    p.name COLLATE pg_catalog."C", c.oid, a.attnum
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-INVENTORY-V1/COLUMN/END@@

\echo @@ADR0047-HAS-INVENTORY-V1/ROUTINE/BEGIN@@
COPY (
  SELECT p.oid, pg_catalog.encode(pg_catalog.convert_to(n.nspname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(p.proname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(
      pg_catalog.pg_get_function_identity_arguments(p.oid), 'UTF8'
    ), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(CASE p.prokind
      WHEN 'f' THEN 'function' WHEN 'p' THEN 'procedure' WHEN 'a' THEN 'aggregate'
      WHEN 'w' THEN 'window-function' END, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to('EXECUTE', 'UTF8'), 'hex')
  FROM pg_catalog.pg_proc AS p
  JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
  WHERE p.prokind IN ('f', 'p', 'a', 'w')
    AND n.nspname NOT IN ('public', 'sf_supervisor_v1')
  ORDER BY n.nspname::text COLLATE pg_catalog."C", p.proname::text COLLATE pg_catalog."C",
    (CASE p.prokind WHEN 'f' THEN 'function' WHEN 'p' THEN 'procedure'
      WHEN 'a' THEN 'aggregate' WHEN 'w' THEN 'window-function' END)
      COLLATE pg_catalog."C",
    pg_catalog.pg_get_function_identity_arguments(p.oid) COLLATE pg_catalog."C", p.oid
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-INVENTORY-V1/ROUTINE/END@@

\echo @@ADR0047-HAS-INVENTORY-V1/TYPE/BEGIN@@
COPY (
  SELECT t.oid, pg_catalog.encode(pg_catalog.convert_to(n.nspname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(t.typname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(CASE
      WHEN t.typelem <> 0
        AND t.typsubscript = 'pg_catalog.array_subscript_handler'::regproc THEN 'array'
      WHEN t.typtype = 'b' THEN 'base' WHEN t.typtype = 'c' THEN 'composite'
      WHEN t.typtype = 'd' THEN 'domain' WHEN t.typtype = 'e' THEN 'enum'
      WHEN t.typtype = 'p' THEN 'pseudo' WHEN t.typtype = 'r' THEN 'range'
      WHEN t.typtype = 'm' THEN 'multirange' END, 'UTF8'), 'hex'),
    t.typelem <> 0 AND t.typsubscript = 'pg_catalog.array_subscript_handler'::regproc,
    element.oid,
    pg_catalog.encode(pg_catalog.convert_to(element_n.nspname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(element.typname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to('USAGE', 'UTF8'), 'hex')
  FROM pg_catalog.pg_type AS t
  JOIN pg_catalog.pg_namespace AS n ON n.oid = t.typnamespace
  LEFT JOIN pg_catalog.pg_type AS element
    ON t.typelem <> 0
   AND t.typsubscript = 'pg_catalog.array_subscript_handler'::regproc
   AND element.oid = t.typelem
  LEFT JOIN pg_catalog.pg_namespace AS element_n ON element_n.oid = element.typnamespace
  WHERE t.typtype IN ('b', 'c', 'd', 'e', 'p', 'r', 'm')
    AND n.nspname NOT IN ('public', 'sf_supervisor_v1')
  ORDER BY n.nspname::text COLLATE pg_catalog."C", t.typname::text COLLATE pg_catalog."C",
    (CASE WHEN t.typelem <> 0
        AND t.typsubscript = 'pg_catalog.array_subscript_handler'::regproc THEN 'array'
      WHEN t.typtype = 'b' THEN 'base' WHEN t.typtype = 'c' THEN 'composite'
      WHEN t.typtype = 'd' THEN 'domain' WHEN t.typtype = 'e' THEN 'enum'
      WHEN t.typtype = 'p' THEN 'pseudo' WHEN t.typtype = 'r' THEN 'range'
      WHEN t.typtype = 'm' THEN 'multirange' END) COLLATE pg_catalog."C", t.oid
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-INVENTORY-V1/TYPE/END@@

\echo @@ADR0047-HAS-INVENTORY-V1/LANGUAGE/BEGIN@@
COPY (
  SELECT l.oid, pg_catalog.encode(pg_catalog.convert_to(l.lanname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to('USAGE', 'UTF8'), 'hex')
  FROM pg_catalog.pg_language AS l
  ORDER BY l.lanname::text COLLATE pg_catalog."C", l.oid
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-INVENTORY-V1/LANGUAGE/END@@

\echo @@ADR0047-HAS-INVENTORY-V1/FDW/BEGIN@@
COPY (
  SELECT f.oid, pg_catalog.encode(pg_catalog.convert_to(f.fdwname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to('USAGE', 'UTF8'), 'hex')
  FROM pg_catalog.pg_foreign_data_wrapper AS f
  ORDER BY f.fdwname::text COLLATE pg_catalog."C", f.oid
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-INVENTORY-V1/FDW/END@@

\echo @@ADR0047-HAS-INVENTORY-V1/SERVER/BEGIN@@
COPY (
  SELECT s.oid, pg_catalog.encode(pg_catalog.convert_to(s.srvname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to('USAGE', 'UTF8'), 'hex')
  FROM pg_catalog.pg_foreign_server AS s
  ORDER BY s.srvname::text COLLATE pg_catalog."C", s.oid
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-INVENTORY-V1/SERVER/END@@

ROLLBACK;
