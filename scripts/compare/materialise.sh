#!/usr/bin/env bash
# materialise.sh — the MATERIALISER axis (a DIFFERENT category from query engines).
# Morph-KGC reads the SAME GTFS R2RML over the SAME PostgreSQL and writes the whole
# virtual graph to a file (it COPIES the data into RDF; it is NOT a query engine and
# answers no SPARQL). We therefore measure DUMP wall-clock + output triple-count and
# file size at 1x/10x — NEVER query latency. The right semantic-fabric counterpart
# is its streaming CONSTRUCT dump (also a full-graph export), not the SELECT race.
#
# Prereqs: a Python venv with morph-kgc + psycopg2-binary; PostgreSQL at :5432.
# Usage: MORPH_PY=/path/to/venv/bin/python scripts/compare/materialise.sh [SCALE] [PGUSER] [DB]
set -euo pipefail

SCALE="${1:-1}"
PGUSER="${2:-henrik}"
DB="${3:-gtfs_bench}"
MORPH_PY="${MORPH_PY:?set MORPH_PY to a python with morph-kgc installed}"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
MAP="$REPO/scripts/ontop/gtfs.r2rml.ttl"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
OUT="$WORK/gtfs-${SCALE}x.nt"
CFG="$WORK/morph.ini"

cat >"$CFG" <<EOF
[CONFIGURATION]
output_file: $OUT
output_format: N-TRIPLES

[DataSource1]
mappings: $MAP
db_url: postgresql+psycopg2://${PGUSER}@localhost:5432/${DB}
EOF

echo ">> Morph-KGC materialise  scale=${SCALE}  (R2RML over PostgreSQL -> N-Triples file)"
start="$(python3 -c 'import time;print(time.time())')"
"$MORPH_PY" -m morph_kgc "$CFG" >/tmp/morph-${SCALE}x.log 2>&1
end="$(python3 -c 'import time;print(time.time())')"
wall="$(awk "BEGIN{printf \"%.2f\", $end-$start}")"

triples="$(wc -l < "$OUT" | tr -d ' ')"
bytes="$(stat -f%z "$OUT")"
mib="$(awk "BEGIN{printf \"%.2f\", $bytes/1048576}")"
printf "   wall_clock=%ss  triples=%s  output=%s bytes (%s MiB) -> %s\n" \
  "$wall" "$triples" "$bytes" "$mib" "$OUT"
