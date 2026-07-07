#!/usr/bin/env bash
# dev_pg.sh — set up live PostgreSQL access for the sf test suites.
#
# The live-PG suites (sf-conformance: differential_pg_sqlite + w3c_pg_suite,
# sf-serve: endpoint) read SF_PG_URL (host/user params, NO dbname) and gracefully
# SKIP when no server is reachable. Their default connection is trust auth on
# `host=localhost port=5432 user=$USER`, with the scratch database named after
# $USER — each run does `DROP SCHEMA public CASCADE` in it, so it must be a
# throwaway DB (this script creates an empty one; never point it at real data).
#
# Usage:
#   scripts/dev_pg.sh                 # ensure local scratch DB, then `cargo test` just works
#   eval "$(scripts/dev_pg.sh --env)" # print the export line for the Docker container instead
set -euo pipefail

PGHOST="${PGHOST:-localhost}"
PGPORT="${PGPORT:-5432}"
PGUSER="${PGUSER:-$USER}"
DB="${PGUSER}"   # the no-dbname default resolves dbname → $USER

if [ "${1:-}" = "--env" ]; then
  # Docker-container alternative (scripts spin up postgres:16 as henrik/sftest:15432).
  echo "export SF_PG_URL='host=localhost port=15432 user=henrik password=sftest'"
  exit 0
fi

if ! pg_isready -h "$PGHOST" -p "$PGPORT" >/dev/null 2>&1; then
  echo "!! no PostgreSQL reachable at $PGHOST:$PGPORT — start one (e.g. a local install or the Docker container) first" >&2
  exit 1
fi

if psql -h "$PGHOST" -p "$PGPORT" -U "$PGUSER" -d "$DB" -tAc 'select 1' >/dev/null 2>&1; then
  echo ">> scratch test DB '$DB' already reachable on $PGHOST:$PGPORT (user $PGUSER)"
else
  echo ">> creating scratch test DB '$DB' on $PGHOST:$PGPORT"
  createdb -h "$PGHOST" -p "$PGPORT" -U "$PGUSER" "$DB"
fi

echo ">> live PG ready. The sf test suites now run instead of skipping:"
echo "     cargo test -p sf-conformance --test differential_pg_sqlite -- --test-threads=1"
echo "     cargo test -p sf-conformance --test w3c_pg_suite           -- --test-threads=1"
echo "   (default connection needs no SF_PG_URL; for the Docker container: eval \"\$(scripts/dev_pg.sh --env)\")"
