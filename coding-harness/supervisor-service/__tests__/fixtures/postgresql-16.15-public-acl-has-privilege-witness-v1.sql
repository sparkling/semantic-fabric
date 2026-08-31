-- SPDX-License-Identifier: MIT
-- ADR-0047 test-only effective-privilege corroboration for one fresh,
-- capability-free role. Expected results come from independent client inputs.

BEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY DEFERRABLE;
SET LOCAL search_path TO pg_catalog;
SET LOCAL row_security TO on;
SET LOCAL quote_all_identifiers TO off;
SET LOCAL client_encoding TO 'UTF8';

\echo @@ADR0047-HAS-V1/ROLE/BEGIN@@
COPY (
  WITH w AS (
    SELECT r.*
    FROM pg_catalog.pg_authid AS r
    WHERE r.rolname = 'sf_public_acl_no_membership_witness_v1'
  )
  SELECT pg_catalog.encode(pg_catalog.convert_to(w.rolname::text, 'UTF8'), 'hex'),
    w.rolsuper, w.rolinherit, w.rolcreaterole, w.rolcreatedb, w.rolcanlogin,
    w.rolreplication, w.rolbypassrls, w.rolconnlimit,
    w.rolpassword IS NULL, w.rolvaliduntil IS NULL,
    NOT EXISTS (
      SELECT 1 FROM pg_catalog.pg_db_role_setting AS s
      WHERE s.setrole = w.oid AND s.setdatabase = 0
    ),
    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_db_role_setting AS s
      WHERE s.setrole = w.oid),
    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_auth_members AS m
      WHERE m.roleid = w.oid OR m.member = w.oid OR m.grantor = w.oid),
    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_authid AS r
      WHERE r.oid <> w.oid
        AND pg_catalog.pg_has_role(w.oid, r.oid, 'MEMBER')),
    pg_catalog.current_setting('server_version_num')::integer,
    pg_catalog.encode(pg_catalog.convert_to(pg_catalog.current_database()::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(session_user::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(current_user::text, 'UTF8'), 'hex')
  FROM w
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-V1/ROLE/END@@

\echo @@ADR0047-HAS-V1/AUTHORITY/BEGIN@@
COPY (
  WITH w AS (
    SELECT r.oid FROM pg_catalog.pg_authid AS r
    WHERE r.rolname = 'sf_public_acl_no_membership_witness_v1'
  ),
  acl_atoms(label, grantee) AS (
    SELECT 'acl-column', a.grantee FROM pg_catalog.pg_attribute AS x
      CROSS JOIN LATERAL pg_catalog.aclexplode(x.attacl) AS a
    UNION ALL SELECT 'acl-database', a.grantee FROM pg_catalog.pg_database AS x
      CROSS JOIN LATERAL pg_catalog.aclexplode(x.datacl) AS a
    UNION ALL SELECT 'acl-default', a.grantee FROM pg_catalog.pg_default_acl AS x
      CROSS JOIN LATERAL pg_catalog.aclexplode(x.defaclacl) AS a
    UNION ALL SELECT 'acl-fdw', a.grantee
      FROM pg_catalog.pg_foreign_data_wrapper AS x
      CROSS JOIN LATERAL pg_catalog.aclexplode(x.fdwacl) AS a
    UNION ALL SELECT 'acl-language', a.grantee FROM pg_catalog.pg_language AS x
      CROSS JOIN LATERAL pg_catalog.aclexplode(x.lanacl) AS a
    UNION ALL SELECT 'acl-large-object', a.grantee
      FROM pg_catalog.pg_largeobject_metadata AS x
      CROSS JOIN LATERAL pg_catalog.aclexplode(x.lomacl) AS a
    UNION ALL SELECT 'acl-parameter', a.grantee FROM pg_catalog.pg_parameter_acl AS x
      CROSS JOIN LATERAL pg_catalog.aclexplode(x.paracl) AS a
    UNION ALL SELECT 'acl-relation', a.grantee FROM pg_catalog.pg_class AS x
      CROSS JOIN LATERAL pg_catalog.aclexplode(x.relacl) AS a
    UNION ALL SELECT 'acl-routine', a.grantee FROM pg_catalog.pg_proc AS x
      CROSS JOIN LATERAL pg_catalog.aclexplode(x.proacl) AS a
    UNION ALL SELECT 'acl-schema', a.grantee FROM pg_catalog.pg_namespace AS x
      CROSS JOIN LATERAL pg_catalog.aclexplode(x.nspacl) AS a
    UNION ALL SELECT 'acl-server', a.grantee FROM pg_catalog.pg_foreign_server AS x
      CROSS JOIN LATERAL pg_catalog.aclexplode(x.srvacl) AS a
    UNION ALL SELECT 'acl-tablespace', a.grantee FROM pg_catalog.pg_tablespace AS x
      CROSS JOIN LATERAL pg_catalog.aclexplode(x.spcacl) AS a
    UNION ALL SELECT 'acl-type', a.grantee FROM pg_catalog.pg_type AS x
      CROSS JOIN LATERAL pg_catalog.aclexplode(x.typacl) AS a
  ),
  acl_labels(label) AS (VALUES
    ('acl-column'), ('acl-database'), ('acl-default'), ('acl-fdw'),
    ('acl-language'), ('acl-large-object'), ('acl-parameter'), ('acl-relation'),
    ('acl-routine'), ('acl-schema'), ('acl-server'), ('acl-tablespace'), ('acl-type')
  ),
  owned(label, owner_oid) AS (
    SELECT 'owned-database', x.datdba FROM pg_catalog.pg_database AS x
    UNION ALL SELECT 'owned-fdw', x.fdwowner FROM pg_catalog.pg_foreign_data_wrapper AS x
    UNION ALL SELECT 'owned-language', x.lanowner FROM pg_catalog.pg_language AS x
    UNION ALL SELECT 'owned-large-object', x.lomowner
      FROM pg_catalog.pg_largeobject_metadata AS x
    UNION ALL SELECT 'owned-relation', x.relowner FROM pg_catalog.pg_class AS x
    UNION ALL SELECT 'owned-routine', x.proowner FROM pg_catalog.pg_proc AS x
    UNION ALL SELECT 'owned-schema', x.nspowner FROM pg_catalog.pg_namespace AS x
    UNION ALL SELECT 'owned-server', x.srvowner FROM pg_catalog.pg_foreign_server AS x
    UNION ALL SELECT 'owned-tablespace', x.spcowner FROM pg_catalog.pg_tablespace AS x
    UNION ALL SELECT 'owned-type', x.typowner FROM pg_catalog.pg_type AS x
  ),
  owned_labels(label) AS (VALUES
    ('owned-database'), ('owned-fdw'), ('owned-language'), ('owned-large-object'),
    ('owned-relation'), ('owned-routine'), ('owned-schema'), ('owned-server'),
    ('owned-tablespace'), ('owned-type')
  ),
  predefined(name) AS (VALUES
    ('pg_checkpoint'), ('pg_create_subscription'), ('pg_database_owner'),
    ('pg_execute_server_program'), ('pg_monitor'), ('pg_read_all_data'),
    ('pg_read_all_settings'), ('pg_read_all_stats'), ('pg_read_server_files'),
    ('pg_signal_backend'), ('pg_stat_scan_tables'), ('pg_use_reserved_connections'),
    ('pg_write_all_data'), ('pg_write_server_files')
  ),
  authority(label, authority_count) AS (
    SELECT l.label, pg_catalog.count(a.grantee) FILTER (WHERE a.grantee = w.oid)
    FROM acl_labels AS l CROSS JOIN w
    LEFT JOIN acl_atoms AS a ON a.label = l.label GROUP BY l.label, w.oid
    UNION ALL
    SELECT l.label, pg_catalog.count(o.owner_oid) FILTER (WHERE o.owner_oid = w.oid)
    FROM owned_labels AS l CROSS JOIN w
    LEFT JOIN owned AS o ON o.label = l.label GROUP BY l.label, w.oid
    UNION ALL
    SELECT 'predefined-member',
      pg_catalog.abs((SELECT pg_catalog.count(*) FROM pg_catalog.pg_authid AS x
        WHERE pg_catalog.left(x.rolname::text, 3) = 'pg_')
        - (SELECT pg_catalog.count(*) FROM predefined))
      + pg_catalog.count(*) FILTER (WHERE r.oid IS NULL
        OR pg_catalog.pg_has_role(w.oid, r.oid, 'MEMBER'))
      FROM w CROSS JOIN predefined AS p
      LEFT JOIN pg_catalog.pg_authid AS r ON r.rolname = p.name
    UNION ALL
    SELECT 'predefined-set',
      pg_catalog.abs((SELECT pg_catalog.count(*) FROM pg_catalog.pg_authid AS x
        WHERE pg_catalog.left(x.rolname::text, 3) = 'pg_')
        - (SELECT pg_catalog.count(*) FROM predefined))
      + pg_catalog.count(*) FILTER (WHERE r.oid IS NULL
        OR pg_catalog.pg_has_role(w.oid, r.oid, 'SET'))
      FROM w CROSS JOIN predefined AS p
      LEFT JOIN pg_catalog.pg_authid AS r ON r.rolname = p.name
    UNION ALL
    SELECT 'predefined-usage',
      pg_catalog.abs((SELECT pg_catalog.count(*) FROM pg_catalog.pg_authid AS x
        WHERE pg_catalog.left(x.rolname::text, 3) = 'pg_')
        - (SELECT pg_catalog.count(*) FROM predefined))
      + pg_catalog.count(*) FILTER (WHERE r.oid IS NULL
        OR pg_catalog.pg_has_role(w.oid, r.oid, 'USAGE'))
      FROM w CROSS JOIN predefined AS p
      LEFT JOIN pg_catalog.pg_authid AS r ON r.rolname = p.name
  )
  SELECT pg_catalog.encode(pg_catalog.convert_to(label, 'UTF8'), 'hex'), authority_count
  FROM authority ORDER BY label COLLATE pg_catalog."C" ASC
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-V1/AUTHORITY/END@@

\echo @@ADR0047-HAS-V1/SCHEMA/BEGIN@@
COPY (
  WITH w AS (SELECT oid FROM pg_catalog.pg_authid
    WHERE rolname = 'sf_public_acl_no_membership_witness_v1'),
  privilege(name) AS (VALUES ('CREATE'::text), ('USAGE'::text))
  SELECT n.oid, pg_catalog.encode(pg_catalog.convert_to(n.nspname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(p.name, 'UTF8'), 'hex'),
    pg_catalog.has_schema_privilege(w.oid, n.oid, p.name),
    pg_catalog.has_schema_privilege(w.oid, n.oid, p.name || ' WITH GRANT OPTION')
  FROM w CROSS JOIN pg_catalog.pg_namespace AS n CROSS JOIN privilege AS p
  WHERE n.nspname NOT IN ('public', 'sf_supervisor_v1')
  ORDER BY n.nspname::text COLLATE pg_catalog."C", p.name COLLATE pg_catalog."C", n.oid
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-V1/SCHEMA/END@@

\echo @@ADR0047-HAS-V1/RELATION/BEGIN@@
COPY (
  WITH w AS (SELECT oid FROM pg_catalog.pg_authid
    WHERE rolname = 'sf_public_acl_no_membership_witness_v1'),
  privilege(name) AS (VALUES ('DELETE'::text), ('INSERT'::text), ('REFERENCES'::text),
    ('SELECT'::text), ('TRIGGER'::text), ('TRUNCATE'::text), ('UPDATE'::text),
    ('USAGE'::text))
  SELECT c.oid, pg_catalog.encode(pg_catalog.convert_to(n.nspname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(c.relname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(CASE c.relkind
      WHEN 'r' THEN 'table' WHEN 'p' THEN 'partitioned-table' WHEN 'v' THEN 'view'
      WHEN 'm' THEN 'materialized-view' WHEN 'f' THEN 'foreign-table'
      WHEN 'S' THEN 'sequence' END, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(p.name, 'UTF8'), 'hex'),
    CASE WHEN c.relkind = 'S'
      THEN pg_catalog.has_sequence_privilege(w.oid, c.oid, p.name)
      ELSE pg_catalog.has_table_privilege(w.oid, c.oid, p.name) END,
    CASE WHEN c.relkind = 'S'
      THEN pg_catalog.has_sequence_privilege(w.oid, c.oid, p.name || ' WITH GRANT OPTION')
      ELSE pg_catalog.has_table_privilege(w.oid, c.oid, p.name || ' WITH GRANT OPTION') END
  FROM w CROSS JOIN pg_catalog.pg_class AS c
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
\echo @@ADR0047-HAS-V1/RELATION/END@@

\echo @@ADR0047-HAS-V1/COLUMN/BEGIN@@
COPY (
  WITH w AS (SELECT oid FROM pg_catalog.pg_authid
    WHERE rolname = 'sf_public_acl_no_membership_witness_v1'),
  privilege(name) AS (VALUES
    ('INSERT'::text), ('REFERENCES'::text), ('SELECT'::text), ('UPDATE'::text))
  SELECT c.oid, a.attnum,
    pg_catalog.encode(pg_catalog.convert_to(n.nspname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(c.relname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(a.attname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(CASE c.relkind
      WHEN 'r' THEN 'table' WHEN 'p' THEN 'partitioned-table' WHEN 'v' THEN 'view'
      WHEN 'm' THEN 'materialized-view' WHEN 'f' THEN 'foreign-table' END, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(p.name, 'UTF8'), 'hex'),
    pg_catalog.has_column_privilege(w.oid, c.oid, a.attnum, p.name),
    pg_catalog.has_column_privilege(
      w.oid, c.oid, a.attnum, p.name || ' WITH GRANT OPTION'
    ),
    pg_catalog.has_table_privilege(w.oid, c.oid, p.name),
    pg_catalog.has_table_privilege(w.oid, c.oid, p.name || ' WITH GRANT OPTION')
  FROM w CROSS JOIN pg_catalog.pg_attribute AS a
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
\echo @@ADR0047-HAS-V1/COLUMN/END@@

\echo @@ADR0047-HAS-V1/ROUTINE/BEGIN@@
COPY (
  WITH w AS (SELECT oid FROM pg_catalog.pg_authid
    WHERE rolname = 'sf_public_acl_no_membership_witness_v1')
  SELECT p.oid, pg_catalog.encode(pg_catalog.convert_to(n.nspname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(p.proname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(
      pg_catalog.pg_get_function_identity_arguments(p.oid), 'UTF8'
    ), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(CASE p.prokind
      WHEN 'f' THEN 'function' WHEN 'p' THEN 'procedure' WHEN 'a' THEN 'aggregate'
      WHEN 'w' THEN 'window-function' END, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to('EXECUTE', 'UTF8'), 'hex'),
    pg_catalog.has_function_privilege(w.oid, p.oid, 'EXECUTE'),
    pg_catalog.has_function_privilege(w.oid, p.oid, 'EXECUTE WITH GRANT OPTION')
  FROM w CROSS JOIN pg_catalog.pg_proc AS p
  JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
  WHERE p.prokind IN ('f', 'p', 'a', 'w')
    AND n.nspname NOT IN ('public', 'sf_supervisor_v1')
  ORDER BY n.nspname::text COLLATE pg_catalog."C", p.proname::text COLLATE pg_catalog."C",
    (CASE p.prokind WHEN 'f' THEN 'function' WHEN 'p' THEN 'procedure'
      WHEN 'a' THEN 'aggregate' WHEN 'w' THEN 'window-function' END)
      COLLATE pg_catalog."C",
    pg_catalog.pg_get_function_identity_arguments(p.oid) COLLATE pg_catalog."C", p.oid
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-V1/ROUTINE/END@@

\echo @@ADR0047-HAS-V1/TYPE/BEGIN@@
COPY (
  WITH w AS (SELECT oid FROM pg_catalog.pg_authid
    WHERE rolname = 'sf_public_acl_no_membership_witness_v1')
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
    pg_catalog.encode(pg_catalog.convert_to('USAGE', 'UTF8'), 'hex'),
    pg_catalog.has_type_privilege(w.oid, t.oid, 'USAGE'),
    pg_catalog.has_type_privilege(w.oid, t.oid, 'USAGE WITH GRANT OPTION')
  FROM w CROSS JOIN pg_catalog.pg_type AS t
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
\echo @@ADR0047-HAS-V1/TYPE/END@@

\echo @@ADR0047-HAS-V1/LANGUAGE/BEGIN@@
COPY (
  WITH w AS (SELECT oid FROM pg_catalog.pg_authid
    WHERE rolname = 'sf_public_acl_no_membership_witness_v1')
  SELECT l.oid, pg_catalog.encode(pg_catalog.convert_to(l.lanname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to('USAGE', 'UTF8'), 'hex'),
    pg_catalog.has_language_privilege(w.oid, l.oid, 'USAGE'),
    pg_catalog.has_language_privilege(w.oid, l.oid, 'USAGE WITH GRANT OPTION')
  FROM w CROSS JOIN pg_catalog.pg_language AS l
  ORDER BY l.lanname::text COLLATE pg_catalog."C", l.oid
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-V1/LANGUAGE/END@@

\echo @@ADR0047-HAS-V1/FDW/BEGIN@@
COPY (
  WITH w AS (SELECT oid FROM pg_catalog.pg_authid
    WHERE rolname = 'sf_public_acl_no_membership_witness_v1')
  SELECT f.oid, pg_catalog.encode(pg_catalog.convert_to(f.fdwname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to('USAGE', 'UTF8'), 'hex'),
    pg_catalog.has_foreign_data_wrapper_privilege(w.oid, f.oid, 'USAGE'),
    pg_catalog.has_foreign_data_wrapper_privilege(
      w.oid, f.oid, 'USAGE WITH GRANT OPTION'
    )
  FROM w CROSS JOIN pg_catalog.pg_foreign_data_wrapper AS f
  ORDER BY f.fdwname::text COLLATE pg_catalog."C", f.oid
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-V1/FDW/END@@

\echo @@ADR0047-HAS-V1/SERVER/BEGIN@@
COPY (
  WITH w AS (SELECT oid FROM pg_catalog.pg_authid
    WHERE rolname = 'sf_public_acl_no_membership_witness_v1')
  SELECT s.oid, pg_catalog.encode(pg_catalog.convert_to(s.srvname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to('USAGE', 'UTF8'), 'hex'),
    pg_catalog.has_server_privilege(w.oid, s.oid, 'USAGE'),
    pg_catalog.has_server_privilege(w.oid, s.oid, 'USAGE WITH GRANT OPTION')
  FROM w CROSS JOIN pg_catalog.pg_foreign_server AS s
  ORDER BY s.srvname::text COLLATE pg_catalog."C", s.oid
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-HAS-V1/SERVER/END@@

ROLLBACK;
