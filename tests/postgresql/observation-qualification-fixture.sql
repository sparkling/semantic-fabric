\set ON_ERROR_STOP on

CREATE ROLE sf_observation_owner_v1
  LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
CREATE ROLE sf_observation_observer_v1
  LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;

CREATE DATABASE sf_observation_qualification_v1
  WITH OWNER = sf_observation_owner_v1
       TEMPLATE = template0
       ENCODING = 'UTF8'
       LOCALE_PROVIDER = libc
       LC_COLLATE = 'C'
       LC_CTYPE = 'C';

REVOKE ALL ON DATABASE sf_observation_qualification_v1 FROM PUBLIC;
GRANT CONNECT ON DATABASE sf_observation_qualification_v1
  TO sf_observation_owner_v1, sf_observation_observer_v1;

\connect sf_observation_qualification_v1 postgres

ALTER SCHEMA public OWNER TO sf_observation_owner_v1;
REVOKE ALL ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO sf_observation_observer_v1;

REVOKE ALL ON FUNCTION pg_catalog.pg_database_collation_actual_version(oid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION pg_catalog.pg_database_collation_actual_version(oid)
  TO sf_observation_owner_v1, sf_observation_observer_v1;

SET ROLE sf_observation_owner_v1;

CREATE TABLE public.qualification_parent (
  parent_id bigint NOT NULL,
  parent_code character varying(32) NOT NULL,
  CONSTRAINT qualification_parent_pk PRIMARY KEY (parent_id),
  CONSTRAINT qualification_parent_code_unique UNIQUE (parent_code)
);

CREATE TABLE public.qualification_types (
  row_id uuid NOT NULL,
  parent_id bigint NOT NULL,
  value_bool boolean NOT NULL,
  value_int2 smallint NOT NULL,
  value_int4 integer NOT NULL,
  value_int8 bigint NOT NULL,
  value_numeric numeric(18, 4) NOT NULL,
  value_float4 real NOT NULL,
  value_float8 double precision NOT NULL,
  value_text text NOT NULL,
  value_varchar character varying(64) NOT NULL,
  value_bpchar character(8) NOT NULL,
  value_bytea bytea NOT NULL,
  value_date date NOT NULL,
  value_time time(3) without time zone NOT NULL,
  value_timetz time(3) with time zone NOT NULL,
  value_timestamp timestamp(3) without time zone NOT NULL,
  value_timestamptz timestamp(3) with time zone NOT NULL,
  value_json json NOT NULL,
  value_jsonb jsonb NOT NULL,
  CONSTRAINT qualification_types_pk PRIMARY KEY (row_id),
  CONSTRAINT qualification_types_parent_fk
    FOREIGN KEY (parent_id) REFERENCES public.qualification_parent (parent_id)
);

RESET ROLE;

REVOKE ALL ON ALL TABLES IN SCHEMA public FROM PUBLIC;
GRANT SELECT ON TABLE public.qualification_parent, public.qualification_types
  TO sf_observation_observer_v1;
