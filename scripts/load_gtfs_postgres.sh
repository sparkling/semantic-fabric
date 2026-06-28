#!/usr/bin/env bash
# load_gtfs_postgres.sh — (re)create the `gtfs_bench` PostgreSQL database and load
# the GTFS-Madrid OBDA subset at a given scale factor, byte-for-byte matching the
# deterministic generator semantic-fabric's sf-bench uses (workload.rs::generate).
# This is the shared dataset for the semantic-fabric vs Ontop head-to-head.
#
# Usage:   scripts/load_gtfs_postgres.sh [SCALE] [DB] [PGHOST] [PGPORT] [PGUSER]
# Example: scripts/load_gtfs_postgres.sh 10
set -euo pipefail

SCALE="${1:-1}"
DB="${2:-gtfs_bench}"
PGHOST="${3:-localhost}"
PGPORT="${4:-5432}"
PGUSER="${5:-$USER}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo ">> (re)creating database $DB on $PGHOST:$PGPORT (user $PGUSER)"
psql -h "$PGHOST" -p "$PGPORT" -U "$PGUSER" -d postgres -v ON_ERROR_STOP=1 \
  -c "DROP DATABASE IF EXISTS $DB;" -c "CREATE DATABASE $DB;"

echo ">> loading GTFS subset at scale=$SCALE"
psql -h "$PGHOST" -p "$PGPORT" -U "$PGUSER" -d "$DB" -v ON_ERROR_STOP=1 \
  -v scale="$SCALE" -f "$HERE/gen_gtfs.sql"
