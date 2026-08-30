-- SPDX-License-Identifier: MIT
-- ADR-0047 independent raw-catalogue transcript. Every catalogue row is kept;
-- null/empty ACL state, physical ACL-item count, exploded-atom count/ordinality and
-- routine-argument structure remain separate evidence for the client oracle.

BEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY DEFERRABLE;
SET LOCAL search_path TO pg_catalog;
SET LOCAL row_security TO on;
SET LOCAL quote_all_identifiers TO off;
SET LOCAL client_encoding TO 'UTF8';

\echo @@ADR0047-RAW-V1/SCHEMA/BEGIN@@
COPY (
  SELECT n.oid,
    pg_catalog.encode(pg_catalog.convert_to(n.nspname::text, 'UTF8'), 'hex'),
    n.nspowner, n.nspacl IS NULL, pg_catalog.cardinality(n.nspacl),
    CASE WHEN acl.ordinality IS NULL THEN 0 ELSE acl.atom_count END,
    acl.ordinality, acl.grantor, acl.grantee,
    pg_catalog.encode(
      pg_catalog.convert_to(acl.privilege_type::text, 'UTF8'), 'hex'
    ),
    acl.is_grantable
  FROM pg_catalog.pg_namespace AS n
  LEFT JOIN LATERAL (
    SELECT expanded.*, count(*) OVER () AS atom_count
    FROM pg_catalog.aclexplode(
      CASE WHEN pg_catalog.cardinality(n.nspacl) = 0
        THEN NULL::aclitem[] ELSE n.nspacl END
    ) WITH ORDINALITY
      AS expanded(grantor, grantee, privilege_type, is_grantable, ordinality)
  ) AS acl ON true
  ORDER BY n.nspname::text COLLATE pg_catalog."C" ASC, n.oid ASC,
    acl.ordinality ASC NULLS FIRST
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-RAW-V1/SCHEMA/END@@

\echo @@ADR0047-RAW-V1/RELATION/BEGIN@@
COPY (
  SELECT c.oid, c.relnamespace,
    pg_catalog.encode(pg_catalog.convert_to(c.relname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(c.relkind::text, 'UTF8'), 'hex'),
    c.relowner, c.relacl IS NULL, pg_catalog.cardinality(c.relacl),
    CASE WHEN acl.ordinality IS NULL THEN 0 ELSE acl.atom_count END,
    acl.ordinality, acl.grantor, acl.grantee,
    pg_catalog.encode(
      pg_catalog.convert_to(acl.privilege_type::text, 'UTF8'), 'hex'
    ),
    acl.is_grantable
  FROM pg_catalog.pg_class AS c
  LEFT JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
  LEFT JOIN LATERAL (
    SELECT expanded.*, count(*) OVER () AS atom_count
    FROM pg_catalog.aclexplode(
      CASE WHEN pg_catalog.cardinality(c.relacl) = 0
        THEN NULL::aclitem[] ELSE c.relacl END
    ) WITH ORDINALITY
      AS expanded(grantor, grantee, privilege_type, is_grantable, ordinality)
  ) AS acl ON true
  ORDER BY n.nspname::text COLLATE pg_catalog."C" ASC NULLS FIRST,
    c.relname::text COLLATE pg_catalog."C" ASC, c.oid ASC,
    acl.ordinality ASC NULLS FIRST
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-RAW-V1/RELATION/END@@

\echo @@ADR0047-RAW-V1/COLUMN/BEGIN@@
COPY (
  SELECT a.attrelid, a.attnum,
    pg_catalog.encode(pg_catalog.convert_to(a.attname::text, 'UTF8'), 'hex'),
    a.attisdropped, a.attacl IS NULL, pg_catalog.cardinality(a.attacl),
    CASE WHEN acl.ordinality IS NULL THEN 0 ELSE acl.atom_count END,
    acl.ordinality, acl.grantor, acl.grantee,
    pg_catalog.encode(
      pg_catalog.convert_to(acl.privilege_type::text, 'UTF8'), 'hex'
    ),
    acl.is_grantable
  FROM pg_catalog.pg_attribute AS a
  LEFT JOIN pg_catalog.pg_class AS c ON c.oid = a.attrelid
  LEFT JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
  LEFT JOIN LATERAL (
    SELECT expanded.*, count(*) OVER () AS atom_count
    FROM pg_catalog.aclexplode(
      CASE WHEN pg_catalog.cardinality(a.attacl) = 0
        THEN NULL::aclitem[] ELSE a.attacl END
    ) WITH ORDINALITY
      AS expanded(grantor, grantee, privilege_type, is_grantable, ordinality)
  ) AS acl ON true
  ORDER BY n.nspname::text COLLATE pg_catalog."C" ASC NULLS FIRST,
    c.relname::text COLLATE pg_catalog."C" ASC NULLS FIRST,
    a.attnum ASC, a.attname::text COLLATE pg_catalog."C" ASC,
    a.attrelid ASC, acl.ordinality ASC NULLS FIRST
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-RAW-V1/COLUMN/END@@

\echo @@ADR0047-RAW-V1/ROUTINE/BEGIN@@
COPY (
  SELECT p.oid, p.pronamespace,
    pg_catalog.encode(pg_catalog.convert_to(p.proname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(p.prokind::text, 'UTF8'), 'hex'),
    p.proowner,
    pg_catalog.encode(pg_catalog.convert_to(a.aggkind::text, 'UTF8'), 'hex'),
    a.aggnumdirectargs,
    p.proallargtypes IS NULL, p.proargmodes IS NULL, p.proargnames IS NULL,
    pg_catalog.cardinality(
      CASE WHEN p.proallargtypes IS NULL
        THEN p.proargtypes::oid[] ELSE p.proallargtypes END
    ),
    argument.ordinality,
    pg_catalog.encode(pg_catalog.convert_to(
      p.proargmodes[argument.ordinality::integer]::text, 'UTF8'
    ), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(
      p.proargnames[argument.ordinality::integer]::text, 'UTF8'
    ), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(pg_catalog.quote_ident(
      p.proargnames[argument.ordinality::integer]
    ), 'UTF8'), 'hex'),
    argument.type_oid,
    pg_catalog.encode(pg_catalog.convert_to(
      pg_catalog.format_type(argument.type_oid, NULL), 'UTF8'
    ), 'hex'),
    p.proacl IS NULL, pg_catalog.cardinality(p.proacl),
    CASE WHEN acl.ordinality IS NULL THEN 0 ELSE acl.atom_count END,
    acl.ordinality, acl.grantor, acl.grantee,
    pg_catalog.encode(
      pg_catalog.convert_to(acl.privilege_type::text, 'UTF8'), 'hex'
    ),
    acl.is_grantable
  FROM pg_catalog.pg_proc AS p
  LEFT JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
  LEFT JOIN pg_catalog.pg_aggregate AS a ON a.aggfnoid = p.oid
  LEFT JOIN LATERAL pg_catalog.unnest(
    CASE WHEN p.proallargtypes IS NULL
      THEN p.proargtypes::oid[] ELSE p.proallargtypes END
  ) WITH ORDINALITY AS argument(type_oid, ordinality) ON true
  LEFT JOIN LATERAL (
    SELECT expanded.*, count(*) OVER () AS atom_count
    FROM pg_catalog.aclexplode(
      CASE WHEN pg_catalog.cardinality(p.proacl) = 0
        THEN NULL::aclitem[] ELSE p.proacl END
    ) WITH ORDINALITY
      AS expanded(grantor, grantee, privilege_type, is_grantable, ordinality)
  ) AS acl ON true
  ORDER BY n.nspname::text COLLATE pg_catalog."C" ASC NULLS FIRST,
    p.proname::text COLLATE pg_catalog."C" ASC, p.oid ASC,
    argument.ordinality ASC NULLS FIRST, acl.ordinality ASC NULLS FIRST
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-RAW-V1/ROUTINE/END@@

\echo @@ADR0047-RAW-V1/TYPE/BEGIN@@
COPY (
  SELECT t.oid, t.typnamespace,
    pg_catalog.encode(pg_catalog.convert_to(t.typname::text, 'UTF8'), 'hex'),
    pg_catalog.encode(pg_catalog.convert_to(t.typtype::text, 'UTF8'), 'hex'),
    t.typowner, t.typelem, t.typsubscript::oid,
    t.typacl IS NULL, pg_catalog.cardinality(t.typacl),
    CASE WHEN acl.ordinality IS NULL THEN 0 ELSE acl.atom_count END,
    acl.ordinality, acl.grantor, acl.grantee,
    pg_catalog.encode(
      pg_catalog.convert_to(acl.privilege_type::text, 'UTF8'), 'hex'
    ),
    acl.is_grantable
  FROM pg_catalog.pg_type AS t
  LEFT JOIN pg_catalog.pg_namespace AS n ON n.oid = t.typnamespace
  LEFT JOIN LATERAL (
    SELECT expanded.*, count(*) OVER () AS atom_count
    FROM pg_catalog.aclexplode(
      CASE WHEN pg_catalog.cardinality(t.typacl) = 0
        THEN NULL::aclitem[] ELSE t.typacl END
    ) WITH ORDINALITY
      AS expanded(grantor, grantee, privilege_type, is_grantable, ordinality)
  ) AS acl ON true
  ORDER BY n.nspname::text COLLATE pg_catalog."C" ASC NULLS FIRST,
    t.typname::text COLLATE pg_catalog."C" ASC, t.oid ASC,
    acl.ordinality ASC NULLS FIRST
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-RAW-V1/TYPE/END@@

\echo @@ADR0047-RAW-V1/LANGUAGE/BEGIN@@
COPY (
  SELECT l.oid,
    pg_catalog.encode(pg_catalog.convert_to(l.lanname::text, 'UTF8'), 'hex'),
    l.lanowner, l.lanacl IS NULL, pg_catalog.cardinality(l.lanacl),
    CASE WHEN acl.ordinality IS NULL THEN 0 ELSE acl.atom_count END,
    acl.ordinality, acl.grantor, acl.grantee,
    pg_catalog.encode(
      pg_catalog.convert_to(acl.privilege_type::text, 'UTF8'), 'hex'
    ),
    acl.is_grantable
  FROM pg_catalog.pg_language AS l
  LEFT JOIN LATERAL (
    SELECT expanded.*, count(*) OVER () AS atom_count
    FROM pg_catalog.aclexplode(
      CASE WHEN pg_catalog.cardinality(l.lanacl) = 0
        THEN NULL::aclitem[] ELSE l.lanacl END
    ) WITH ORDINALITY
      AS expanded(grantor, grantee, privilege_type, is_grantable, ordinality)
  ) AS acl ON true
  ORDER BY l.lanname::text COLLATE pg_catalog."C" ASC, l.oid ASC,
    acl.ordinality ASC NULLS FIRST
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-RAW-V1/LANGUAGE/END@@

\echo @@ADR0047-RAW-V1/FDW/BEGIN@@
COPY (
  SELECT f.oid,
    pg_catalog.encode(pg_catalog.convert_to(f.fdwname::text, 'UTF8'), 'hex'),
    f.fdwowner, f.fdwacl IS NULL, pg_catalog.cardinality(f.fdwacl),
    CASE WHEN acl.ordinality IS NULL THEN 0 ELSE acl.atom_count END,
    acl.ordinality, acl.grantor, acl.grantee,
    pg_catalog.encode(
      pg_catalog.convert_to(acl.privilege_type::text, 'UTF8'), 'hex'
    ),
    acl.is_grantable
  FROM pg_catalog.pg_foreign_data_wrapper AS f
  LEFT JOIN LATERAL (
    SELECT expanded.*, count(*) OVER () AS atom_count
    FROM pg_catalog.aclexplode(
      CASE WHEN pg_catalog.cardinality(f.fdwacl) = 0
        THEN NULL::aclitem[] ELSE f.fdwacl END
    ) WITH ORDINALITY
      AS expanded(grantor, grantee, privilege_type, is_grantable, ordinality)
  ) AS acl ON true
  ORDER BY f.fdwname::text COLLATE pg_catalog."C" ASC, f.oid ASC,
    acl.ordinality ASC NULLS FIRST
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-RAW-V1/FDW/END@@

\echo @@ADR0047-RAW-V1/SERVER/BEGIN@@
COPY (
  SELECT s.oid,
    pg_catalog.encode(pg_catalog.convert_to(s.srvname::text, 'UTF8'), 'hex'),
    s.srvowner, s.srvfdw,
    s.srvacl IS NULL, pg_catalog.cardinality(s.srvacl),
    CASE WHEN acl.ordinality IS NULL THEN 0 ELSE acl.atom_count END,
    acl.ordinality, acl.grantor, acl.grantee,
    pg_catalog.encode(
      pg_catalog.convert_to(acl.privilege_type::text, 'UTF8'), 'hex'
    ),
    acl.is_grantable
  FROM pg_catalog.pg_foreign_server AS s
  LEFT JOIN LATERAL (
    SELECT expanded.*, count(*) OVER () AS atom_count
    FROM pg_catalog.aclexplode(
      CASE WHEN pg_catalog.cardinality(s.srvacl) = 0
        THEN NULL::aclitem[] ELSE s.srvacl END
    ) WITH ORDINALITY
      AS expanded(grantor, grantee, privilege_type, is_grantable, ordinality)
  ) AS acl ON true
  ORDER BY s.srvname::text COLLATE pg_catalog."C" ASC, s.oid ASC,
    acl.ordinality ASC NULLS FIRST
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-RAW-V1/SERVER/END@@

\echo @@ADR0047-RAW-V1/LARGE_OBJECT/BEGIN@@
COPY (
  SELECT l.oid, NULL::text, l.lomowner,
    l.lomacl IS NULL, pg_catalog.cardinality(l.lomacl),
    CASE WHEN acl.ordinality IS NULL THEN 0 ELSE acl.atom_count END,
    acl.ordinality, acl.grantor, acl.grantee,
    pg_catalog.encode(
      pg_catalog.convert_to(acl.privilege_type::text, 'UTF8'), 'hex'
    ),
    acl.is_grantable
  FROM pg_catalog.pg_largeobject_metadata AS l
  LEFT JOIN LATERAL (
    SELECT expanded.*, count(*) OVER () AS atom_count
    FROM pg_catalog.aclexplode(
      CASE WHEN pg_catalog.cardinality(l.lomacl) = 0
        THEN NULL::aclitem[] ELSE l.lomacl END
    ) WITH ORDINALITY
      AS expanded(grantor, grantee, privilege_type, is_grantable, ordinality)
  ) AS acl ON true
  ORDER BY l.oid ASC, acl.ordinality ASC NULLS FIRST
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-RAW-V1/LARGE_OBJECT/END@@

\echo @@ADR0047-RAW-V1/CONTROL/BEGIN@@
COPY (
  SELECT
    (SELECT count(*) FROM pg_catalog.pg_namespace),
    (SELECT count(*) FROM pg_catalog.pg_namespace AS n
      CROSS JOIN LATERAL pg_catalog.unnest(n.nspacl) AS item(value)),
    (SELECT count(*) FROM pg_catalog.pg_namespace AS n
      CROSS JOIN LATERAL pg_catalog.aclexplode(
        CASE WHEN pg_catalog.cardinality(n.nspacl) = 0
          THEN NULL::aclitem[] ELSE n.nspacl END
      ) AS atom),
    (SELECT count(*) FROM pg_catalog.pg_class),
    (SELECT count(*) FROM pg_catalog.pg_class AS c
      CROSS JOIN LATERAL pg_catalog.unnest(c.relacl) AS item(value)),
    (SELECT count(*) FROM pg_catalog.pg_class AS c
      CROSS JOIN LATERAL pg_catalog.aclexplode(
        CASE WHEN pg_catalog.cardinality(c.relacl) = 0
          THEN NULL::aclitem[] ELSE c.relacl END
      ) AS atom),
    (SELECT count(*) FROM pg_catalog.pg_attribute),
    (SELECT count(*) FROM pg_catalog.pg_attribute AS a
      CROSS JOIN LATERAL pg_catalog.unnest(a.attacl) AS item(value)),
    (SELECT count(*) FROM pg_catalog.pg_attribute AS a
      CROSS JOIN LATERAL pg_catalog.aclexplode(
        CASE WHEN pg_catalog.cardinality(a.attacl) = 0
          THEN NULL::aclitem[] ELSE a.attacl END
      ) AS atom),
    (SELECT count(*) FROM pg_catalog.pg_proc),
    (SELECT count(*) FROM pg_catalog.pg_proc AS p
      CROSS JOIN LATERAL pg_catalog.unnest(p.proacl) AS item(value)),
    (SELECT count(*) FROM pg_catalog.pg_proc AS p
      CROSS JOIN LATERAL pg_catalog.aclexplode(
        CASE WHEN pg_catalog.cardinality(p.proacl) = 0
          THEN NULL::aclitem[] ELSE p.proacl END
      ) AS atom),
    (SELECT count(*) FROM pg_catalog.pg_proc AS p
      CROSS JOIN LATERAL pg_catalog.unnest(
        CASE WHEN p.proallargtypes IS NULL
          THEN p.proargtypes::oid[] ELSE p.proallargtypes END
      ) AS argument(type_oid)),
    (SELECT count(*) FROM pg_catalog.pg_type),
    (SELECT count(*) FROM pg_catalog.pg_type AS t
      CROSS JOIN LATERAL pg_catalog.unnest(t.typacl) AS item(value)),
    (SELECT count(*) FROM pg_catalog.pg_type AS t
      CROSS JOIN LATERAL pg_catalog.aclexplode(
        CASE WHEN pg_catalog.cardinality(t.typacl) = 0
          THEN NULL::aclitem[] ELSE t.typacl END
      ) AS atom),
    (SELECT count(*) FROM pg_catalog.pg_language),
    (SELECT count(*) FROM pg_catalog.pg_language AS l
      CROSS JOIN LATERAL pg_catalog.unnest(l.lanacl) AS item(value)),
    (SELECT count(*) FROM pg_catalog.pg_language AS l
      CROSS JOIN LATERAL pg_catalog.aclexplode(
        CASE WHEN pg_catalog.cardinality(l.lanacl) = 0
          THEN NULL::aclitem[] ELSE l.lanacl END
      ) AS atom),
    (SELECT count(*) FROM pg_catalog.pg_foreign_data_wrapper),
    (SELECT count(*) FROM pg_catalog.pg_foreign_data_wrapper AS f
      CROSS JOIN LATERAL pg_catalog.unnest(f.fdwacl) AS item(value)),
    (SELECT count(*) FROM pg_catalog.pg_foreign_data_wrapper AS f
      CROSS JOIN LATERAL pg_catalog.aclexplode(
        CASE WHEN pg_catalog.cardinality(f.fdwacl) = 0
          THEN NULL::aclitem[] ELSE f.fdwacl END
      ) AS atom),
    (SELECT count(*) FROM pg_catalog.pg_foreign_server),
    (SELECT count(*) FROM pg_catalog.pg_foreign_server AS s
      CROSS JOIN LATERAL pg_catalog.unnest(s.srvacl) AS item(value)),
    (SELECT count(*) FROM pg_catalog.pg_foreign_server AS s
      CROSS JOIN LATERAL pg_catalog.aclexplode(
        CASE WHEN pg_catalog.cardinality(s.srvacl) = 0
          THEN NULL::aclitem[] ELSE s.srvacl END
      ) AS atom),
    (SELECT count(*) FROM pg_catalog.pg_largeobject_metadata),
    (SELECT count(*) FROM pg_catalog.pg_largeobject_metadata AS l
      CROSS JOIN LATERAL pg_catalog.unnest(l.lomacl) AS item(value)),
    (SELECT count(*) FROM pg_catalog.pg_largeobject_metadata AS l
      CROSS JOIN LATERAL pg_catalog.aclexplode(
        CASE WHEN pg_catalog.cardinality(l.lomacl) = 0
          THEN NULL::aclitem[] ELSE l.lomacl END
      ) AS atom),
    (SELECT count(*) FROM pg_catalog.pg_default_acl),
    (SELECT count(*) FROM pg_catalog.pg_parameter_acl),
    (SELECT count(*) FROM pg_catalog.pg_user_mapping),
    (SELECT count(*)
      FROM pg_catalog.pg_depend AS d
      JOIN pg_catalog.pg_namespace AS n ON n.oid = d.refobjid
      WHERE d.refclassid = 'pg_catalog.pg_namespace'::regclass
        AND n.nspname = 'public')
) TO STDOUT WITH (FORMAT text, DELIMITER E'\t', NULL E'\\N');
\echo @@ADR0047-RAW-V1/CONTROL/END@@

ROLLBACK;
